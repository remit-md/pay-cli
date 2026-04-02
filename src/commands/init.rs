use anyhow::Result;
use clap::Args;

use crate::auth;
use crate::config::Config;
use crate::error;
use crate::keystore;
use crate::ows;

/// Initialize a Pay agent wallet.
///
/// Default: Pay's own signer with AES-256-GCM encrypted key storage.
/// Use --ows for OWS (Open Wallet Standard) signer.
#[derive(Args)]
pub struct InitArgs {
    /// Wallet name (default: pay-{hostname}, only used with --ows)
    #[arg(long)]
    pub name: Option<String>,

    /// Chain: "base" (mainnet) or "base-sepolia" (testnet), only used with --ows
    #[arg(long)]
    pub chain: Option<String>,

    /// Use OWS (Open Wallet Standard) instead of Pay's local signer
    #[arg(long)]
    pub ows: bool,
}

pub async fn run(args: InitArgs, ctx: super::Context) -> Result<()> {
    if args.ows {
        return run_ows(args, ctx).await;
    }

    // Default: Pay's own signer (priority #1)
    run_default(ctx).await
}

/// Default init: Pay's own AES-256-GCM encrypted signer.
async fn run_default(_ctx: super::Context) -> Result<()> {
    if Config::is_initialized() && keystore::key_exists() {
        let key = keystore::resolve_key()?;
        let addr = auth::derive_address(&key);
        error::success(&format!("Already initialized. Wallet: {addr}"));
        return Ok(());
    }

    let key = keystore::generate_key()?;
    let addr = auth::derive_address(&key);

    let password = std::env::var("PAYSKILL_SIGNER_KEY").unwrap_or_default();
    if password.is_empty() {
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

/// Helper to extract a string from a JSON value.
fn jstr(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// OWS init: create wallet in OWS vault with chain-lock policy via `ows` CLI.
async fn run_ows(args: InitArgs, ctx: super::Context) -> Result<()> {
    let chain = args.chain.unwrap_or_else(ows::detect_chain);
    let wallet_name = args.name.unwrap_or_else(ows::default_wallet_name);

    ows::chain_to_caip2(&chain)?;

    // Step 1: Check if OWS is installed
    if !ows::is_ows_available() {
        println!("OWS not detected. Installing via npm...");
        ows::install_ows_via_npm()?;

        if !ows::is_ows_available() {
            return Err(anyhow::anyhow!(
                "OWS installation failed. Install manually:\n  \
                 npm install -g @open-wallet-standard/core\n  \
                 Or use `pay init` (without --ows) for Pay's local signer."
            ));
        }
        error::success("OWS installed");
    }

    // Step 2: Create wallet
    println!("Creating wallet '{wallet_name}'...");
    let wallet = ows::create_wallet(&wallet_name)?;
    let address = ows::wallet_evm_address(&wallet)
        .ok_or_else(|| anyhow::anyhow!("wallet has no EVM account"))?;
    error::success(&format!("Wallet created: {address}"));

    // Step 3: Create chain-lock policy
    let policy = ows::create_chain_policy(&chain)?;
    let policy_id = jstr(&policy, "id");
    error::success(&format!(
        "Policy created: {policy_id} (chain lock: {chain})"
    ));

    // Step 4: Create API key bound to wallet + policy
    let wallet_id = jstr(&wallet, "id");
    let key_result = ows::create_api_key(&wallet_id, &policy_id)?;
    let token_field = jstr(&key_result, "token");
    let token = if token_field.is_empty() {
        jstr(&key_result, "api_key")
    } else {
        token_field
    };
    error::success("API key created");

    // Step 5: Output
    if ctx.json {
        error::print_json(&serde_json::json!({
            "wallet_id": wallet_id,
            "wallet_name": jstr(&wallet, "name"),
            "address": address,
            "chain": chain,
            "policy_id": policy_id,
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
            ("Policy", &policy_id),
            ("Vault", &ows::vault_path_display()),
        ]);
        println!();
        eprintln!("API Key (save this — shown once):");
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
