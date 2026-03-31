use anyhow::Result;
use clap::Args;

use crate::error;

#[derive(Args)]
pub struct StatusArgs {
    /// Wallet address to check
    #[arg(long)]
    pub wallet: Option<String>,
}

pub async fn run(args: StatusArgs, mut ctx: super::Context) -> Result<()> {
    super::require_init()?;

    let wallet = args.wallet.unwrap_or_default();
    let path = format!("/status?wallet={wallet}");
    let resp = ctx.get(&path).await?;

    if ctx.json {
        error::print_json(&resp);
    } else {
        let balance = resp["balance_usdc"].as_str().unwrap_or("unknown");
        let tabs = resp["open_tabs"].as_i64().unwrap_or(0);
        let locked = resp["total_locked"].as_u64().unwrap_or(0);
        error::print_kv(&[
            ("Balance", &format!("{balance} USDC")),
            ("Open tabs", &tabs.to_string()),
            ("Locked", &super::format_amount(locked)),
        ]);
    }
    Ok(())
}
