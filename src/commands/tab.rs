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

    let body = serde_json::json!({
        "provider": args.provider,
        "amount": amount,
        "max_charge_per_call": max_charge,
    });
    let resp = ctx.post("/tabs", &body).await?;

    if ctx.json {
        error::print_json(&resp);
    } else {
        let tab_id = resp["tab_id"].as_str().unwrap_or("unknown");
        let fee = resp["activation_fee"].as_u64().unwrap_or(0);
        error::success(&format!(
            "Tab opened: {tab_id} ({}, fee: {})",
            super::format_amount(amount),
            super::format_amount(fee),
        ));
    }
    Ok(())
}

async fn run_close(args: TabCloseArgs, ctx: &super::Context) -> Result<()> {
    let resp = ctx
        .post(
            &format!("/tabs/{}/close", args.tab_id),
            &serde_json::json!({}),
        )
        .await?;

    if ctx.json {
        error::print_json(&resp);
    } else {
        let charged = resp["total_charged"].as_u64().unwrap_or(0);
        let count = resp["charge_count"].as_i64().unwrap_or(0);
        error::success(&format!(
            "Tab {} closed — {} charged over {} calls",
            args.tab_id,
            super::format_amount(charged),
            count,
        ));
    }
    Ok(())
}

async fn run_charge(args: TabChargeArgs, ctx: &super::Context) -> Result<()> {
    let amount = super::parse_amount(&args.amount)?;
    let body = serde_json::json!({ "amount": amount });
    let resp = ctx
        .post(&format!("/tabs/{}/charge", args.tab_id), &body)
        .await?;

    if ctx.json {
        error::print_json(&resp);
    } else {
        let status = resp["status"].as_str().unwrap_or("unknown");
        let remaining = resp["balance_remaining"].as_u64().unwrap_or(0);
        error::success(&format!(
            "Charge {} on {} [{}] — {} remaining",
            super::format_amount(amount),
            args.tab_id,
            status,
            super::format_amount(remaining),
        ));
    }
    Ok(())
}

async fn run_topup(args: TabTopupArgs, ctx: &super::Context) -> Result<()> {
    let amount = super::parse_amount(&args.amount)?;
    let body = serde_json::json!({ "amount": amount });
    let resp = ctx
        .post(&format!("/tabs/{}/topup", args.tab_id), &body)
        .await?;

    if ctx.json {
        error::print_json(&resp);
    } else {
        let new_balance = resp["new_balance"].as_u64().unwrap_or(0);
        error::success(&format!(
            "Topped up {} on {} — balance: {}",
            super::format_amount(amount),
            args.tab_id,
            super::format_amount(new_balance),
        ));
    }
    Ok(())
}

async fn run_list(ctx: &super::Context) -> Result<()> {
    let resp = ctx.get("/tabs").await?;

    if ctx.json {
        error::print_json(&resp);
    } else {
        let tabs = resp.as_array();
        match tabs {
            Some(tabs) if !tabs.is_empty() => {
                for tab in tabs {
                    let id = tab["id"].as_str().unwrap_or("?");
                    let provider = tab["provider"].as_str().unwrap_or("?");
                    let balance = tab["balance_remaining"].as_u64().unwrap_or(0);
                    let status = tab["status"].as_str().unwrap_or("?");
                    error::print_kv(&[
                        ("Tab", id),
                        ("Provider", provider),
                        ("Balance", &super::format_amount(balance)),
                        ("Status", status),
                    ]);
                }
            }
            _ => error::success("No open tabs"),
        }
    }
    Ok(())
}
