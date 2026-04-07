pub mod direct;
pub mod discover;
pub mod init;
pub mod key;
pub mod network;
pub mod ows_cmd;
pub mod request;
pub mod sign;
pub mod signer_cmd;
pub mod status;
pub mod tab;
pub mod webhook;

use anyhow::{bail, Result};
use k256::ecdsa::SigningKey;

use crate::auth;
use crate::config::Config;

/// Shared context passed to all command handlers.
pub struct Context {
    pub json: bool,
    pub config: Config,
    pub http: reqwest::Client,
    signing_key: Option<SigningKey>,
}

impl Context {
    pub fn new(json: bool, config: Config) -> Self {
        Self {
            json,
            config,
            http: reqwest::Client::new(),
            signing_key: None,
        }
    }

    /// Load the signing key (lazy, cached).
    pub fn load_key(&mut self) -> Result<&SigningKey> {
        if self.signing_key.is_none() {
            self.signing_key = Some(crate::signer::resolve_key()?);
        }
        Ok(self.signing_key.as_ref().expect("key just loaded"))
    }

    /// Get the wallet address (requires loaded key).
    pub fn address(&mut self) -> Result<String> {
        let key = self.load_key()?;
        Ok(auth::derive_address(key))
    }

    /// Get the effective API URL.
    pub fn api_url(&self) -> &str {
        self.config.api_url()
    }

    /// Build auth headers for a request.
    fn auth_headers(&mut self, method: &str, path: &str) -> Result<Vec<(String, String)>> {
        let router = self.config.router_address().to_string();
        if router.is_empty() {
            bail!("router_address not set in config. Run `pay init` or `pay network testnet`.");
        }
        let chain_id = self.config.chain_id();
        let key = self.load_key()?;
        auth::build_auth_headers(key, method, path, chain_id, &router)
    }

    /// Resolve the full path for signing: /api/v1{path}.
    /// The EIP-712 hash must use the exact path the server sees (no query string).
    fn full_path(&self, path: &str) -> String {
        // Strip query string — server uses uri.path() which excludes ?params
        let path_only = path.split('?').next().unwrap_or(path);

        // Extract the path portion from the API URL (e.g., "/api/v1" from "http://host/api/v1")
        let api_url = self.api_url();
        // Find the path after the host: skip "https://host" or "http://host:port"
        if let Some(idx) = api_url.find("://") {
            let after_scheme = &api_url[idx + 3..];
            if let Some(slash_idx) = after_scheme.find('/') {
                let base_path = after_scheme[slash_idx..].trim_end_matches('/');
                return format!("{base_path}{path_only}");
            }
        }
        format!("/api/v1{path_only}")
    }

    /// Make an authenticated GET request to the API.
    /// On 401, tries to refresh config from server and retry once.
    pub async fn get(&mut self, path: &str) -> Result<serde_json::Value> {
        let url = format!("{}{}", self.api_url(), path);
        let sign_path = self.full_path(path);
        let headers = self.auth_headers("GET", &sign_path)?;
        let mut req = self.http.get(&url);
        for (k, v) in &headers {
            req = req.header(k, v);
        }
        let resp = req.send().await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            if self.try_refresh_config().await {
                // Retry with refreshed config
                let sign_path = self.full_path(path);
                let headers = self.auth_headers("GET", &sign_path)?;
                let mut req = self.http.get(&url);
                for (k, v) in &headers {
                    req = req.header(k, v);
                }
                let resp = req.send().await?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    bail!("API error ({status}): {body}");
                }
                return Ok(resp.json().await?);
            }
            let body = resp.text().await.unwrap_or_default();
            bail!("API error (401 Unauthorized): {body}");
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("API error ({status}): {body}");
        }
        Ok(resp.json().await?)
    }

    /// Make an authenticated POST request to the API.
    /// On 401, tries to refresh config from server and retry once.
    pub async fn post(
        &mut self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let url = format!("{}{}", self.api_url(), path);
        let sign_path = self.full_path(path);
        let headers = self.auth_headers("POST", &sign_path)?;
        let mut req = self.http.post(&url).json(body);
        for (k, v) in &headers {
            req = req.header(k, v);
        }
        let resp = req.send().await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            if self.try_refresh_config().await {
                let sign_path = self.full_path(path);
                let headers = self.auth_headers("POST", &sign_path)?;
                let mut req = self.http.post(&url).json(body);
                for (k, v) in &headers {
                    req = req.header(k, v);
                }
                let resp = req.send().await?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    bail!("API error ({status}): {body}");
                }
                return Ok(resp.json().await?);
            }
            let body = resp.text().await.unwrap_or_default();
            bail!("API error (401 Unauthorized): {body}");
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("API error ({status}): {body}");
        }
        Ok(resp.json().await?)
    }

    /// Make an authenticated DELETE request to the API.
    /// On 401, tries to refresh config from server and retry once.
    pub async fn del(&mut self, path: &str) -> Result<()> {
        let url = format!("{}{}", self.api_url(), path);
        let sign_path = self.full_path(path);
        let headers = self.auth_headers("DELETE", &sign_path)?;
        let mut req = self.http.delete(&url);
        for (k, v) in &headers {
            req = req.header(k, v);
        }
        let resp = req.send().await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            if self.try_refresh_config().await {
                let sign_path = self.full_path(path);
                let headers = self.auth_headers("DELETE", &sign_path)?;
                let mut req = self.http.delete(&url);
                for (k, v) in &headers {
                    req = req.header(k, v);
                }
                let resp = req.send().await?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    bail!("API error ({status}): {body}");
                }
                return Ok(());
            }
            let body = resp.text().await.unwrap_or_default();
            bail!("API error (401 Unauthorized): {body}");
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("API error ({status}): {body}");
        }
        Ok(())
    }

    /// Try to refresh config from server (fetch fresh router address).
    /// Returns true if config was updated and a retry is worthwhile.
    async fn try_refresh_config(&mut self) -> bool {
        if let Ok(()) = self.config.bootstrap_from_server().await {
            if let Ok(()) = self.config.save() {
                return true;
            }
        }
        false
    }
}

/// Require that `pay init` has been run.
pub fn require_init() -> Result<()> {
    // Check for any of: env var, .meta (keychain), .enc (encrypted file), config
    let has_env = std::env::var("PAYSKILL_SIGNER_KEY").is_ok();
    let has_meta = crate::signer::keyring::MetaFile::exists("default").unwrap_or(false);
    let has_enc = crate::signer::keystore::Keystore::open()
        .map(|ks| ks.exists("default"))
        .unwrap_or(false);
    let has_config = Config::is_initialized();

    if !has_env && !has_meta && !has_enc && !has_config {
        bail!("Wallet not initialized. Run `pay init` first.");
    }
    Ok(())
}

/// Validate an Ethereum address (0x + 40 hex chars).
pub fn validate_address(addr: &str) -> Result<()> {
    if addr.len() != 42
        || !addr.starts_with("0x")
        || !addr[2..].chars().all(|c| c.is_ascii_hexdigit())
    {
        bail!("Invalid address: {addr}");
    }
    Ok(())
}

/// Parse a dollar amount string to USDC micro-units (6 decimals).
/// "1.50" → 1_500_000, "5" → 5_000_000
pub fn parse_amount(s: &str) -> Result<u64> {
    let amount: f64 = s
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid amount: {s}"))?;
    if amount <= 0.0 {
        bail!("Amount must be positive");
    }
    let micro = (amount * 1_000_000.0).round() as u64;
    Ok(micro)
}

/// Format USDC micro-units as a dollar string: 1_500_000 → "$1.50"
pub fn format_amount(micro: u64) -> String {
    let dollars = micro as f64 / 1_000_000.0;
    format!("${dollars:.2}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_address_valid() {
        let addr = format!("0x{}", "a1".repeat(20));
        assert!(validate_address(&addr).is_ok());
    }

    #[test]
    fn test_validate_address_too_short() {
        assert!(validate_address("0x1234").is_err());
    }

    #[test]
    fn test_validate_address_no_prefix() {
        let addr = "a1".repeat(20);
        assert!(validate_address(&addr).is_err());
    }

    #[test]
    fn test_validate_address_non_hex() {
        let addr = format!("0x{}", "zz".repeat(20));
        assert!(validate_address(&addr).is_err());
    }

    #[test]
    fn test_parse_amount_dollars() {
        assert_eq!(parse_amount("1.50").unwrap(), 1_500_000);
    }

    #[test]
    fn test_parse_amount_whole() {
        assert_eq!(parse_amount("5").unwrap(), 5_000_000);
    }

    #[test]
    fn test_parse_amount_zero() {
        assert!(parse_amount("0").is_err());
    }

    #[test]
    fn test_parse_amount_negative() {
        assert!(parse_amount("-1").is_err());
    }

    #[test]
    fn test_parse_amount_invalid() {
        assert!(parse_amount("abc").is_err());
    }

    #[test]
    fn test_format_amount() {
        assert_eq!(format_amount(1_500_000), "$1.50");
        assert_eq!(format_amount(5_000_000), "$5.00");
        assert_eq!(format_amount(100_000), "$0.10");
    }

    #[test]
    fn test_full_path() {
        let ctx = Context::new(
            false,
            crate::config::Config {
                api_url: Some("http://localhost:3001/api/v1".to_string()),
                ..Default::default()
            },
        );
        assert_eq!(ctx.full_path("/direct"), "/api/v1/direct");
        assert_eq!(ctx.full_path("/tabs"), "/api/v1/tabs");
        assert_eq!(ctx.full_path("/mint"), "/api/v1/mint");
        // Query strings must be stripped (server signs path only)
        assert_eq!(ctx.full_path("/status?wallet=0xabc"), "/api/v1/status");
    }

    #[test]
    fn test_full_path_https() {
        let ctx = Context::new(
            false,
            crate::config::Config {
                api_url: Some("https://pay-skill.com/api/v1".to_string()),
                ..Default::default()
            },
        );
        assert_eq!(ctx.full_path("/status"), "/api/v1/status");
    }

    #[test]
    fn test_full_path_default() {
        let ctx = Context::new(false, crate::config::Config::default());
        // Default is now mainnet (pay-skill.com/api/v1)
        assert_eq!(ctx.full_path("/direct"), "/api/v1/direct");
    }
}
