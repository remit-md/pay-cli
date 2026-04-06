use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use base64::Engine as _;
use clap::Args;
use reqwest::Method;

use crate::error;

#[derive(Args)]
pub struct RequestArgs {
    /// URL to request (x402 payment handled automatically)
    pub url: String,

    /// HTTP method (GET, POST, PUT, DELETE, PATCH, HEAD)
    #[arg(short = 'X', long = "request")]
    pub method: Option<String>,

    /// Add header (repeatable): "Key: Value"
    #[arg(short = 'H', long = "header")]
    pub headers: Vec<String>,

    /// Request body (prefix with @ to read from file)
    #[arg(short = 'd', long = "data")]
    pub data: Option<String>,

    /// Write response body to file
    #[arg(short = 'o', long = "output")]
    pub output: Option<PathBuf>,

    /// Show request/response headers
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,

    /// Suppress status messages, output body only
    #[arg(short = 's', long = "silent")]
    pub silent: bool,

    /// Follow redirects (enabled by default, accepted for curl compatibility)
    #[arg(short = 'L', long = "location", hide = true)]
    pub location: bool,

    /// Disable following redirects
    #[arg(long = "no-location")]
    pub no_location: bool,

    /// Connection timeout in seconds
    #[arg(long = "connect-timeout", default_value = "10")]
    pub connect_timeout: u64,

    /// Maximum total request time in seconds
    #[arg(long = "max-time", default_value = "30")]
    pub max_time: u64,

    /// Skip x402 payment handling
    #[arg(long = "no-pay")]
    pub no_pay: bool,
}

pub async fn run(args: RequestArgs, mut ctx: super::Context) -> Result<()> {
    super::require_init()?;

    // Resolve inputs once — reused for both initial request and x402 retry
    let method = resolve_method(&args)?;
    let body = resolve_body(&args.data)?;
    let headers = parse_headers(&args.headers)?;
    let auto_ct = body.is_some() && !has_content_type(&headers);
    let client = build_client(&args)?;

    if args.verbose {
        print_verbose_request(&method, &args.url, &headers, &body, auto_ct);
    }

    // Initial request
    let req = build_request(
        &client,
        &method,
        &args.url,
        &headers,
        &body,
        auto_ct,
        args.max_time,
        &[],
    );
    let resp = req.send().await?;

    // Not 402 or --no-pay: output and return
    if resp.status().as_u16() != 402 || args.no_pay {
        return output_response(resp, &args, &ctx).await;
    }

    // ── x402 payment flow ──────────────────────────────────────────────

    let reqs = parse_402_requirements(resp).await?;
    let settlement = reqs.settlement.as_str();
    let amount = reqs.amount;
    let to = &reqs.pay_to;

    if !args.silent && !ctx.json {
        error::success(&format!(
            "402 Payment Required: {} ({settlement})",
            super::format_amount(amount),
        ));
    }

    // Build v2 PAYMENT-SIGNATURE via tab or direct settlement
    let encoded_payload: String;

    if settlement == "tab" {
        let charge_body = serde_json::json!({ "amount": amount });
        let tabs: Vec<serde_json::Value> = ctx
            .get("/tabs")
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default();
        let tab = tabs.iter().find(|t| {
            t["provider"].as_str() == Some(to.as_str()) && t["status"].as_str() == Some("open")
        });

        let tab_id = if let Some(t) = tab {
            t["id"].as_str().unwrap_or("").to_string()
        } else {
            let tab_amount = std::cmp::max(amount * 10, 5_000_000);
            let contracts = crate::permit::get_contracts(&mut ctx).await?;
            let permit =
                crate::permit::prepare_and_sign(&mut ctx, tab_amount, &contracts.tab).await?;
            let open_body = serde_json::json!({
                "provider": to,
                "amount": tab_amount,
                "max_charge_per_call": amount,
                "permit": permit.to_json(),
            });
            let open_resp = ctx.post("/tabs", &open_body).await?;
            open_resp["tab_id"].as_str().unwrap_or("").to_string()
        };

        let charge_resp = ctx
            .post(&format!("/tabs/{tab_id}/charge"), &charge_body)
            .await?;
        let charge_id = charge_resp["charge_id"].as_str().unwrap_or("").to_string();

        let payment_payload = serde_json::json!({
            "x402Version": 2,
            "accepted": reqs.accepted,
            "payload": {},
            "extensions": {
                "pay": {
                    "settlement": "tab",
                    "tabId": tab_id,
                    "chargeId": charge_id,
                }
            }
        });
        encoded_payload = base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_string(&payment_payload)?);
    } else {
        // Direct settlement — sign EIP-3009 TransferWithAuthorization
        let contracts = crate::permit::get_contracts(&mut ctx).await?;
        let chain_id = ctx.config.chain_id();
        let usdc_addr = contracts.usdc.clone();
        let key = ctx.load_key()?;
        let auth =
            crate::eip3009::sign_transfer_authorization(key, to, amount, chain_id, &usdc_addr)?;

        let payment_payload = serde_json::json!({
            "x402Version": 2,
            "accepted": reqs.accepted,
            "payload": {
                "signature": auth.combined_signature(),
                "authorization": {
                    "from": auth.from,
                    "to": auth.to,
                    "value": auth.amount.to_string(),
                    "validAfter": auth.valid_after,
                    "validBefore": auth.valid_before,
                    "nonce": auth.nonce,
                }
            },
            "extensions": {}
        });
        encoded_payload = base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_string(&payment_payload)?);
    }

    // Retry with same method/headers/body + PAYMENT-SIGNATURE
    let extra = [("PAYMENT-SIGNATURE".to_string(), encoded_payload)];

    if args.verbose {
        eprintln!("> [retry with PAYMENT-SIGNATURE]");
        print_verbose_request(&method, &args.url, &headers, &body, auto_ct);
    }

    let retry_req = build_request(
        &client,
        &method,
        &args.url,
        &headers,
        &body,
        auto_ct,
        args.max_time,
        &extra,
    );
    let retry_resp = retry_req.send().await?;

    output_response(retry_resp, &args, &ctx).await
}

// ── Helpers ────────────────────────────────────────────────────────────

/// Resolve HTTP method: explicit -X wins, then -d implies POST, else GET.
fn resolve_method(args: &RequestArgs) -> Result<Method> {
    if let Some(ref m) = args.method {
        Method::from_bytes(m.to_uppercase().as_bytes())
            .map_err(|_| anyhow::anyhow!("invalid HTTP method: {m}"))
    } else if args.data.is_some() {
        Ok(Method::POST)
    } else {
        Ok(Method::GET)
    }
}

/// Read body from argument or @file.
fn resolve_body(data: &Option<String>) -> Result<Option<String>> {
    match data {
        Some(d) if d.starts_with('@') => {
            let path = &d[1..];
            let content = std::fs::read_to_string(path)
                .map_err(|e| anyhow::anyhow!("failed to read {path}: {e}"))?;
            Ok(Some(content))
        }
        Some(d) => Ok(Some(d.clone())),
        None => Ok(None),
    }
}

/// Parse "Key: Value" header strings, splitting on the first colon.
fn parse_headers(raw: &[String]) -> Result<Vec<(String, String)>> {
    raw.iter()
        .map(|h| {
            h.split_once(':')
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                .ok_or_else(|| anyhow::anyhow!("invalid header (expected 'Key: Value'): {h}"))
        })
        .collect()
}

/// Case-insensitive check for Content-Type in parsed headers.
fn has_content_type(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
}

/// Build a reqwest client with redirect and connect-timeout settings.
fn build_client(args: &RequestArgs) -> Result<reqwest::Client> {
    let mut builder =
        reqwest::Client::builder().connect_timeout(Duration::from_secs(args.connect_timeout));
    if args.no_location {
        builder = builder.redirect(reqwest::redirect::Policy::none());
    }
    builder
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build HTTP client: {e}"))
}

/// Assemble a RequestBuilder from resolved components.
fn build_request(
    client: &reqwest::Client,
    method: &Method,
    url: &str,
    headers: &[(String, String)],
    body: &Option<String>,
    auto_content_type: bool,
    max_time: u64,
    extra_headers: &[(String, String)],
) -> reqwest::RequestBuilder {
    let mut builder = client
        .request(method.clone(), url)
        .timeout(Duration::from_secs(max_time));

    for (k, v) in headers {
        builder = builder.header(k.as_str(), v.as_str());
    }

    if let Some(data) = body {
        if auto_content_type {
            builder = builder.header("Content-Type", "application/json");
        }
        builder = builder.body(data.clone());
    }

    for (k, v) in extra_headers {
        builder = builder.header(k.as_str(), v.as_str());
    }

    builder
}

// ── Output ─────────────────────────────────────────────────────────────

fn print_verbose_request(
    method: &Method,
    url: &str,
    headers: &[(String, String)],
    body: &Option<String>,
    auto_ct: bool,
) {
    eprintln!("> {method} {url}");
    if auto_ct {
        eprintln!("> Content-Type: application/json");
    }
    for (k, v) in headers {
        eprintln!("> {k}: {v}");
    }
    if body.is_some() {
        eprintln!("> [body]");
    }
    eprintln!(">");
}

fn print_verbose_response(resp: &reqwest::Response) {
    eprintln!(
        "< {} {}",
        resp.status().as_u16(),
        resp.status().canonical_reason().unwrap_or("")
    );
    for (k, v) in resp.headers() {
        if let Ok(val) = v.to_str() {
            eprintln!("< {k}: {val}");
        }
    }
    eprintln!("<");
}

async fn output_response(
    resp: reqwest::Response,
    args: &RequestArgs,
    ctx: &super::Context,
) -> Result<()> {
    let status = resp.status();

    if args.verbose {
        print_verbose_response(&resp);
    }

    // Write to file
    if let Some(ref path) = args.output {
        let bytes = resp.bytes().await?;
        std::fs::write(path, &bytes)
            .map_err(|e| anyhow::anyhow!("failed to write to {}: {e}", path.display()))?;
        if !args.silent {
            eprintln!("Wrote {} bytes to {}", bytes.len(), path.display());
        }
        return Ok(());
    }

    // JSON mode (global --json flag)
    if ctx.json {
        let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!(null));
        error::print_json(&body);
        return Ok(());
    }

    // Silent: body only, no status prefix
    if args.silent {
        let body = resp.text().await.unwrap_or_default();
        print!("{body}");
        return Ok(());
    }

    // Default: status + body
    let body = resp.text().await.unwrap_or_default();
    error::success(&format!("[{status}] {body}"));
    Ok(())
}

// ── x402 v2 parsing ───────────────────────────────────────────────────

/// Parsed v2 payment requirements extracted from a 402 response.
struct ParsedRequirements {
    settlement: String,
    amount: u64,
    pay_to: String,
    /// The full accepts[0] object, echoed back in PAYMENT-SIGNATURE.accepted.
    accepted: serde_json::Value,
}

/// Try to extract v2 requirements from a decoded JSON value.
/// Returns None if not a valid v2 structure.
fn try_extract_v2(decoded: &serde_json::Value) -> Option<ParsedRequirements> {
    if decoded.get("x402Version")?.as_u64()? != 2 {
        return None;
    }
    let accepts = decoded.get("accepts")?.as_array()?;
    let accepted = accepts.first()?;
    let extra = accepted.get("extra")?;
    Some(ParsedRequirements {
        settlement: extra.get("settlement")?.as_str()?.to_string(),
        amount: accepted
            .get("amount")?
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())?,
        pay_to: accepted.get("payTo")?.as_str()?.to_string(),
        accepted: accepted.clone(),
    })
}

/// Parse x402 V2 payment requirements from a 402 response.
///
/// Checks PAYMENT-REQUIRED header first (base64-encoded JSON),
/// falls back to response body. Requires x402Version 2.
async fn parse_402_requirements(resp: reqwest::Response) -> Result<ParsedRequirements> {
    if let Some(pr_header) = resp.headers().get("payment-required") {
        if let Some(decoded) = pr_header
            .to_str()
            .ok()
            .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        {
            if let Some(reqs) = try_extract_v2(&decoded) {
                return Ok(reqs);
            }
        }
    }

    // Fallback: try response body as v2
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("invalid 402 response body: {e}"))?;

    if let Some(reqs) = try_extract_v2(&body) {
        return Ok(reqs);
    }

    anyhow::bail!("Unrecognized 402 format — expected x402 v2 PaymentRequired")
}
