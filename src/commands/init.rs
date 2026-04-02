use anyhow::Result;
use clap::Args;

use crate::auth;
use crate::config::Config;
use crate::error;
use crate::keystore;
use crate::ows;

/// Initialize a Pay agent wallet.
///
/// Default: stores key in OS keychain (no password needed).
/// Use --ows for OWS (Open Wallet Standard) signer.
#[derive(Args)]
pub struct InitArgs {
    /// Wallet name (default: pay-{hostname})
    #[arg(long)]
    pub name: Option<String>,

    /// Chain: "base" (mainnet) or "base-sepolia" (testnet)
    #[arg(long)]
    pub chain: Option<String>,

    /// Use OWS (Open Wallet Standard) instead of local signer
    #[arg(long)]
    pub ows: bool,
}

pub async fn run(args: InitArgs, ctx: super::Context) -> Result<()> {
    if args.ows {
        return run_ows(args, ctx).await;
    }

    // Default: local signer
    run_default(ctx).await
}

/// Default init: local signer (existing Pay logic).
async fn run_default(_ctx: super::Context) -> Result<()> {
    if Config::is_initialized() && keystore::key_exists() {
        let key = keystore::resolve_key()?;
        let addr = auth::derive_address(&key);
        error::success(&format!("Already initialized. Wallet: {addr}"));
        return Ok(());
    }

    // Generate a new keypair
    let key = keystore::generate_key()?;
    let addr = auth::derive_address(&key);

    // Determine password for encryption
    let password = std::env::var("PAYSKILL_SIGNER_KEY").unwrap_or_default();
    if password.is_empty() {
        // Generate a random password and tell the user to save it
        let mut pw_bytes = [0u8; 32];
        getrandom::fill(&mut pw_bytes).map_err(|e| anyhow::anyhow!("rng failed: {e}"))?;
        let generated_pw = hex::encode(pw_bytes);

        keystore::store_key(&key, &generated_pw)?;

        let config = Config::default();
        config.save()?;

        error::success(&format!("Wallet initialized: {addr}"));
        eprintln!();
        eprintln!("  Set this environment variable to unlock your wallet:");
        eprintln!("  export PAYSKILL_SIGNER_KEY={generated_pw}");
        eprintln!();
        eprintln!("  Save this value securely. You need it to sign transactions.");
    } else {
        keystore::store_key(&key, &password)?;

        let config = Config::default();
        config.save()?;

        error::success(&format!("Wallet initialized: {addr}"));
    }

    Ok(())
}

/// OWS init: create an OWS wallet + policy + API key.
async fn run_ows(args: InitArgs, ctx: super::Context) -> Result<()> {
    let chain = args.chain.unwrap_or_else(ows::detect_chain);
    let wallet_name = args.name.unwrap_or_else(ows::default_wallet_name);

    // Validate chain
    ows::chain_to_caip2(&chain)?;

    // Step 1: Check if OWS is installed
    if !ows::is_ows_available() {
        println!("OWS not detected. Installing via npm...");
        ows::install_ows_via_npm().map_err(|e| anyhow::anyhow!("failed to install OWS: {e}"))?;

        // Verify it worked
        if !ows::is_ows_available() {
            return Err(anyhow::anyhow!(
                "OWS installation failed. Install manually: npm install -g @open-wallet-standard/core\n\
                 Or use `pay init` (without --ows) for the local signer."
            ));
        }
        error::success("OWS installed");
    }

    // Step 2: Create wallet (no passphrase — API key auth only)
    println!("Creating wallet '{wallet_name}'...");
    let wallet = ows::create_wallet(&wallet_name)?;
    let address = ows::wallet_evm_address(&wallet)
        .ok_or_else(|| anyhow::anyhow!("wallet has no EVM account"))?;
    error::success(&format!("Wallet created: {address}"));

    // Step 3: Create chain-lock policy
    let policy = ows::create_chain_policy(&chain)?;
    error::success(&format!(
        "Policy created: {} (chain lock: {})",
        policy.id, chain
    ));

    // Step 4: Create API key bound to wallet + policy
    let (token, _key_file) = ows::create_api_key(&wallet.id, &policy.id)?;
    error::success("API key created");

    // Step 5: Output
    if ctx.json {
        error::print_json(&serde_json::json!({
            "wallet_id": wallet.id,
            "wallet_name": wallet.name,
            "address": address,
            "chain": chain,
            "policy_id": policy.id,
            "api_key": token,
            "mcp_config": serde_json::from_str::<serde_json::Value>(
                &ows::mcp_config_json(&wallet_name, &chain)
            ).unwrap_or_default(),
        }));
    } else {
        println!();
        error::print_kv(&[
            ("Wallet", &wallet_name),
            ("Address", &address),
            ("Chain", &chain),
            ("Policy", &policy.id),
            ("Vault", &ows::vault_path_display()),
        ]);
        println!();
        eprintln!("API Key (save this \u{2014} shown once):");
        eprintln!("  {token}");
        println!();
        eprintln!("MCP config (add to your claude_desktop_config.json):");
        eprintln!("{}", ows::mcp_config_json(&wallet_name, &chain));
        println!();
        eprintln!("Set OWS_API_KEY={token} in your environment.");
        eprintln!("Then your agent can use Pay via MCP with OWS-secured signing.");
    }

    Ok(())
}
