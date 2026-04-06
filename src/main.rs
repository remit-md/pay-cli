mod auth;
mod commands;
mod config;
mod eip3009;
#[allow(dead_code)]
mod error;
#[allow(dead_code)]
mod ows;
mod permit;
mod signer;

use anyhow::Result;
use clap::{Parser, Subcommand};

use commands::Context;
use config::Config;

/// pay — payment infrastructure for AI agents
#[derive(Parser)]
#[command(
    name = "pay",
    version,
    about,
    after_help = "Output is JSON by default. Use --plain for human-readable output."
)]
struct Cli {
    /// Human-readable output instead of JSON (JSON is the default)
    #[arg(long, global = true)]
    plain: bool,

    /// Use testnet (Base Sepolia) for this command only
    #[arg(long, global = true)]
    testnet: bool,

    /// Override API URL
    #[arg(long, global = true, env = "PAYSKILL_API_URL", hide = true)]
    api_url: Option<String>,

    /// Override chain ID
    #[arg(long, global = true, env = "PAYSKILL_CHAIN_ID", hide = true)]
    chain_id: Option<u64>,

    /// Override router contract address
    #[arg(long, global = true, env = "PAYSKILL_ROUTER_ADDRESS", hide = true)]
    router_address: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// First-time wallet setup
    Init(commands::init::InitArgs),
    /// Wallet balance and open tabs
    Status(commands::status::StatusArgs),
    /// Send a one-shot USDC payment
    Direct(commands::direct::DirectArgs),
    /// Tab management (open, close, charge, topup, list)
    Tab(commands::tab::TabArgs),
    /// Make an x402 request (auto-handles payment)
    Request(commands::request::RequestArgs),
    /// Webhook management (register, list, delete)
    Webhook(commands::webhook::WebhookArgs),
    /// Signer subprocess (stdin/stdout protocol for SDKs)
    Sign(commands::sign::SignArgs),
    /// OWS (Open Wallet Standard) wallet management
    Ows {
        #[command(subcommand)]
        action: commands::ows_cmd::OwsAction,
    },
    /// Advanced wallet management (init, import, export)
    Signer {
        #[command(subcommand)]
        action: commands::signer_cmd::SignerAction,
    },
    /// Plain private key management (dev/testing)
    Key {
        #[command(subcommand)]
        action: commands::key::KeyAction,
    },
    /// Show or switch network (testnet/mainnet)
    Network(commands::network::NetworkArgs),
    /// Show wallet address
    Address,
    /// Open funding page
    Fund,
    /// Withdraw USDC
    Withdraw(WithdrawArgs),
    /// Mint testnet USDC (testnet only)
    Mint(MintArgs),
}

#[derive(clap::Args)]
struct MintArgs {
    /// Amount in USDC (e.g., "100.00")
    amount: String,
}

#[derive(clap::Args)]
struct WithdrawArgs {
    /// Recipient address
    to: String,
    /// Amount in USDC
    amount: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let mut config = Config::load()?;
    if cli.testnet {
        config.set_testnet();
    }
    if let Some(url) = &cli.api_url {
        config.api_url = Some(url.clone());
    }
    if let Some(id) = cli.chain_id {
        config.chain_id = Some(id);
    }
    if let Some(addr) = &cli.router_address {
        config.router_address = Some(addr.clone());
    }

    let json = !cli.plain;
    let mut ctx = Context::new(json, config);

    match cli.command {
        Commands::Init(args) => commands::init::run(args, ctx).await,
        Commands::Status(args) => commands::status::run(args, ctx).await,
        Commands::Direct(args) => commands::direct::run(args, ctx).await,
        Commands::Tab(args) => commands::tab::run(args, ctx).await,
        Commands::Request(args) => commands::request::run(args, ctx).await,
        Commands::Webhook(args) => commands::webhook::run(args, ctx).await,
        Commands::Sign(args) => commands::sign::run(args, ctx).await,
        Commands::Signer { action } => commands::signer_cmd::run(action, ctx).await,
        Commands::Ows { action } => commands::ows_cmd::run(action, ctx).await,
        Commands::Key { action } => commands::key::run(action, ctx).await,
        Commands::Network(args) => commands::network::run(args, ctx).await,
        Commands::Address => {
            commands::require_init()?;
            let addr = ctx.address()?;
            if ctx.json {
                error::print_json(&serde_json::json!({ "address": addr }));
            } else {
                println!("{addr}");
            }
            Ok(())
        }
        Commands::Fund => {
            commands::require_init()?;
            let resp = ctx.post("/links/fund", &serde_json::json!({})).await?;
            let url = resp["url"].as_str().unwrap_or("");
            if ctx.json {
                error::print_json(&resp);
            } else if url.is_empty() {
                error::success("Fund link not available");
            } else {
                error::success(&format!("Open to fund: {url}"));
            }
            Ok(())
        }
        Commands::Withdraw(args) => {
            commands::require_init()?;
            commands::validate_address(&args.to)?;
            let _amount = commands::parse_amount(&args.amount)?;
            let resp = ctx.post("/links/withdraw", &serde_json::json!({})).await?;
            let url = resp["url"].as_str().unwrap_or("");
            if ctx.json {
                error::print_json(&resp);
            } else if url.is_empty() {
                error::success("Withdraw link not available");
            } else {
                error::success(&format!("Open to withdraw: {url}"));
            }
            Ok(())
        }
        Commands::Mint(args) => {
            commands::require_init()?;
            // parse_amount returns micro-USDC, but /mint expects whole USDC
            let micro = commands::parse_amount(&args.amount)?;
            let whole = micro / 1_000_000;
            let wallet = ctx.address()?;
            let resp = ctx
                .post(
                    "/mint",
                    &serde_json::json!({ "wallet": wallet, "amount": whole }),
                )
                .await?;
            if ctx.json {
                error::print_json(&resp);
            } else {
                let tx = resp["tx_hash"].as_str().unwrap_or("unknown");
                error::success(&format!(
                    "Minted {} testnet USDC\n  Tx: {tx}",
                    commands::format_amount(micro)
                ));
            }
            Ok(())
        }
    }
}
