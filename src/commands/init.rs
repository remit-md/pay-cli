use anyhow::Result;
use clap::Args;

use crate::auth;
use crate::config::Config;
use crate::error;
use crate::keystore;

#[derive(Args)]
pub struct InitArgs;

pub async fn run(_args: InitArgs, _ctx: super::Context) -> Result<()> {
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
        getrandom::fill(&mut pw_bytes)
            .map_err(|e| anyhow::anyhow!("rng failed: {e}"))?;
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
