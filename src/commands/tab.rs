use anyhow::Result;
use clap::{Args, Subcommand};

use crate::error;

#[derive(Args)]
pub struct TabArgs {
    #[command(subcommand)]
    pub action: TabAction,
}

#[derive(Subcommand)]
pub enum TabAction {
    /// Open a pre-funded metered tab
    Open(TabOpenArgs),
    /// Close a tab
    Close(TabCloseArgs),
    /// Charge a tab (provider-side)
    Charge(TabChargeArgs),
    /// Add funds to an open tab
    Topup(TabTopupArgs),
    /// List open tabs
    List,
}

#[derive(Args)]
pub struct TabOpenArgs {
    /// Provider wallet address (0x...)
    pub provider: String,
    /// Amount to lock in USDC (e.g., "20.00")
    pub amount: String,
    /// Maximum charge per call in USDC
    #[arg(long = "max-charge")]
    pub max_charge: String,
}

#[derive(Args)]
pub struct TabCloseArgs {
    /// Tab ID
    pub tab_id: String,
}

#[derive(Args)]
pub struct TabChargeArgs {
    /// Tab ID
    pub tab_id: String,
    /// Charge amount in USDC
    pub amount: String,
}

#[derive(Args)]
pub struct TabTopupArgs {
    /// Tab ID
    pub tab_id: String,
    /// Top-up amount in USDC
    pub amount: String,
}

pub async fn run(args: TabArgs, ctx: super::Context) -> Result<()> {
    super::require_init()?;
    let _ = ctx.api_url();

    match args.action {
        TabAction::Open(a) => run_open(a, &ctx).await,
        TabAction::Close(a) => run_close(a, &ctx).await,
        TabAction::Charge(a) => run_charge(a, &ctx).await,
        TabAction::Topup(a) => run_topup(a, &ctx).await,
        TabAction::List => run_list(&ctx).await,
    }
}

async fn run_open(args: TabOpenArgs, ctx: &super::Context) -> Result<()> {
    super::validate_address(&args.provider)?;
    let amount = super::parse_amount(&args.amount)?;
    if amount < 5_000_000 {
        anyhow::bail!("Minimum tab amount is $5.00");
    }
    let max_charge = super::parse_amount(&args.max_charge)?;

    if ctx.json {
        error::print_json(&serde_json::json!({
            "status": "not_implemented",
            "provider": args.provider,
            "amount": amount,
            "max_charge_per_call": max_charge,
        }));
    } else {
        error::success(&format!(
            "Tab open {} with {} (not yet connected)",
            super::format_amount(amount),
            args.provider,
        ));
    }
    Ok(())
}

async fn run_close(args: TabCloseArgs, ctx: &super::Context) -> Result<()> {
    if ctx.json {
        error::print_json(&serde_json::json!({
            "status": "not_implemented",
            "tab_id": args.tab_id,
        }));
    } else {
        error::success(&format!("Tab close {} (not yet connected)", args.tab_id));
    }
    Ok(())
}

async fn run_charge(args: TabChargeArgs, ctx: &super::Context) -> Result<()> {
    let amount = super::parse_amount(&args.amount)?;
    if ctx.json {
        error::print_json(&serde_json::json!({
            "status": "not_implemented",
            "tab_id": args.tab_id,
            "amount": amount,
        }));
    } else {
        error::success(&format!(
            "Tab charge {} on {} (not yet connected)",
            super::format_amount(amount),
            args.tab_id,
        ));
    }
    Ok(())
}

async fn run_topup(args: TabTopupArgs, ctx: &super::Context) -> Result<()> {
    let amount = super::parse_amount(&args.amount)?;
    if ctx.json {
        error::print_json(&serde_json::json!({
            "status": "not_implemented",
            "tab_id": args.tab_id,
            "amount": amount,
        }));
    } else {
        error::success(&format!(
            "Tab topup {} on {} (not yet connected)",
            super::format_amount(amount),
            args.tab_id,
        ));
    }
    Ok(())
}

async fn run_list(ctx: &super::Context) -> Result<()> {
    if ctx.json {
        error::print_json(&serde_json::json!({
            "status": "not_implemented",
            "tabs": [],
        }));
    } else {
        error::success("No open tabs (not yet connected to server)");
    }
    Ok(())
}
