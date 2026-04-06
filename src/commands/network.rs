//! `pay network` — show or switch network (testnet/mainnet).

use anyhow::{bail, Result};
use clap::Args;

use crate::config::Config;
use crate::error;

#[derive(Args)]
pub struct NetworkArgs {
    /// Network to switch to: "testnet" or "mainnet"
    network: Option<String>,
}

pub async fn run(args: NetworkArgs, ctx: super::Context) -> Result<()> {
    match args.network.as_deref() {
        None => show_network(ctx),
        Some("testnet") => switch_network(ctx, true).await,
        Some("mainnet") => switch_network(ctx, false).await,
        Some(other) => bail!("Unknown network: {other}. Use \"testnet\" or \"mainnet\"."),
    }
}

fn show_network(ctx: super::Context) -> Result<()> {
    let network = ctx.config.network_name();
    let chain_id = ctx.config.chain_id();
    let api = ctx.config.api_url();
    let router = ctx.config.router_address();

    if ctx.json {
        error::print_json(&serde_json::json!({
            "network": if ctx.config.is_testnet() { "testnet" } else { "mainnet" },
            "name": network,
            "chain_id": chain_id,
            "api_url": api,
            "router_address": router,
        }));
    } else {
        error::print_kv(&[
            ("Network", network),
            ("Chain ID", &chain_id.to_string()),
            ("API", api),
            ("Router", router),
        ]);
    }
    Ok(())
}

async fn switch_network(ctx: super::Context, testnet: bool) -> Result<()> {
    let mut config = Config::load()?;

    if testnet {
        config.set_testnet();
    } else {
        config.set_mainnet();
    }

    if let Err(e) = config.bootstrap_from_server().await {
        eprintln!("Warning: could not fetch config from server: {e}");
        eprintln!(
            "Router address will use default. Run `pay network` again when server is reachable."
        );
    }

    config.save()?;

    let network = config.network_name();
    if ctx.json {
        error::print_json(&serde_json::json!({
            "network": if testnet { "testnet" } else { "mainnet" },
            "name": network,
            "chain_id": config.chain_id(),
            "api_url": config.api_url(),
            "router_address": config.router_address(),
        }));
    } else {
        error::success(&format!("Switched to {network}"));
    }
    Ok(())
}
