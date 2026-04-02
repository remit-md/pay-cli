//! `pay init` — default AES-256-GCM signer setup.
//!
//! For alternative signer paths, see:
//! - `pay ows init` — OWS (Open Wallet Standard) wallet
//! - `pay key init` — plain private key (dev/testing)

use anyhow::Result;
use clap::Args;

use crate::auth;
use crate::config::Config;
use crate::error;
use crate::keystore;

/// Initialize a Pay agent wallet (default signer).
///
/// Generates a secp256k1 keypair and stores it encrypted with AES-256-GCM.
/// This is Pay's own signer — priority #1. For other signer modes, see:
///   pay ows init   — Open Wallet Standard
///   pay key init   — plain private key
#[derive(Args)]
pub struct InitArgs;

pub async fn run(_args: InitArgs, _ctx: super::Context) -> Result<()> {
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
