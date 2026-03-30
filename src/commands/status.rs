use anyhow::Result;
use clap::Args;

use crate::error;

#[derive(Args)]
pub struct StatusArgs;

pub async fn run(_args: StatusArgs, ctx: super::Context) -> Result<()> {
    super::require_init()?;

    // Stub: will call GET /api/v1/status when server exists
    if ctx.json {
        error::print_json(&serde_json::json!({
            "status": "not_implemented",
            "api_url": ctx.api_url(),
        }));
    } else {
        error::success("Status (not yet connected to server)");
        error::print_kv(&[("API", ctx.api_url())]);
    }
    Ok(())
}
