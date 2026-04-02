//! `pay signer` — advanced wallet management (init, import, export).
//!
//! For the default first-time setup, see `pay init`.
//! For plain dev keys, see `pay key init`.

use anyhow::{bail, Result};
use clap::{Args, Subcommand};
use std::io::IsTerminal;

use crate::auth;
use crate::signer::{keyring, keystore, password};

#[derive(Subcommand)]
pub enum SignerAction {
    /// Create a new wallet (supports named wallets and --no-keychain)
    Init(SignerInitArgs),
    /// Import an existing private key
    Import(SignerImportArgs),
    /// Export a private key (displays in plaintext)
    Export(SignerExportArgs),
}

#[derive(Args)]
pub struct SignerInitArgs {
    /// Wallet name (default: "default")
    #[arg(long, default_value = "default")]
    name: String,

    /// Force encrypted file storage instead of OS keychain
    #[arg(long)]
    no_keychain: bool,
}

#[derive(Args)]
pub struct SignerImportArgs {
    /// Private key as hex (with or without 0x prefix)
    #[arg(long)]
    key: String,

    /// Wallet name (default: "default")
    #[arg(long, default_value = "default")]
    name: String,

    /// Force encrypted file storage instead of OS keychain
    #[arg(long)]
    no_keychain: bool,
}

#[derive(Args)]
pub struct SignerExportArgs {
    /// Wallet name (default: "default")
    #[arg(long, default_value = "default")]
    name: String,

    /// Path to a specific .enc file to export from
    #[arg(long)]
    keystore: Option<String>,
}

pub async fn run(action: SignerAction, _ctx: super::Context) -> Result<()> {
    match action {
        SignerAction::Init(args) => run_init(args),
        SignerAction::Import(args) => run_import(args),
        SignerAction::Export(args) => run_export(args),
    }
}

/// Check if a wallet already exists (either .enc or .meta).
fn wallet_exists(name: &str) -> Result<bool> {
    let ks = keystore::Keystore::open()?;
    if ks.exists(name) {
        return Ok(true);
    }
    if keyring::MetaFile::exists(name)? {
        return Ok(true);
    }
    Ok(false)
}

fn run_init(args: SignerInitArgs) -> Result<()> {
    if wallet_exists(&args.name)? {
        bail!(
            "Wallet '{}' already exists. Delete it first or use a different --name.",
            args.name
        );
    }

    let use_keychain = !args.no_keychain && keyring::is_available();

    if use_keychain {
        // Generate key, store in OS keychain, write .meta
        let mut raw = zeroize::Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut *raw).map_err(|e| anyhow::anyhow!("rng failed: {e}"))?;

        let key = k256::ecdsa::SigningKey::from_slice(&*raw)
            .map_err(|_| anyhow::anyhow!("failed to create signing key"))?;
        let address = auth::derive_address(&key);

        keyring::store_key(&args.name, &*raw)?;

        let meta = keyring::MetaFile {
            version: 2,
            name: args.name.clone(),
            address: address.clone(),
            storage: "keychain".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        meta.write_to_disk()?;

        eprintln!("Wallet '{}' created: {address}", args.name);
        eprintln!("Key stored in OS keychain (encrypted by your login).");
        eprintln!("No password or env vars needed.");
    } else {
        // Generate key, encrypt with password, write .enc
        let pw = password::acquire_for_encrypt()?;
        let ks = keystore::Keystore::open()?;
        let address = ks.generate(&args.name, &pw)?;

        eprintln!("Wallet '{}' created: {address}", args.name);
        if args.no_keychain {
            eprintln!("Key stored as encrypted file.");
        } else {
            eprintln!("OS keychain not available. Key stored as encrypted file.");
        }
        eprintln!("Set PAYSKILL_SIGNER_KEY to your password to use non-interactively.");
    }

    Ok(())
}

fn run_import(args: SignerImportArgs) -> Result<()> {
    if wallet_exists(&args.name)? {
        bail!(
            "Wallet '{}' already exists. Delete it first or use a different --name.",
            args.name
        );
    }

    // Parse and validate key
    let hex_clean = args.key.trim_start_matches("0x");
    let raw_bytes = hex::decode(hex_clean).map_err(|_| anyhow::anyhow!("key is not valid hex"))?;
    if raw_bytes.len() != 32 {
        bail!("private key must be exactly 32 bytes (64 hex chars)");
    }
    let mut raw = zeroize::Zeroizing::new([0u8; 32]);
    raw.copy_from_slice(&raw_bytes);

    let key = k256::ecdsa::SigningKey::from_slice(&*raw)
        .map_err(|_| anyhow::anyhow!("not a valid secp256k1 private key"))?;
    let address = auth::derive_address(&key);

    let use_keychain = !args.no_keychain && keyring::is_available();

    if use_keychain {
        keyring::store_key(&args.name, &*raw)?;

        let meta = keyring::MetaFile {
            version: 2,
            name: args.name.clone(),
            address: address.clone(),
            storage: "keychain".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        meta.write_to_disk()?;

        eprintln!("Imported wallet '{}': {address}", args.name);
        eprintln!("Key stored in OS keychain.");
    } else {
        let pw = password::acquire_for_encrypt()?;
        let ks = keystore::Keystore::open()?;
        ks.import(&args.name, &args.key, &pw)?;

        eprintln!("Imported wallet '{}': {address}", args.name);
        eprintln!("Key stored as encrypted file.");
    }

    Ok(())
}

fn run_export(args: SignerExportArgs) -> Result<()> {
    // Require interactive terminal for safety
    if !std::io::stderr().is_terminal() {
        bail!("Export requires an interactive terminal for safety confirmation.");
    }

    eprintln!("WARNING: This will display your private key in plaintext.");
    eprintln!("Press Enter to continue, or Ctrl+C to cancel.");
    let mut buf = String::new();
    std::io::stdin()
        .read_line(&mut buf)
        .map_err(|e| anyhow::anyhow!("failed to read confirmation: {e}"))?;

    // Resolve key source
    if let Some(keystore_path) = &args.keystore {
        // Explicit .enc file
        let path = std::path::PathBuf::from(keystore_path);
        let key_file = keystore::load_file(&path)?;
        let pw = password::acquire_for_decrypt()?;
        let key = keystore::decrypt(&key_file, &pw)?;
        let hex = format!("0x{}", hex::encode(key.to_bytes()));
        println!("{hex}");
        return Ok(());
    }

    // Try .meta (keychain) first
    if let Ok(true) = keyring::MetaFile::exists(&args.name) {
        if let Ok(meta) = keyring::MetaFile::load(&args.name) {
            if meta.storage == "keychain" {
                let raw = keyring::load_key(&args.name)?;
                let hex = format!("0x{}", hex::encode(&*raw));
                println!("{hex}");
                return Ok(());
            }
        }
    }

    // Try .enc file
    let ks = keystore::Keystore::open()?;
    if ks.exists(&args.name) {
        let key_file = ks.load(&args.name)?;
        let pw = password::acquire_for_decrypt()?;
        let key = keystore::decrypt(&key_file, &pw)?;
        let hex = format!("0x{}", hex::encode(key.to_bytes()));
        println!("{hex}");
        return Ok(());
    }

    bail!(
        "No wallet '{}' found. Check --name or --keystore path.",
        args.name
    );
}
