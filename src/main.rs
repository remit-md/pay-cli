mod auth;
mod commands;
mod config;
mod eip3009;
#[allow(dead_code)]
mod error;
mod os_auth;
#[allow(dead_code)]
mod ows;
mod permit;
mod signer;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};

use commands::Context;
use config::Config;

/// pay -- payment infrastructure for AI agents
#[derive(Parser)]
#[command(
    name = "pay",
    version,
    about = "pay -- payment infrastructure for AI agents",
    long_about = "pay -- payment infrastructure for AI agents\n\n\
        USDC payments on Base. Three primitives:\n\
        direct payments, metered tabs, and x402 HTTP paywalls.",
    after_help = "Output is JSON by default. Use --plain for human-readable output.",
    after_long_help = "Output is JSON by default. Use --plain for human-readable output.\n\n\
        EXAMPLES:\n  \
          pay init                               Set up wallet\n  \
          pay status                             Check balance\n  \
          pay direct 0xABC...DEF 5.00            Send $5 USDC\n  \
          pay tab open 0xABC...DEF 20.00         Open a $20 tab\n  \
          pay request https://api.example.com    Make a paid API call\n  \
          pay discover weather                   Find paid services\n  \
          pay update                             Update to latest version\n\n\
        LEARN MORE:\n  \
          https://pay-skill.com/docs/cli"
)]
struct Cli {
    /// Human-readable output instead of JSON (JSON is the default)
    #[arg(long, global = true)]
    plain: bool,

    /// Use testnet (Base Sepolia) for this command only
    #[arg(long, global = true)]
    testnet: bool,

    /// Override API URL
    #[arg(long, global = true, env = "PAYSKILL_API_URL", hide_short_help = true)]
    api_url: Option<String>,

    /// Override chain ID
    #[arg(long, global = true, env = "PAYSKILL_CHAIN_ID", hide_short_help = true)]
    chain_id: Option<u64>,

    /// Override router contract address
    #[arg(
        long,
        global = true,
        env = "PAYSKILL_ROUTER_ADDRESS",
        hide_short_help = true
    )]
    router_address: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    // -- Core payment commands --
    /// First-time wallet setup
    #[command(display_order = 1)]
    Init(commands::init::InitArgs),
    /// Wallet balance and open tabs
    #[command(display_order = 2, visible_alias = "balance")]
    Status(commands::status::StatusArgs),
    /// Send a one-shot USDC payment
    #[command(display_order = 3, visible_alias = "send")]
    Direct(commands::direct::DirectArgs),
    /// Tab management (open, close, charge, topup, list)
    #[command(display_order = 4)]
    Tab(commands::tab::TabArgs),
    /// Make an HTTP request with automatic x402 payment
    #[command(display_order = 5, visible_alias = "req")]
    Request(commands::request::RequestArgs),
    /// Search for paid API services
    #[command(display_order = 6)]
    Discover(commands::discover::DiscoverArgs),

    // -- Account & config commands --
    /// Webhook management (register, list, delete)
    #[command(display_order = 10)]
    Webhook(commands::webhook::WebhookArgs),
    /// Show current network or switch (testnet/mainnet)
    #[command(display_order = 11)]
    Network(commands::network::NetworkArgs),
    /// Show wallet address
    #[command(display_order = 12)]
    Address,
    /// Generate a funding link (opens in browser)
    #[command(display_order = 13)]
    Fund(FundArgs),
    /// Generate a withdrawal link (opens in browser)
    #[command(display_order = 14)]
    Withdraw(WithdrawArgs),
    /// Mint testnet USDC (testnet only)
    #[command(display_order = 15)]
    Mint(MintArgs),

    // -- Advanced commands --
    /// Advanced wallet management (init, import, export)
    #[command(display_order = 20)]
    Signer {
        #[command(subcommand)]
        action: commands::signer_cmd::SignerAction,
    },
    /// OWS (Open Wallet Standard) wallet management
    #[command(display_order = 21)]
    Ows {
        #[command(subcommand)]
        action: commands::ows_cmd::OwsAction,
    },
    /// Update pay to the latest version
    #[command(display_order = 22)]
    Update(commands::update::UpdateArgs),
    /// Generate shell completion scripts
    #[command(display_order = 23)]
    Completions(commands::completions::CompletionsArgs),

    // -- Hidden (internal) commands --
    /// Signer subprocess (stdin/stdout protocol for SDKs)
    #[command(hide = true)]
    Sign(commands::sign::SignArgs),
    /// Plain private key management (dev/testing)
    #[command(hide = true)]
    Key {
        #[command(subcommand)]
        action: commands::key::KeyAction,
    },
}

#[derive(clap::Args)]
struct FundArgs {
    /// Message to show the operator (repeatable)
    #[arg(long, short = 'm')]
    message: Vec<String>,

    /// Agent display name shown on the funding page
    #[arg(long)]
    name: Option<String>,
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

/// Build the CLI command tree (used by completions generator).
pub fn build_cli() -> clap::Command {
    Cli::command()
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
        Commands::Fund(args) => {
            commands::require_init()?;
            let messages: Vec<serde_json::Value> = args
                .message
                .iter()
                .map(|text| serde_json::json!({ "role": "agent", "text": text }))
                .collect();
            let mut body = serde_json::json!({ "messages": messages });
            if let Some(name) = &args.name {
                body["agent_name"] = serde_json::json!(name);
            }
            let resp = ctx.post("/links/fund", &body).await?;
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
            let micro = commands::parse_amount(&args.amount)?;

            // Sign USDC permit granting the relayer transferFrom allowance.
            let contracts = permit::get_contracts(&mut ctx).await?;
            let permit_sig = permit::prepare_and_sign(&mut ctx, micro, &contracts.relayer).await?;

            let resp = ctx
                .post(
                    "/links/withdraw",
                    &serde_json::json!({
                        "permit": {
                            "value": micro,
                            "deadline": permit_sig.deadline,
                            "v": permit_sig.v,
                            "r": permit_sig.r,
                            "s": permit_sig.s,
                        }
                    }),
                )
                .await?;
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
        Commands::Discover(args) => commands::discover::run(args, ctx).await,
        Commands::Update(args) => commands::update::run(args, ctx).await,
        Commands::Completions(args) => commands::completions::run(args),
    }
}
