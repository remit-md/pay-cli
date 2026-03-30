use anyhow::Result;
use clap::Args;

use crate::error;

#[derive(Args)]
pub struct DirectArgs {
    /// Recipient wallet address (0x...)
    pub to: String,
    /// Amount in USDC (e.g., "5.00" for $5)
    pub amount: String,
    /// Optional memo
    #[arg(long)]
    pub memo: Option<String>,
}

pub async fn run(args: DirectArgs, ctx: super::Context) -> Result<()> {
    super::require_init()?;
    super::validate_address(&args.to)?;
    let amount = super::parse_amount(&args.amount)?;
    if amount < 1_000_000 {
        anyhow::bail!("Minimum direct payment is $1.00");
    }

    // Stub: will call POST /api/v1/direct when server exists
    let _ = ctx.api_url();
    if ctx.json {
        error::print_json(&serde_json::json!({
            "status": "not_implemented",
            "to": args.to,
            "amount": amount,
        }));
    } else {
        error::success(&format!(
            "Direct payment {} to {} (not yet connected to server)",
            super::format_amount(amount),
            args.to,
        ));
    }
    Ok(())
}
