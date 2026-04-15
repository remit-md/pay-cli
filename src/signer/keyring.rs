//! OS keychain backend for private key storage.
//!
//! Stores private keys as 0x-prefixed hex strings in the platform's native
//! credential store:
//! - macOS: Security.framework Keychain
//! - Windows: Credential Manager (DPAPI)
//! - Linux: Secret Service API (GNOME Keyring / KWallet)
//!
//! Hex string format ensures cross-language compatibility: both the Rust CLI
//! (`keyring::get_password`) and the Node SDK (`keytar.getPassword`) can read
//! the same credential as a UTF-8 string.
//!
//! Security invariants:
//! - K1: Raw key returned in `Zeroizing<[u8; 32]>` -- zeroed on drop.
//! - K2: Encrypted at rest by the OS. No extra password needed for interactive use.
//! - K3: No key material in error messages.
//! - K4: MetaFile contains ONLY public info (address, storage type). No secrets.
//! - K5: `is_available()` is non-destructive -- never stores, loads, or deletes.
//! - K6: `store_key` takes `&[u8; 32]` -- caller controls key lifetime.
//! - K7: `load_key` migrates legacy raw-byte entries to hex on read.
#![deny(unsafe_code)]
#![deny(clippy::unwrap_used)]

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use zeroize::Zeroizing;

/// Service name used in all keychain entries.
#[cfg(feature = "keychain")]
const KEYCHAIN_SERVICE: &str = "pay";

// -- MetaFile -----------------------------------------------------------------

/// On-disk metadata for keychain-stored keys.
///
/// Contains ONLY public information -- no secret material.
/// Written to `~/.pay/keys/{name}.meta` when using the keychain path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaFile {
    pub version: u32,
    pub name: String,
    /// Wallet address (0x-prefixed, lowercase). Public info.
    pub address: String,
    /// Storage backend: "keychain" or "file".
    pub storage: String,
    pub created_at: String,
}

impl MetaFile {
    /// Path to the meta file for a given wallet name.
    ///
    /// Uses `PAY_KEYS_DIR` env override if set (for testing).
    pub fn path(name: &str) -> Result<PathBuf> {
        let base = if let Ok(dir) = std::env::var("PAY_KEYS_DIR") {
            PathBuf::from(dir)
        } else {
            dirs::home_dir()
                .context("cannot locate home directory")?
                .join(".pay")
                .join("keys")
        };
        Ok(base.join(format!("{name}.meta")))
    }

    /// Write meta file to disk (creates parent dirs if needed).
    pub fn write_to_disk(&self) -> Result<()> {
        let path = Self::path(&self.name)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create keys directory: {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self).context("cannot serialize meta file")?;
        std::fs::write(&path, json)
            .with_context(|| format!("cannot write meta file: {}", path.display()))
    }

    /// Load meta file from disk.
    pub fn load(name: &str) -> Result<Self> {
        let path = Self::path(name)?;
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read meta file: {}", path.display()))?;
        serde_json::from_str(&contents)
            .with_context(|| format!("cannot parse meta file: {}", path.display()))
    }

    /// Check if a meta file exists for the given wallet name.
    pub fn exists(name: &str) -> Result<bool> {
        Ok(Self::path(name)?.exists())
    }

    /// Delete the meta file from disk.
    #[allow(dead_code)]
    pub fn delete(name: &str) -> Result<()> {
        let path = Self::path(name)?;
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("cannot delete meta file: {}", path.display()))?;
        }
        Ok(())
    }
}

// -- Keychain operations ------------------------------------------------------

/// Check if the OS keychain is available for use.
///
/// K5: Non-destructive -- never stores, loads, or deletes.
#[cfg(feature = "keychain")]
pub fn is_available() -> bool {
    let entry = match keyring::Entry::new(KEYCHAIN_SERVICE, "__pay_probe__") {
        Ok(e) => e,
        Err(_) => return false,
    };

    match entry.get_secret() {
        Ok(_) => true,
        Err(keyring::Error::NoEntry) => true,
        Err(_) => false,
    }
}

#[cfg(not(feature = "keychain"))]
pub fn is_available() -> bool {
    false
}

/// Store a 32-byte private key in the OS keychain as a 0x-prefixed hex string.
///
/// Hex format ensures both Rust (`get_password`) and Node (`keytar.getPassword`)
/// can read the credential reliably across platforms.
///
/// K6: Takes `&[u8; 32]` -- caller controls key lifetime.
#[cfg(feature = "keychain")]
pub fn store_key(label: &str, key: &[u8; 32]) -> Result<()> {
    let entry =
        keyring::Entry::new(KEYCHAIN_SERVICE, label).map_err(|e| anyhow!("keychain error: {e}"))?;
    let hex = format!("0x{}", hex::encode(key));
    entry
        .set_password(&hex)
        .map_err(|e| anyhow!("failed to store key in OS keychain: {e}"))
}

#[cfg(not(feature = "keychain"))]
pub fn store_key(_label: &str, _key: &[u8; 32]) -> Result<()> {
    Err(anyhow!(
        "OS keychain not available (static build). Use --no-keychain for encrypted file storage."
    ))
}

/// Retrieve a 32-byte private key from the OS keychain.
///
/// Tries hex string format first (new format). If the credential is not valid
/// hex, falls back to raw 32-byte format (legacy) and auto-migrates the entry
/// to hex so future reads (including from Node/keytar) work correctly.
///
/// K1: Returns `Zeroizing<[u8; 32]>` -- zeroed on drop.
/// K3: Error messages never include key material.
/// K7: Migrates legacy raw-byte entries to hex on read.
#[cfg(feature = "keychain")]
pub fn load_key(label: &str) -> Result<Zeroizing<[u8; 32]>> {
    let entry =
        keyring::Entry::new(KEYCHAIN_SERVICE, label).map_err(|e| anyhow!("keychain error: {e}"))?;

    // Try reading as a string first (hex format, new path).
    if let Ok(password) = entry.get_password() {
        let trimmed = password.trim();
        let hex_str = trimmed.strip_prefix("0x").unwrap_or(trimmed);
        if hex_str.len() == 64 {
            if let Ok(bytes) = hex::decode(hex_str) {
                if bytes.len() == 32 {
                    let mut arr = Zeroizing::new([0u8; 32]);
                    arr.copy_from_slice(&bytes);
                    return Ok(arr);
                }
            }
        }
    }

    // Fall back to raw bytes (legacy format).
    let secret = entry.get_secret().map_err(|e| match e {
        keyring::Error::NoEntry => {
            anyhow!("no key '{label}' found in OS keychain. Run: pay init")
        }
        keyring::Error::Ambiguous(_) => {
            anyhow!("multiple keychain entries found for '{label}' -- resolve manually")
        }
        other => anyhow!("failed to load key from OS keychain: {other}"),
    })?;

    if secret.len() != 32 {
        return Err(anyhow!(
            "keychain key has wrong length: expected 32 bytes, got {}",
            secret.len()
        ));
    }

    let mut arr = Zeroizing::new([0u8; 32]);
    arr.copy_from_slice(&secret);

    // K7: Auto-migrate legacy entry to hex format.
    let hex = format!("0x{}", hex::encode(*arr));
    if entry.set_password(&hex).is_ok() {
        eprintln!("pay: migrated keychain entry '{label}' from raw bytes to hex format");
    }

    Ok(arr)
}

#[cfg(not(feature = "keychain"))]
pub fn load_key(_label: &str) -> Result<Zeroizing<[u8; 32]>> {
    Err(anyhow!(
        "OS keychain not available (static build). Use encrypted .enc keystore instead."
    ))
}

/// Delete a key from the OS keychain.
#[allow(dead_code)]
#[cfg(feature = "keychain")]
pub fn delete_key(label: &str) -> Result<()> {
    let entry =
        keyring::Entry::new(KEYCHAIN_SERVICE, label).map_err(|e| anyhow!("keychain error: {e}"))?;
    entry.delete_credential().map_err(|e| match e {
        keyring::Error::NoEntry => {
            anyhow!("no key '{label}' found in OS keychain")
        }
        other => anyhow!("failed to delete key from OS keychain: {other}"),
    })
}

#[allow(dead_code)]
#[cfg(not(feature = "keychain"))]
pub fn delete_key(_label: &str) -> Result<()> {
    Err(anyhow!("OS keychain not available (static build)."))
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // -- MetaFile tests -------------------------------------------------------

    #[test]
    fn meta_file_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let meta_path = dir.path().join("test.meta");

        let meta = MetaFile {
            version: 2,
            name: "test".to_string(),
            address: "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string(),
            storage: "keychain".to_string(),
            created_at: "2026-03-28T14:30:00Z".to_string(),
        };

        let json = serde_json::to_string_pretty(&meta).unwrap();
        std::fs::write(&meta_path, &json).unwrap();

        let contents = std::fs::read_to_string(&meta_path).unwrap();
        let loaded: MetaFile = serde_json::from_str(&contents).unwrap();

        assert_eq!(loaded.version, 2);
        assert_eq!(loaded.name, "test");
        assert_eq!(loaded.address, "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
        assert_eq!(loaded.storage, "keychain");
    }

    #[test]
    fn meta_file_has_no_secret_fields() {
        let meta = MetaFile {
            version: 2,
            name: "default".to_string(),
            address: "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string(),
            storage: "keychain".to_string(),
            created_at: "2026-03-28T14:30:00Z".to_string(),
        };

        let json = serde_json::to_string(&meta).unwrap();

        assert!(
            !json.contains("private_key"),
            "meta file must not contain 'private_key'"
        );
        assert!(
            !json.contains("password"),
            "meta file must not contain 'password'"
        );
        assert!(
            !json.contains("secret"),
            "meta file must not contain 'secret'"
        );
        assert!(
            !json.contains("ciphertext"),
            "meta file must not contain 'ciphertext'"
        );
    }

    #[test]
    fn meta_file_path_uses_pay_dir() {
        let path = MetaFile::path("default").unwrap();
        let path_str = path.to_string_lossy();
        assert!(path_str.contains(".pay"));
        assert!(path_str.contains("keys"));
        assert!(path_str.ends_with("default.meta"));
    }

    // -- Keychain availability ------------------------------------------------

    #[test]
    fn is_available_does_not_panic() {
        // K5: non-destructive, should never panic
        let _result = is_available();
    }

    // -- Keychain store/load/delete roundtrip ---------------------------------

    #[test]
    #[cfg(feature = "keychain")]
    fn keychain_roundtrip_if_available() {
        if !is_available() {
            eprintln!("skipping keychain_roundtrip_if_available: no keychain available");
            return;
        }

        let test_label = "__pay_test_roundtrip__";
        let test_key: [u8; 32] = [0xab; 32];

        store_key(test_label, &test_key).expect("store_key should succeed");

        // Verify stored as hex string, not raw bytes
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, test_label).unwrap();
        let stored = entry.get_password().expect("should read as string");
        assert!(
            stored.starts_with("0x"),
            "stored value must be 0x-prefixed hex"
        );
        assert_eq!(stored.len(), 66, "stored hex must be 66 chars (0x + 64)");

        let loaded = load_key(test_label).expect("load_key should succeed");
        assert_eq!(*loaded, test_key, "loaded key must match stored key");

        delete_key(test_label).expect("delete_key should succeed");

        let result = load_key(test_label);
        assert!(result.is_err(), "load after delete should fail");
    }

    /// Verify that load_key can read legacy raw-byte entries and auto-migrates them.
    #[test]
    #[cfg(feature = "keychain")]
    fn keychain_legacy_raw_bytes_migration() {
        if !is_available() {
            eprintln!("skipping keychain_legacy_raw_bytes_migration: no keychain available");
            return;
        }

        let test_label = "__pay_test_legacy_migration__";
        let test_key: [u8; 32] = [0xcd; 32];

        // Store as raw bytes (legacy format) using set_secret directly
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, test_label).unwrap();
        entry.set_secret(test_key.as_ref()).unwrap();

        // load_key should still read it and auto-migrate
        let loaded = load_key(test_label).expect("load_key should read legacy raw bytes");
        assert_eq!(*loaded, test_key, "loaded key must match stored key");

        // After migration, entry should now be hex
        let migrated = entry
            .get_password()
            .expect("should read as string after migration");
        assert!(
            migrated.starts_with("0x"),
            "migrated value must be 0x-prefixed hex"
        );
        let expected_hex = format!("0x{}", hex::encode(test_key));
        assert_eq!(migrated, expected_hex, "migrated hex must match");

        delete_key(test_label).expect("cleanup");
    }

    #[test]
    #[cfg(feature = "keychain")]
    fn load_nonexistent_key_fails() {
        if !is_available() {
            eprintln!("skipping load_nonexistent_key_fails: no keychain available");
            return;
        }

        let result = load_key("__pay_nonexistent_test_key__");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("no key"),
            "error should mention missing key: {err_msg}"
        );
    }

    #[test]
    #[cfg(feature = "keychain")]
    fn delete_nonexistent_key_fails() {
        if !is_available() {
            eprintln!("skipping delete_nonexistent_key_fails: no keychain available");
            return;
        }

        let result = delete_key("__pay_nonexistent_test_key__");
        assert!(result.is_err());
    }

    #[test]
    #[cfg(feature = "keychain")]
    fn error_messages_contain_no_key_material() {
        if !is_available() {
            return;
        }

        let result = load_key("__pay_error_msg_test__");
        if let Err(e) = result {
            let msg = e.to_string();
            assert!(
                !msg.contains("ab".repeat(16).as_str()),
                "error must not contain key material"
            );
        }
    }
}
