//! Encrypted key storage for the pay CLI.
//!
//! Keys are stored as AES-256-GCM encrypted files at ~/.pay/keys/default.enc.
//! The encryption password comes from PAYSKILL_SIGNER_KEY env var.
//!
//! File format: JSON { "nonce": hex, "ciphertext": hex }

use std::fs;
use std::path::PathBuf;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{bail, Context, Result};
use k256::ecdsa::SigningKey;
use serde::{Deserialize, Serialize};

const KEYS_DIR: &str = "keys";
const DEFAULT_KEY_FILE: &str = "default.enc";

#[derive(Serialize, Deserialize)]
struct EncryptedKey {
    nonce: String,
    ciphertext: String,
}

/// Generate a new random signing key.
pub fn generate_key() -> Result<SigningKey> {
    let mut key_bytes = [0u8; 32];
    getrandom::fill(&mut key_bytes).map_err(|e| anyhow::anyhow!("rng failed: {e}"))?;
    SigningKey::from_slice(&key_bytes).context("failed to create signing key from random bytes")
}

/// Store a signing key encrypted at ~/.pay/keys/default.enc.
pub fn store_key(key: &SigningKey, password: &str) -> Result<()> {
    let path = key_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create keys dir: {}", parent.display()))?;
    }

    let key_bytes = key.to_bytes();
    let cipher = derive_cipher(password)?;

    let mut nonce_bytes = [0u8; 12];
    getrandom::fill(&mut nonce_bytes).map_err(|e| anyhow::anyhow!("rng failed: {e}"))?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, key_bytes.as_slice())
        .map_err(|e| anyhow::anyhow!("encryption failed: {e}"))?;

    let encrypted = EncryptedKey {
        nonce: hex::encode(nonce_bytes),
        ciphertext: hex::encode(ciphertext),
    };

    let json = serde_json::to_string_pretty(&encrypted)?;
    fs::write(&path, json)
        .with_context(|| format!("failed to write key file: {}", path.display()))?;

    Ok(())
}

/// Load a signing key from ~/.pay/keys/default.enc.
pub fn load_key(password: &str) -> Result<SigningKey> {
    let path = key_path();
    if !path.exists() {
        bail!("No key found. Run `pay init` first.");
    }

    let json = fs::read_to_string(&path)
        .with_context(|| format!("failed to read key file: {}", path.display()))?;
    let encrypted: EncryptedKey =
        serde_json::from_str(&json).context("failed to parse key file")?;

    let nonce_bytes = hex::decode(&encrypted.nonce).context("invalid nonce in key file")?;
    let ciphertext =
        hex::decode(&encrypted.ciphertext).context("invalid ciphertext in key file")?;

    let cipher = derive_cipher(password)?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext.as_slice())
        .map_err(|_| anyhow::anyhow!("decryption failed — wrong password or corrupted key file"))?;

    SigningKey::from_slice(&plaintext).context("invalid key data after decryption")
}

/// Load signing key from env var or encrypted file.
///
/// Priority:
/// 1. PAYSKILL_SIGNER_KEY as raw hex private key
/// 2. Encrypted file with PAYSKILL_SIGNER_KEY as password
pub fn resolve_key() -> Result<SigningKey> {
    let password = std::env::var("PAYSKILL_SIGNER_KEY")
        .context("PAYSKILL_SIGNER_KEY not set. Run `pay init` and set PAYSKILL_SIGNER_KEY.")?;

    // Try as raw hex key first
    let clean = password.strip_prefix("0x").unwrap_or(&password);
    if clean.len() == 64 {
        if let Ok(bytes) = hex::decode(clean) {
            if let Ok(key) = SigningKey::from_slice(&bytes) {
                return Ok(key);
            }
        }
    }

    // Fall back to encrypted file with password
    load_key(&password)
}

/// Check if a key file exists.
pub fn key_exists() -> bool {
    key_path().exists()
}

fn key_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pay")
        .join(KEYS_DIR)
        .join(DEFAULT_KEY_FILE)
}

/// Derive an AES-256 cipher from a password using SHA-256.
fn derive_cipher(password: &str) -> Result<Aes256Gcm> {
    use sha3::{Digest, Sha3_256};
    let key = Sha3_256::digest(password.as_bytes());
    Aes256Gcm::new_from_slice(&key).map_err(|e| anyhow::anyhow!("cipher init failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::derive_address;

    #[test]
    fn generate_key_produces_valid_key() {
        let key = generate_key().unwrap();
        let addr = derive_address(&key);
        assert!(addr.starts_with("0x"));
        assert_eq!(addr.len(), 42);
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let key = generate_key().unwrap();
        let password = "test-password-123";

        // Use a temp dir instead of ~/.pay
        let dir = std::env::temp_dir().join("pay-test-keystore");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let path = dir.join("test.enc");

        // Encrypt
        let key_bytes = key.to_bytes();
        let cipher = derive_cipher(password).unwrap();
        let mut nonce_bytes = [0u8; 12];
        getrandom::fill(&mut nonce_bytes).unwrap();
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher.encrypt(nonce, key_bytes.as_slice()).unwrap();

        let encrypted = EncryptedKey {
            nonce: hex::encode(nonce_bytes),
            ciphertext: hex::encode(ciphertext),
        };
        fs::write(&path, serde_json::to_string(&encrypted).unwrap()).unwrap();

        // Decrypt
        let json = fs::read_to_string(&path).unwrap();
        let enc: EncryptedKey = serde_json::from_str(&json).unwrap();
        let n = hex::decode(&enc.nonce).unwrap();
        let ct = hex::decode(&enc.ciphertext).unwrap();
        let cipher2 = derive_cipher(password).unwrap();
        let plaintext = cipher2
            .decrypt(Nonce::from_slice(&n), ct.as_slice())
            .unwrap();

        let recovered = SigningKey::from_slice(&plaintext).unwrap();
        assert_eq!(derive_address(&key), derive_address(&recovered));

        let _ = fs::remove_dir_all(&dir);
    }
}
