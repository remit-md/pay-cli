use anyhow::Result;
use clap::Subcommand;

use crate::commands::Context;
use crate::error;
use crate::ows;

#[derive(Subcommand)]
pub enum WalletAction {
    /// List all OWS wallets
    List,
    /// Generate a fund link for a wallet
    Fund(WalletFundArgs),
    /// Set spending policy on a wallet
    SetPolicy(SetPolicyArgs),
    /// Update wallet display settings
    Settings(WalletSettingsArgs),
}

#[derive(clap::Args)]
pub struct WalletFundArgs {
    /// Wallet name or ID (default: detect from OWS_WALLET_ID env)
    #[arg(long)]
    pub wallet: Option<String>,

    /// Pre-fill amount in USDC
    #[arg(long)]
    pub amount: Option<String>,
}

#[derive(clap::Args)]
pub struct SetPolicyArgs {
    /// Chain: "base" or "base-sepolia"
    #[arg(long)]
    pub chain: Option<String>,

    /// Per-transaction spending cap in USDC (e.g., 500)
    #[arg(long)]
    pub max_tx: Option<f64>,

    /// Daily spending cap in USDC (e.g., 5000)
    #[arg(long)]
    pub daily_limit: Option<f64>,
}

#[derive(clap::Args)]
pub struct WalletSettingsArgs {
    /// Set the wallet display name
    #[arg(long)]
    pub display_name: Option<String>,
}

pub async fn run(action: WalletAction, ctx: Context) -> Result<()> {
    match action {
        WalletAction::List => run_list(ctx).await,
        WalletAction::Fund(args) => run_fund(args, ctx).await,
        WalletAction::SetPolicy(args) => run_set_policy(args, ctx).await,
        WalletAction::Settings(args) => run_settings(args, ctx).await,
    }
}

async fn run_list(ctx: Context) -> Result<()> {
    let wallets = ows::list_wallets()?;

    if wallets.is_empty() {
        if ctx.json {
            error::print_json(&serde_json::json!([]));
        } else {
            println!("No OWS wallets found. Run `pay init --ows` to create one.");
        }
        return Ok(());
    }

    if ctx.json {
        let entries: Vec<serde_json::Value> = wallets
            .iter()
            .map(|w| {
                let address = ows::wallet_evm_address(w).unwrap_or_default();
                serde_json::json!({
                    "id": w.id,
                    "name": w.name,
                    "address": address,
                    "created_at": w.created_at,
                    "accounts": w.accounts.len(),
                })
            })
            .collect();
        error::print_json(&entries);
    } else {
        let mut table = comfy_table::Table::new();
        table.set_header(vec!["Name", "Address", "ID", "Created"]);
        for w in &wallets {
            let address = ows::wallet_evm_address(w).unwrap_or_else(|| "\u{2014}".to_string());
            let short_id = if w.id.len() > 8 {
                format!("{}...", &w.id[..8])
            } else {
                w.id.clone()
            };
            table.add_row(vec![w.name.clone(), address, short_id, w.created_at.clone()]);
        }
        println!("{table}");
    }

    Ok(())
}

async fn run_fund(args: WalletFundArgs, ctx: Context) -> Result<()> {
    // Resolve wallet
    let wallet_name = args
        .wallet
        .or_else(|| std::env::var("OWS_WALLET_ID").ok())
        .ok_or_else(|| {
            anyhow::anyhow!("no wallet specified. Use --wallet or set OWS_WALLET_ID.")
        })?;

    let wallet = ows::get_wallet(&wallet_name)?;
    let address = ows::wallet_evm_address(&wallet)
        .ok_or_else(|| anyhow::anyhow!("wallet has no EVM account"))?;

    // Build the fund link URL
    let is_testnet = ctx.config.is_testnet();
    let base = if is_testnet {
        "https://testnet.pay-skill.com"
    } else {
        "https://pay-skill.com"
    };

    let mut url = format!("{base}/fund/{address}");
    if let Some(ref amount) = args.amount {
        url = format!("{url}?amount={amount}");
    }

    if ctx.json {
        error::print_json(&serde_json::json!({
            "url": url,
            "wallet": wallet.name,
            "address": address,
        }));
    } else {
        error::print_kv(&[("Wallet", &wallet.name), ("Address", &address)]);
        println!();
        error::success(&format!("Fund link: {url}"));
        println!();
        println!("Open this link to add USDC to your wallet.");

        // Try to open in browser
        if open_url(&url).is_err() {
            println!("(Could not open browser automatically)");
        }
    }

    Ok(())
}

async fn run_set_policy(args: SetPolicyArgs, ctx: Context) -> Result<()> {
    let chain = args.chain.unwrap_or_else(ows::detect_chain);

    // Validate chain
    ows::chain_to_caip2(&chain)?;

    let has_limits = args.max_tx.is_some() || args.daily_limit.is_some();

    let policy = if has_limits {
        // Validate positive amounts
        if let Some(max_tx) = args.max_tx {
            if max_tx <= 0.0 {
                return Err(anyhow::anyhow!("--max-tx must be positive, got {max_tx}"));
            }
        }
        if let Some(daily) = args.daily_limit {
            if daily <= 0.0 {
                return Err(anyhow::anyhow!(
                    "--daily-limit must be positive, got {daily}"
                ));
            }
        }

        ows::create_spending_policy(&chain, args.max_tx, args.daily_limit)?
    } else {
        ows::create_chain_policy(&chain)?
    };

    if ctx.json {
        error::print_json(&serde_json::json!({
            "policy_id": policy.id,
            "name": policy.name,
            "chain": chain,
            "max_tx_usdc": args.max_tx,
            "daily_limit_usdc": args.daily_limit,
            "rules": policy.rules.len(),
        }));
    } else {
        error::success(&format!("Policy '{}' saved", policy.id));
        error::print_kv(&[
            ("Policy ID", &policy.id),
            ("Name", &policy.name),
            ("Chain", &chain),
            (
                "Max per-tx",
                &args
                    .max_tx
                    .map(|v| format!("${v}"))
                    .unwrap_or_else(|| "unlimited".to_string()),
            ),
            (
                "Daily limit",
                &args
                    .daily_limit
                    .map(|v| format!("${v}"))
                    .unwrap_or_else(|| "unlimited".to_string()),
            ),
        ]);

        if has_limits {
            println!();
            println!("Spending limits use the @pay-skill/ows-policy executable.");
            println!("Install it: npm install -g @pay-skill/ows-policy");
        }
    }

    Ok(())
}

async fn run_settings(args: WalletSettingsArgs, ctx: Context) -> Result<()> {
    if ctx.json {
        error::print_json(&serde_json::json!({
            "display_name": args.display_name,
        }));
    } else {
        error::print_kv(&[(
            "Display Name",
            args.display_name
                .as_deref()
                .unwrap_or("(not set)"),
        )]);
    }

    Ok(())
}

/// Try to open a URL in the default browser.
fn open_url(url: &str) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to open browser: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to open browser: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to open browser: {e}"))?;
    }
    Ok(())
}
