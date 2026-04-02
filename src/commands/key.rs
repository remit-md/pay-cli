//! `pay key` subcommand — plain private key generation (dev/testing).

use anyhow::Result;
use clap::{Args, Subcommand};
use k256::ecdsa::SigningKey;

use crate::error;

#[derive(Subcommand)]
pub enum KeyAction {
    /// Generate a new private key
    Init(KeyInitArgs),
}

#[derive(Args)]
pub struct KeyInitArgs {
    /// Write PAYSKILL_KEY=0x... to .env file
    #[arg(long)]
    write_env: bool,
}

pub async fn run(action: KeyAction, _ctx: super::Context) -> Result<()> {
    match action {
        KeyAction::Init(args) => run_init(args),
    }
}

fn run_init(args: KeyInitArgs) -> Result<()> {
    // Generate random private key
    let mut rng_bytes = [0u8; 32];
    getrandom::fill(&mut rng_bytes).map_err(|e| anyhow::anyhow!("rng failed: {e}"))?;

    let signing_key = SigningKey::from_bytes((&rng_bytes).into())
        .map_err(|e| anyhow::anyhow!("invalid key bytes: {e}"))?;

    let private_key_hex = format!("0x{}", hex::encode(signing_key.to_bytes()));
    let address = crate::auth::derive_address(&signing_key);

    if args.write_env {
        let env_line = format!("PAYSKILL_KEY={private_key_hex}\n");
        let env_path = std::path::Path::new(".env");

        if env_path.exists() {
            let content = std::fs::read_to_string(env_path)
                .map_err(|e| anyhow::anyhow!("failed to read .env: {e}"))?;
            if content.contains("PAYSKILL_KEY=") {
                anyhow::bail!(".env already contains PAYSKILL_KEY — remove it first");
            }
            let mut new_content = content;
            if !new_content.ends_with('\n') {
                new_content.push('\n');
            }
            new_content.push_str(&env_line);
            std::fs::write(env_path, new_content)
                .map_err(|e| anyhow::anyhow!("failed to write .env: {e}"))?;
        } else {
            std::fs::write(env_path, env_line)
                .map_err(|e| anyhow::anyhow!("failed to create .env: {e}"))?;
        }

        error::success(&format!("Key written to .env — address: {address}"));
    }

    // Always output JSON for machine consumption
    let json = serde_json::json!({
        "address": address,
        "private_key": private_key_hex,
    });
    println!("{}", serde_json::to_string_pretty(&json)
        .map_err(|e| anyhow::anyhow!("JSON serialization failed: {e}"))?);

    Ok(())
}
