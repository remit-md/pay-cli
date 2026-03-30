mod commands;
mod config;
#[allow(dead_code)]
mod error;

use anyhow::Result;
use clap::{Parser, Subcommand};

use commands::Context;
use config::Config;

/// pay — payment infrastructure for AI agents
#[derive(Parser)]
#[command(name = "pay", version, about)]
struct Cli {
    /// Output as JSON instead of human-readable format
    #[arg(long, global = true)]
    json: bool,

    /// Override API URL
    #[arg(long, global = true, env = "PAYSKILL_API_URL")]
    api_url: Option<String>,

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
    /// Open funding page
    Fund,
    /// Withdraw USDC
    Withdraw(WithdrawArgs),
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
    if let Some(url) = &cli.api_url {
        config.api_url = Some(url.clone());
    }

    let ctx = Context::new(cli.json, config);

    match cli.command {
        Commands::Init(args) => commands::init::run(args, ctx).await,
        Commands::Status(args) => commands::status::run(args, ctx).await,
        Commands::Direct(args) => commands::direct::run(args, ctx).await,
        Commands::Tab(args) => commands::tab::run(args, ctx).await,
        Commands::Request(args) => commands::request::run(args, ctx).await,
        Commands::Webhook(args) => commands::webhook::run(args, ctx).await,
        Commands::Sign(args) => commands::sign::run(args, ctx).await,
        Commands::Fund => {
            commands::require_init()?;
            let resp = ctx.get("/fund-link").await?;
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
            let amount = commands::parse_amount(&args.amount)?;
            let path = format!("/withdraw-link?amount={amount}&to={}", args.to);
            let resp = ctx.get(&path).await?;
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
    }
}
