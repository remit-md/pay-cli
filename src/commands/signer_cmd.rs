//! `pay signer` — advanced wallet management (init, import, export, backup, restore).
//!
//! For the default first-time setup, see `pay init`.
//! For plain dev keys, see `pay key init`.

use anyhow::{bail, Result};
use clap::{Args, Subcommand};
use std::io::IsTerminal;
use std::path::PathBuf;

use crate::auth;
use crate::config::Config;
use crate::os_auth;
use crate::signer::{keyring, keystore, password};

#[derive(Subcommand)]
pub enum SignerAction {
    /// Create a new wallet (supports named wallets and --no-keychain)
    Init(SignerInitArgs),
    /// Import an existing private key
    Import(SignerImportArgs),
    /// Export private key to clipboard (OS auth required, never printed)
    Export(SignerExportArgs),
    /// Create encrypted backup file (OS auth required)
    Backup(SignerBackupArgs),
    /// Restore key from encrypted backup file
    Restore(SignerRestoreArgs),
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

    /// Clipboard auto-clear timeout in seconds (default: 15)
    #[arg(long, default_value = "15")]
    clear_after: u64,
}

#[derive(Args)]
pub struct SignerBackupArgs {
    /// Wallet name (default: "default")
    #[arg(long, default_value = "default")]
    name: String,

    /// Output file path (default: ./pay-backup-{name}.enc)
    #[arg(long, short)]
    output: Option<String>,
}

#[derive(Args)]
pub struct SignerRestoreArgs {
    /// Path to encrypted backup file
    file: String,

    /// Wallet name to restore as (default: "default")
    #[arg(long, default_value = "default")]
    name: String,

    /// Force encrypted file storage instead of OS keychain
    #[arg(long)]
    no_keychain: bool,
}

pub async fn run(action: SignerAction, _ctx: super::Context) -> Result<()> {
    match action {
        SignerAction::Init(args) => run_init(args),
        SignerAction::Import(args) => run_import(args),
        SignerAction::Export(args) => run_export(args),
        SignerAction::Backup(args) => run_backup(args),
        SignerAction::Restore(args) => run_restore(args),
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

        keyring::store_key(&args.name, &raw)?;

        let meta = keyring::MetaFile {
            version: 2,
            name: args.name.clone(),
            address: address.clone(),
            storage: "keychain".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        meta.write_to_disk()?;

        if !Config::is_initialized() {
            Config::default().save()?;
        }

        eprintln!("Wallet '{}' created: {address}", args.name);
        eprintln!("Key stored in OS keychain (encrypted by your login).");
        eprintln!("No password or env vars needed.");
    } else {
        // Generate key, encrypt with password, write .enc
        let pw = password::acquire_for_encrypt()?;
        let ks = keystore::Keystore::open()?;
        let address = ks.generate(&args.name, &pw)?;

        if !Config::is_initialized() {
            Config::default().save()?;
        }

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
        keyring::store_key(&args.name, &raw)?;

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
    if !std::io::stderr().is_terminal() {
        bail!("Export requires an interactive terminal.");
    }

    // OS auth gate — verify identity before key access
    os_auth::verify_identity("Pay CLI needs to verify your identity to export the private key.")?;

    // Load the key
    let hex_key = load_key_hex(&args.name)?;

    // Clipboard only — never print to terminal
    if os_auth::clipboard_copy_and_clear(&hex_key, args.clear_after) {
        eprintln!("Private key copied to clipboard.");
        eprintln!("Clipboard will auto-clear in {} seconds.", args.clear_after);
        eprintln!();
        eprintln!("DO NOT paste this anywhere except a wallet import screen.");

        // Wait for the clear thread to finish before exiting
        std::thread::sleep(std::time::Duration::from_secs(args.clear_after));
        eprintln!("Clipboard cleared.");
    } else {
        bail!(
            "Failed to copy to clipboard. No clipboard command available.\n\
             On Linux, install xclip or xsel."
        );
    }

    Ok(())
}

fn run_backup(args: SignerBackupArgs) -> Result<()> {
    if !std::io::stderr().is_terminal() {
        bail!("Backup requires an interactive terminal.");
    }

    // OS auth gate
    os_auth::verify_identity(
        "Pay CLI needs to verify your identity to create an encrypted backup.",
    )?;

    // Load the raw key
    let raw = load_key_raw(&args.name)?;

    // Get address for the backup file metadata
    let key = k256::ecdsa::SigningKey::from_slice(&*raw)
        .map_err(|_| anyhow::anyhow!("invalid key"))?;
    let address = auth::derive_address(&key);

    // Prompt for backup encryption password
    eprintln!("Set a password for the backup file.");
    eprintln!("You'll need this password to restore on a new machine.");
    let pw = password::acquire_for_encrypt()?;

    // Determine output path
    let output_path = match &args.output {
        Some(p) => PathBuf::from(p),
        None => {
            let filename = format!("pay-backup-{}.enc", args.name);
            PathBuf::from(&filename)
        }
    };

    // Create the encrypted backup
    let encryption = keystore::encrypt_key(&*raw, &pw)?;
    let key_file = keystore::EncryptedKeyFile {
        version: 2,
        name: args.name.clone(),
        address: address.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        encryption,
    };
    let json = serde_json::to_string_pretty(&key_file)
        .map_err(|e| anyhow::anyhow!("failed to serialize backup: {e}"))?;
    std::fs::write(&output_path, json)
        .map_err(|e| anyhow::anyhow!("failed to write backup: {e}"))?;

    eprintln!("Encrypted backup written to: {}", output_path.display());
    eprintln!("Address: {address}");
    eprintln!();
    eprintln!("To restore on another machine:");
    eprintln!("  pay signer restore {}", output_path.display());

    Ok(())
}

fn run_restore(args: SignerRestoreArgs) -> Result<()> {
    if wallet_exists(&args.name)? {
        bail!(
            "Wallet '{}' already exists. Delete it first or use a different --name.",
            args.name
        );
    }

    let path = PathBuf::from(&args.file);
    if !path.exists() {
        bail!("Backup file not found: {}", path.display());
    }

    // Decrypt the backup file
    eprintln!("Enter the password for this backup file:");
    let key_file = keystore::load_file(&path)?;
    let pw = password::acquire_for_decrypt()?;
    let signing_key = keystore::decrypt(&key_file, &pw)?;
    let address = auth::derive_address(&signing_key);

    // Store in the local keychain or as encrypted file
    let raw_bytes = signing_key.to_bytes();
    let mut raw = zeroize::Zeroizing::new([0u8; 32]);
    raw.copy_from_slice(&raw_bytes);

    let use_keychain = !args.no_keychain && keyring::is_available();

    if use_keychain {
        keyring::store_key(&args.name, &raw)?;

        let meta = keyring::MetaFile {
            version: 2,
            name: args.name.clone(),
            address: address.clone(),
            storage: "keychain".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        meta.write_to_disk()?;

        if !Config::is_initialized() {
            Config::default().save()?;
        }

        eprintln!("Restored wallet '{}': {address}", args.name);
        eprintln!("Key stored in OS keychain.");
    } else {
        let ks = keystore::Keystore::open()?;
        let hex_key = format!("0x{}", hex::encode(&*raw));
        // Re-encrypt with a new password for local storage
        eprintln!("Set a local password for this machine:");
        let local_pw = password::acquire_for_encrypt()?;
        ks.import(&args.name, &hex_key, &local_pw)?;

        if !Config::is_initialized() {
            Config::default().save()?;
        }

        eprintln!("Restored wallet '{}': {address}", args.name);
        eprintln!("Key stored as encrypted file.");
    }

    Ok(())
}

// ── Key loading helpers ───────────────────────────────────────────

fn load_key_hex(name: &str) -> Result<String> {
    let raw = load_key_raw(name)?;
    Ok(format!("0x{}", hex::encode(&*raw)))
}

fn load_key_raw(name: &str) -> Result<zeroize::Zeroizing<[u8; 32]>> {
    // Try .meta (keychain) first
    if let Ok(true) = keyring::MetaFile::exists(name) {
        if let Ok(meta) = keyring::MetaFile::load(name) {
            if meta.storage == "keychain" {
                return keyring::load_key(name);
            }
        }
    }

    // Try .enc file
    let ks = keystore::Keystore::open()?;
    if ks.exists(name) {
        let key_file = ks.load(name)?;
        let pw = password::acquire_for_decrypt()?;
        let key = keystore::decrypt(&key_file, &pw)?;
        let mut raw = zeroize::Zeroizing::new([0u8; 32]);
        raw.copy_from_slice(&key.to_bytes());
        return Ok(raw);
    }

    bail!(
        "No wallet '{name}' found. Check --name or run `pay init`."
    );
}
