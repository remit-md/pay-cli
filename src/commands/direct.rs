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

pub async fn run(args: DirectArgs, mut ctx: super::Context) -> Result<()> {
    super::require_init()?;
    super::validate_address(&args.to)?;
    let amount = super::parse_amount(&args.amount)?;
    if amount < 1_000_000 {
        anyhow::bail!("Minimum direct payment is $1.00");
    }

    let body = serde_json::json!({
        "to": args.to,
        "amount": amount,
        "memo": args.memo.unwrap_or_default(),
    });

    let resp = ctx.post("/direct", &body).await?;

    if ctx.json {
        error::print_json(&resp);
    } else {
        let tx = resp["tx_hash"].as_str().unwrap_or("pending");
        let status = resp["status"].as_str().unwrap_or("unknown");
        error::success(&format!(
            "Sent {} to {} [{}] tx: {}",
            super::format_amount(amount),
            args.to,
            status,
            tx,
        ));
    }
    Ok(())
}
