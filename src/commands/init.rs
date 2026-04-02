//! `pay init` — first-time wallet setup.
//!
//! Keychain-first: if OS keychain is available, stores key there (no password
//! needed). Falls back to encrypted .enc file with interactive password prompt.
//!
//! For alternative signer paths, see:
//! - `pay signer init` — named wallets, --no-keychain
//! - `pay ows init` — OWS (Open Wallet Standard) wallet
//! - `pay key init` — plain private key (dev/testing)

use anyhow::Result;
use clap::Args;

use crate::auth;
use crate::config::Config;
use crate::error;
use crate::signer::{keyring, keystore, password};

#[derive(Args)]
pub struct InitArgs {
    /// Force encrypted file storage instead of OS keychain
    #[arg(long)]
    no_keychain: bool,
}

pub async fn run(args: InitArgs, _ctx: super::Context) -> Result<()> {
    // Already initialized?
    let ks = keystore::Keystore::open()?;
    let has_enc = ks.exists("default");
    let has_meta = keyring::MetaFile::exists("default").unwrap_or(false);

    if Config::is_initialized() && (has_enc || has_meta) {
        // Try to resolve the existing key to show the address
        if has_meta {
            if let Ok(meta) = keyring::MetaFile::load("default") {
                error::success(&format!("Already initialized. Wallet: {}", meta.address));
                return Ok(());
            }
        }
        if has_enc {
            // Try env var or old keystore to resolve
            match crate::signer::resolve_key() {
                Ok(key) => {
                    let addr = auth::derive_address(&key);
                    error::success(&format!("Already initialized. Wallet: {addr}"));
                    return Ok(());
                }
                Err(_) => {
                    eprintln!("Wallet exists but cannot resolve key.");
                    eprintln!("Set PAYSKILL_SIGNER_KEY to unlock, or delete ~/.pay/ to start fresh.");
                    return Ok(());
                }
            }
        }
        error::success("Already initialized.");
        return Ok(());
    }

    let use_keychain = !args.no_keychain && keyring::is_available();

    if use_keychain {
        // Generate key, store in OS keychain, write .meta
        let mut raw = zeroize::Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut *raw).map_err(|e| anyhow::anyhow!("rng failed: {e}"))?;

        let key = k256::ecdsa::SigningKey::from_slice(&*raw)
            .map_err(|_| anyhow::anyhow!("failed to create signing key"))?;
        let address = auth::derive_address(&key);

        keyring::store_key("default", &*raw)?;

        let meta = keyring::MetaFile {
            version: 2,
            name: "default".to_string(),
            address: address.clone(),
            storage: "keychain".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        meta.write_to_disk()?;

        let config = Config::default();
        config.save()?;

        error::success(&format!("Wallet initialized: {address}"));
        eprintln!();
        eprintln!("  Key stored in OS keychain (encrypted by your login).");
        eprintln!("  No password or env vars needed.");
    } else {
        // Generate key, encrypt with password, write .enc
        let pw = password::acquire_for_encrypt()?;
        let address = ks.generate("default", &pw)?;

        let config = Config::default();
        config.save()?;

        error::success(&format!("Wallet initialized: {address}"));
        eprintln!();
        eprintln!("  Key stored as encrypted file.");
        eprintln!("  Set PAYSKILL_SIGNER_KEY to your password for non-interactive use.");
    }

    Ok(())
}
