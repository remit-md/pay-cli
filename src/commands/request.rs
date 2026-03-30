use anyhow::Result;
use clap::Args;

use crate::error;

#[derive(Args)]
pub struct RequestArgs {
    /// URL to request (x402 payment handled automatically)
    pub url: String,
}

pub async fn run(args: RequestArgs, ctx: super::Context) -> Result<()> {
    super::require_init()?;

    // Stub: will implement x402 flow when server exists
    let _ = ctx.api_url();
    if ctx.json {
        error::print_json(&serde_json::json!({
            "status": "not_implemented",
            "url": args.url,
        }));
    } else {
        error::success(&format!("Request {} (not yet connected)", args.url));
    }
    Ok(())
}
