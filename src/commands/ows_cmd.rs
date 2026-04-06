//! `pay ows` subcommand — OWS wallet management via `ows` CLI subprocess.

use anyhow::{bail, Result};
use clap::{Args, Subcommand};
use serde_json::Value;

use crate::error;
use crate::ows;

#[derive(Subcommand)]
pub enum OwsAction {
    /// Initialize an OWS wallet for Pay
    Init(InitArgs),
    /// List OWS wallets
    List(ListArgs),
    /// Fund an OWS wallet (opens browser)
    Fund(FundArgs),
    /// Set a spending policy on an OWS wallet
    SetPolicy(SetPolicyArgs),
}

#[derive(Args)]
pub struct InitArgs {
    /// Wallet name (default: pay-{hostname})
    #[arg(long)]
    name: Option<String>,

    /// Chain (base or base-sepolia)
    #[arg(long, default_value = "base")]
    chain: String,

    /// Install OWS CLI via npm if not found
    #[arg(long)]
    install: bool,
}

#[derive(Args)]
pub struct ListArgs {
    /// Output as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
pub struct FundArgs {
    /// Wallet name or ID
    #[arg(long, env = "OWS_WALLET_ID")]
    wallet: String,
}

#[derive(Args)]
pub struct SetPolicyArgs {
    /// Wallet name or ID
    #[arg(long, env = "OWS_WALLET_ID")]
    wallet: Option<String>,

    /// Chain (base or base-sepolia)
    #[arg(long, default_value = "base")]
    chain: String,

    /// Max USDC per transaction (e.g. "10.00")
    #[arg(long, allow_hyphen_values = true)]
    max_tx: String,

    /// Daily spending limit in USDC (e.g. "100.00")
    #[arg(long, allow_hyphen_values = true)]
    daily_limit: String,
}

/// Helper to extract a string from a JSON value.
fn jstr(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or("").to_string()
}

pub async fn run(action: OwsAction, ctx: super::Context) -> Result<()> {
    match action {
        OwsAction::Init(args) => run_init(args).await,
        OwsAction::List(args) => run_list(args, &ctx),
        OwsAction::Fund(args) => run_fund(args, ctx),
        OwsAction::SetPolicy(args) => run_set_policy(args),
    }
}

async fn run_init(args: InitArgs) -> Result<()> {
    // Check / install OWS CLI
    if !ows::is_ows_available() {
        if args.install {
            eprintln!("Installing OWS CLI via npm...");
            ows::install_ows_via_npm()?;
            error::success("OWS CLI installed");
        } else {
            bail!(
                "ows CLI not found. Install with:\n  \
                 npm install -g @open-wallet-standard/cli\n\n\
                 Or run: pay ows init --install"
            );
        }
    }

    let name = args.name.unwrap_or_else(ows::default_wallet_name);

    // Check if wallet already exists
    if let Ok(existing) = ows::get_wallet(&name) {
        let addr = ows::wallet_evm_address(&existing).unwrap_or_default();
        error::success(&format!("OWS wallet already exists: {name}"));
        if !addr.is_empty() {
            eprintln!("  EVM address: {addr}");
        }
        eprintln!();
        eprintln!("  MCP config:");
        eprintln!("  {}", ows::mcp_config_json(&name, &args.chain));
        return Ok(());
    }

    // Validate chain
    ows::chain_to_caip2(&args.chain)?;

    // Create wallet
    let wallet = ows::create_wallet(&name)?;
    let addr = ows::wallet_evm_address(&wallet).unwrap_or_default();
    let wallet_id = jstr(&wallet, "id");

    error::success(&format!("OWS wallet created: {name}"));
    if !addr.is_empty() {
        eprintln!("  EVM address: {addr}");
    }
    eprintln!("  Wallet ID: {wallet_id}");
    eprintln!("  Vault: {}", ows::vault_path_display());
    eprintln!();
    eprintln!("  MCP config:");
    eprintln!("  {}", ows::mcp_config_json(&name, &args.chain));

    Ok(())
}

fn run_list(args: ListArgs, ctx: &super::Context) -> Result<()> {
    let wallets = ows::list_wallets()?;

    if args.json || ctx.json {
        let json = serde_json::to_string_pretty(&wallets)
            .map_err(|e| anyhow::anyhow!("JSON serialization failed: {e}"))?;
        println!("{json}");
        return Ok(());
    }

    if wallets.is_empty() {
        eprintln!("No OWS wallets found. Run `pay ows init` to create one.");
        return Ok(());
    }

    for w in &wallets {
        let name = jstr(w, "name");
        let id = jstr(w, "id");
        let addr = ows::wallet_evm_address(w).unwrap_or_else(|| "no EVM address".to_string());
        println!("{name}  {addr}  ({id})");
    }

    Ok(())
}

fn run_fund(args: FundArgs, ctx: super::Context) -> Result<()> {
    let wallet = ows::get_wallet(&args.wallet)?;
    let addr = ows::wallet_evm_address(&wallet)
        .ok_or_else(|| anyhow::anyhow!("wallet has no EVM address"))?;

    let url = if ctx.config.is_testnet() {
        format!("https://testnet.pay-skill.com/fund?wallet={addr}")
    } else {
        format!("https://pay-skill.com/fund?wallet={addr}")
    };

    error::success(&format!("Open to fund: {url}"));
    let _ = open_url(&url);

    Ok(())
}

fn run_set_policy(args: SetPolicyArgs) -> Result<()> {
    // Validate chain
    ows::chain_to_caip2(&args.chain)?;

    // Validate amounts are positive
    let max_tx: f64 = args
        .max_tx
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid max-tx amount: {}", args.max_tx))?;
    if max_tx <= 0.0 {
        bail!("max-tx must be positive, got: {}", args.max_tx);
    }

    let daily: f64 = args
        .daily_limit
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid daily-limit amount: {}", args.daily_limit))?;
    if daily <= 0.0 {
        bail!("daily-limit must be positive, got: {}", args.daily_limit);
    }

    let wallet_name = args.wallet.unwrap_or_else(ows::default_wallet_name);
    let wallet = ows::get_wallet(&wallet_name)?;
    let wallet_id = jstr(&wallet, "id");

    let policy = ows::create_spending_policy(&args.chain, Some(max_tx), Some(daily))?;
    let policy_id = jstr(&policy, "id");

    let key_stdout = ows::create_api_key(&wallet_id, &policy_id)?;
    let key_value = ows::parse_api_token(&key_stdout).unwrap_or_default();

    error::success(&format!("Spending policy set on wallet: {wallet_name}"));
    eprintln!("  Max per tx: ${}", args.max_tx);
    eprintln!("  Daily limit: ${}", args.daily_limit);
    eprintln!("  Chain: {}", args.chain);
    if !key_value.is_empty() {
        eprintln!("  API key: {key_value}");
    }

    Ok(())
}

/// Open a URL in the default browser (cross-platform).
fn open_url(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()?;
    }
    Ok(())
}
