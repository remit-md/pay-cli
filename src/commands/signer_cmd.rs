//! `pay signer` — advanced wallet management (init, import, export).
//!
//! For the default first-time setup, see `pay init`.
//! For plain dev keys, see `pay key init`.

use anyhow::{bail, Result};
use clap::{Args, Subcommand};
use std::io::IsTerminal;

use crate::auth;
use crate::config::Config;
use crate::signer::{keyring, keystore, password};

#[derive(Subcommand)]
#[command(long_about = "Advanced wallet management. Key resolution order: \
        PAYSKILL_SIGNER_KEY env var > OS keychain (.meta) > encrypted file (.enc). \
        Use `pay init` for first-time setup.")]
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
    /// Private key as hex (with or without 0x prefix).
    /// WARNING: visible in shell history. Prefer --key-file or stdin.
    #[arg(long)]
    key: Option<String>,

    /// Read private key from a file (first line, hex)
    #[arg(long)]
    key_file: Option<String>,

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
        SignerAction::Init(args) => run_init(args).await,
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

async fn run_init(args: SignerInitArgs) -> Result<()> {
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
            let mut config = Config::default();
            if let Err(e) = config.bootstrap_from_server().await {
                eprintln!("Warning: could not fetch config from server: {e}");
            }
            config.save()?;
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
            let mut config = Config::default();
            if let Err(e) = config.bootstrap_from_server().await {
                eprintln!("Warning: could not fetch config from server: {e}");
            }
            config.save()?;
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

/// Resolve the private key hex from --key, --key-file, or stdin.
fn resolve_import_key(args: &SignerImportArgs) -> Result<String> {
    if let Some(ref k) = args.key {
        return Ok(k.clone());
    }

    if let Some(ref path) = args.key_file {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read key file '{}': {e}", path))?;
        let line = contents.lines().next().unwrap_or("").trim().to_string();
        if line.is_empty() {
            bail!("key file '{}' is empty", path);
        }
        return Ok(line);
    }

    // Try stdin (only if piped, not interactive)
    if !std::io::stdin().is_terminal() {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
            .map_err(|e| anyhow::anyhow!("failed to read key from stdin: {e}"))?;
        let line = buf.lines().next().unwrap_or("").trim().to_string();
        if line.is_empty() {
            bail!("no key provided on stdin");
        }
        return Ok(line);
    }

    bail!(
        "no private key provided. Use one of:\n  \
         --key-file <path>     read from a file (recommended)\n  \
         echo <key> | pay signer import   pipe via stdin\n  \
         --key <hex>           pass directly (visible in shell history)"
    );
}

fn run_import(args: SignerImportArgs) -> Result<()> {
    if wallet_exists(&args.name)? {
        bail!(
            "Wallet '{}' already exists. Delete it first or use a different --name.",
            args.name
        );
    }

    // Resolve key from --key, --key-file, or stdin
    let key_hex = resolve_import_key(&args)?;

    // Parse and validate key
    let hex_clean = key_hex.trim_start_matches("0x");
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
        ks.import(&args.name, &key_hex, &pw)?;

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

    // Verify identity via OS biometric/password before revealing the key.
    // This MUST succeed — no fallback, no bypass.
    crate::os_auth::verify_identity(
        "Pay CLI needs to verify your identity to export the private key.",
    )?;

    // Load the key
    let hex_key = load_key_for_export(&args)?;

    // Copy to clipboard with auto-clear instead of printing to stdout
    let clear_secs = 15;
    if crate::os_auth::clipboard_copy_and_clear(&hex_key, clear_secs) {
        eprintln!();
        eprintln!("Private key copied to clipboard.");
        eprintln!("Clipboard will auto-clear in {clear_secs} seconds.");
        eprintln!();
        eprintln!("DO NOT paste this anywhere except a wallet import screen.");

        // Wait for clipboard to clear before exiting
        std::thread::sleep(std::time::Duration::from_secs(clear_secs));
        eprintln!("Clipboard cleared.");
    } else {
        // Clipboard unavailable — print to stdout as last resort
        eprintln!("WARNING: Could not copy to clipboard. Printing to stdout.");
        println!("{hex_key}");
    }

    Ok(())
}

/// Load the private key hex from whichever source is available.
fn load_key_for_export(args: &SignerExportArgs) -> Result<String> {
    // Explicit .enc file path
    if let Some(keystore_path) = &args.keystore {
        let path = std::path::PathBuf::from(keystore_path);
        let key_file = keystore::load_file(&path)?;
        let pw = password::acquire_for_decrypt()?;
        let key = keystore::decrypt(&key_file, &pw)?;
        return Ok(format!("0x{}", hex::encode(key.to_bytes())));
    }

    // Try .meta (keychain) first
    if let Ok(true) = keyring::MetaFile::exists(&args.name) {
        if let Ok(meta) = keyring::MetaFile::load(&args.name) {
            if meta.storage == "keychain" {
                let raw = keyring::load_key(&args.name)?;
                return Ok(format!("0x{}", hex::encode(&raw)));
            }
        }
    }

    // Try .enc file
    let ks = keystore::Keystore::open()?;
    if ks.exists(&args.name) {
        let key_file = ks.load(&args.name)?;
        let pw = password::acquire_for_decrypt()?;
        let key = keystore::decrypt(&key_file, &pw)?;
        return Ok(format!("0x{}", hex::encode(key.to_bytes())));
    }

    bail!(
        "No wallet '{}' found. Check --name or --keystore path.",
        args.name
    );
}
