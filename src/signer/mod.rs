pub mod keyring;
pub mod keystore;
pub mod password;

use anyhow::{bail, Result};
use k256::ecdsa::SigningKey;

/// Resolve the signing key using the unified priority chain.
///
/// 1. `PAYSKILL_SIGNER_KEY` as raw 64-hex-char key
/// 2. OS keychain via `.meta` file (no password needed)
/// 3. `.enc` file + `PAYSKILL_SIGNER_KEY` as password
/// 4. `.enc` file + interactive rpassword prompt (terminal only)
/// 5. Error
pub fn resolve_key() -> Result<SigningKey> {
    let env_val = std::env::var("PAYSKILL_SIGNER_KEY").ok();

    // 1. Raw hex key from env
    if let Some(ref val) = env_val {
        let clean = val.strip_prefix("0x").unwrap_or(val);
        if clean.len() == 64 && clean.chars().all(|c| c.is_ascii_hexdigit()) {
            if let Ok(bytes) = hex::decode(clean) {
                if let Ok(key) = SigningKey::from_slice(&bytes) {
                    return Ok(key);
                }
            }
        }
    }

    // 2. OS keychain via .meta
    if let Ok(true) = keyring::MetaFile::exists("default") {
        if let Ok(meta) = keyring::MetaFile::load("default") {
            if meta.storage == "keychain" {
                match keyring::load_key("default") {
                    Ok(raw) => {
                        return SigningKey::from_slice(&*raw)
                            .map_err(|_| anyhow::anyhow!("keychain key is not valid"));
                    }
                    Err(e) => {
                        eprintln!("Warning: keychain entry not found, trying .enc: {e}");
                    }
                }
            }
        }
    }

    // 3. .enc file + password from env
    let ks = keystore::Keystore::open()?;
    if ks.exists("default") {
        // Detect legacy format (pre-scrypt)
        if let Ok(kf) = ks.load("default") {
            // New format has version field
            if let Some(ref val) = env_val {
                let clean = val.strip_prefix("0x").unwrap_or(val);
                // Not a raw hex key, use as password
                if !(clean.len() == 64 && clean.chars().all(|c| c.is_ascii_hexdigit())) {
                    return keystore::decrypt(&kf, val);
                }
            }

            // 4. Interactive prompt
            if let Ok(pw) = password::acquire_for_decrypt() {
                return keystore::decrypt(&kf, &pw);
            }
        } else {
            // Parse failed — likely legacy format
            bail!(
                "Legacy key file detected at ~/.pay/keys/default.enc. \
                 Run `pay init` to create a new wallet, or back up and delete the file."
            );
        }
    }

    // 5. Error
    bail!("No signing key configured. Run `pay init` or set PAYSKILL_SIGNER_KEY.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        key: &'static str,
        old: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, val: &str) -> Self {
            let old = std::env::var(key).ok();
            std::env::set_var(key, val);
            Self { key, old }
        }

        fn remove(key: &'static str) -> Self {
            let old = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, old }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.old {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn resolve_raw_hex_key_from_env() {
        let _lock = ENV_MUTEX.lock();
        let _guard = EnvGuard::set(
            "PAYSKILL_SIGNER_KEY",
            "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        );
        let key = resolve_key().unwrap();
        let addr = crate::auth::derive_address(&key);
        assert_eq!(addr, "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");
    }

    #[test]
    fn resolve_raw_hex_key_with_0x_prefix() {
        let _lock = ENV_MUTEX.lock();
        let _guard = EnvGuard::set(
            "PAYSKILL_SIGNER_KEY",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        );
        let key = resolve_key().unwrap();
        let addr = crate::auth::derive_address(&key);
        assert_eq!(addr, "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");
    }

    #[test]
    fn resolve_password_decrypts_enc_file() {
        let _lock = ENV_MUTEX.lock();

        // Create a temp keystore with a known key
        let dir = tempfile::TempDir::new().unwrap();
        let _keys_guard = EnvGuard::set("PAY_KEYS_DIR", dir.path().to_str().unwrap());

        let ks = keystore::Keystore::open_in(dir.path().to_path_buf());
        let key_hex = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let password = "test-password";
        ks.import("default", key_hex, password).unwrap();

        let _env_guard = EnvGuard::set("PAYSKILL_SIGNER_KEY", password);

        let key = resolve_key().unwrap();
        let addr = crate::auth::derive_address(&key);
        assert_eq!(addr, "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");
    }

    #[test]
    fn resolve_fails_with_no_key() {
        let _lock = ENV_MUTEX.lock();
        let _env_guard = EnvGuard::remove("PAYSKILL_SIGNER_KEY");

        // Point to empty temp dir so no .meta or .enc found
        let dir = tempfile::TempDir::new().unwrap();
        let _keys_guard = EnvGuard::set("PAY_KEYS_DIR", dir.path().to_str().unwrap());

        let result = resolve_key();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No signing key"));
    }
}
