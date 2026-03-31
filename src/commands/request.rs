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

    // 402 — parse payment requirements
    let requirements: serde_json::Value = resp.json().await?;
    let settlement = requirements["settlement"].as_str().unwrap_or("direct");
    let amount = requirements["amount"].as_u64().unwrap_or(0);
    let to = requirements["to"].as_str().unwrap_or("");

    if !ctx.json {
        error::success(&format!(
            "402 Payment Required: {} ({settlement})",
            super::format_amount(amount),
        ));
    }

    // Pay via the appropriate settlement mode
    if settlement == "tab" {
        let charge_body = serde_json::json!({ "amount": amount });
        // Try to find existing tab, or auto-open
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
            let tab_amount = std::cmp::max(amount * 10, 5_000_000);
            let open_body = serde_json::json!({
                "provider": to,
                "amount": tab_amount,
                "max_charge_per_call": amount,
            });
            let open_resp = ctx.post("/tabs", &open_body).await?;
            open_resp["tab_id"].as_str().unwrap_or("").to_string()
        };

        ctx.post(&format!("/tabs/{tab_id}/charge"), &charge_body)
            .await?;
    } else {
        let pay_body = serde_json::json!({
            "to": to,
            "amount": amount,
        });
        ctx.post("/direct", &pay_body).await?;
    }

    // Retry the original request
    let retry_resp = ctx.http.get(&args.url).send().await?;
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
