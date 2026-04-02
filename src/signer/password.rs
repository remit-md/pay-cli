//! Password acquisition for encrypted key files.
//!
//! Priority:
//! 1. `PAYSKILL_SIGNER_KEY` env var (if not a raw hex key)
//! 2. Interactive terminal prompt via rpassword
//! 3. Error with actionable message

use anyhow::{bail, Result};
use std::io::IsTerminal;

const ENV_VAR: &str = "PAYSKILL_SIGNER_KEY";

/// Check if a string looks like a raw hex private key (64 hex chars, optional 0x prefix).
fn is_raw_hex_key(val: &str) -> bool {
    let clean = val.strip_prefix("0x").unwrap_or(val);
    clean.len() == 64 && clean.chars().all(|c| c.is_ascii_hexdigit())
}

/// Try to extract a password from the env var.
///
/// Returns `Some(password)` if `PAYSKILL_SIGNER_KEY` is set and is NOT a raw hex key.
/// Returns `None` if unset, empty, or looks like a raw key.
pub fn from_env() -> Option<String> {
    match std::env::var(ENV_VAR) {
        Ok(val) if !val.is_empty() && !is_raw_hex_key(&val) => Some(val),
        _ => None,
    }
}

/// Acquire a password for encrypting a new key (with confirmation).
///
/// Priority: env var -> interactive prompt (twice) -> error.
pub fn acquire_for_encrypt() -> Result<String> {
    // 1. Env var
    if let Some(pw) = from_env() {
        return Ok(pw);
    }

    // 2. Interactive prompt
    if !std::io::stderr().is_terminal() {
        bail!(
            "No password available. Set {} (non-hex value) or run interactively.",
            ENV_VAR
        );
    }

    loop {
        let pw = rpassword::prompt_password("Choose a password: ")
            .map_err(|e| anyhow::anyhow!("failed to read password: {e}"))?;
        if pw.is_empty() {
            eprintln!("Password cannot be empty.");
            continue;
        }

        let confirm = rpassword::prompt_password("Confirm password: ")
            .map_err(|e| anyhow::anyhow!("failed to read password: {e}"))?;
        if pw != confirm {
            eprintln!("Passwords do not match. Try again.");
            continue;
        }

        return Ok(pw);
    }
}

/// Acquire a password for decrypting an existing key (no confirmation).
///
/// Priority: env var -> interactive prompt (once) -> error.
pub fn acquire_for_decrypt() -> Result<String> {
    // 1. Env var
    if let Some(pw) = from_env() {
        return Ok(pw);
    }

    // 2. Interactive prompt
    if !std::io::stderr().is_terminal() {
        bail!(
            "No password available. Set {} (non-hex value) or run interactively.",
            ENV_VAR
        );
    }

    let pw = rpassword::prompt_password("Enter password: ")
        .map_err(|e| anyhow::anyhow!("failed to read password: {e}"))?;
    Ok(pw)
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize env-mutating tests to avoid races.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn is_raw_hex_key_detects_64_hex() {
        assert!(is_raw_hex_key(
            "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
        ));
        assert!(is_raw_hex_key(
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
        ));
    }

    #[test]
    fn is_raw_hex_key_rejects_passwords() {
        assert!(!is_raw_hex_key("my-password-123"));
        assert!(!is_raw_hex_key("short-hex-abcdef"));
        assert!(!is_raw_hex_key("")); // empty
        assert!(!is_raw_hex_key("zzzz0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")); // non-hex chars
    }

    #[test]
    fn from_env_returns_none_for_hex_key() {
        let _lock = ENV_MUTEX.lock();
        let _guard = EnvGuard::new(
            ENV_VAR,
            "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        );
        assert!(from_env().is_none());
    }

    #[test]
    fn from_env_returns_password_for_non_hex() {
        let _lock = ENV_MUTEX.lock();
        let _guard = EnvGuard::new(ENV_VAR, "my-password");
        assert_eq!(from_env(), Some("my-password".to_string()));
    }

    #[test]
    fn from_env_returns_none_when_unset() {
        let _lock = ENV_MUTEX.lock();
        let _guard = EnvGuard::remove(ENV_VAR);
        assert!(from_env().is_none());
    }

    /// RAII guard for env var mutation in tests.
    struct EnvGuard {
        key: &'static str,
        old: Option<String>,
    }

    impl EnvGuard {
        fn new(key: &'static str, val: &str) -> Self {
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
}
