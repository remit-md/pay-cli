use anyhow::Result;
use clap::Args;

use crate::error;

#[derive(Args)]
pub struct RequestArgs {
    /// URL to request (x402 payment handled automatically)
    pub url: String,
}

pub async fn run(args: RequestArgs, mut ctx: super::Context) -> Result<()> {
    super::require_init()?;

    // Make the initial request
    let resp = ctx.http.get(&args.url).send().await?;

    if resp.status().as_u16() != 402 {
        // No payment needed
        if ctx.json {
            let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!(null));
            error::print_json(&body);
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            error::success(&format!("[{status}] {body}"));
        }
        return Ok(());
    }

    // 402 — parse payment requirements (x402 V2)
    // V2: check PAYMENT-REQUIRED header first (base64-encoded JSON)
    let requirements = parse_402_requirements(resp).await?;
    let settlement = requirements["settlement"].as_str().unwrap_or("direct");
    let amount = requirements["amount"].as_u64().unwrap_or(0);
    let to = requirements["to"].as_str().unwrap_or("");

    if !ctx.json {
        error::success(&format!(
            "402 Payment Required: {} ({settlement})",
            super::format_amount(amount),
        ));
    }

    // Pay via the appropriate settlement mode and build PAYMENT-SIGNATURE
    let payment_signature: serde_json::Value;

    if settlement == "tab" {
        // Tab settlement: find or open tab, charge it
        let charge_body = serde_json::json!({ "amount": amount });
        let tabs: Vec<serde_json::Value> = ctx
            .get("/tabs")
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default();
        let tab = tabs
            .iter()
            .find(|t| t["provider"].as_str() == Some(to) && t["status"].as_str() == Some("open"));

        let tab_id = if let Some(t) = tab {
            t["id"].as_str().unwrap_or("").to_string()
        } else {
            // Auto-open a tab: 10x the per-call price, min $5
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

        payment_signature = serde_json::json!({
            "settlement": "tab",
            "tab_id": tab_id,
            "charge_id": charge_id,
        });
    } else {
        // Direct settlement — sign EIP-3009 TransferWithAuthorization
        let contracts = crate::permit::get_contracts(&mut ctx).await?;
        let chain_id = ctx.config.chain_id();
        let usdc_addr = contracts.usdc.clone();
        let key = ctx.load_key()?;
        let auth =
            crate::eip3009::sign_transfer_authorization(key, to, amount, chain_id, &usdc_addr)?;

        payment_signature = auth.to_json();
    }

    // Retry the original request with V2 PAYMENT-SIGNATURE header
    let retry_resp = ctx
        .http
        .get(&args.url)
        .header("PAYMENT-SIGNATURE", payment_signature.to_string())
        .send()
        .await?;

    if ctx.json {
        let body: serde_json::Value = retry_resp.json().await.unwrap_or(serde_json::json!(null));
        error::print_json(&body);
    } else {
        let status = retry_resp.status();
        let body = retry_resp.text().await.unwrap_or_default();
        error::success(&format!("[{status}] {body}"));
    }
    Ok(())
}

/// Parse x402 V2 payment requirements from a 402 response.
///
/// Checks PAYMENT-REQUIRED header first (base64-encoded JSON),
/// falls back to response body.
async fn parse_402_requirements(resp: reqwest::Response) -> Result<serde_json::Value> {
    use base64::Engine;

    // V2: check PAYMENT-REQUIRED header (base64-encoded JSON)
    if let Some(pr_header) = resp.headers().get("payment-required") {
        if let Ok(header_str) = pr_header.to_str() {
            if let Ok(decoded_bytes) =
                base64::engine::general_purpose::STANDARD.decode(header_str)
            {
                if let Ok(decoded_str) = String::from_utf8(decoded_bytes) {
                    if let Ok(requirements) =
                        serde_json::from_str::<serde_json::Value>(&decoded_str)
                    {
                        return Ok(requirements);
                    }
                }
            }
        }
    }

    // Fallback: parse from response body
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("invalid 402 response body: {e}"))?;

    // Check if body has a "requirements" sub-object
    if let Some(reqs) = body.get("requirements") {
        return Ok(reqs.clone());
    }
    Ok(body)
}
