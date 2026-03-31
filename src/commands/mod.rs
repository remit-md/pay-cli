pub mod direct;
pub mod init;
pub mod request;
pub mod sign;
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
            self.signing_key = Some(crate::keystore::resolve_key()?);
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
        let chain_id = self.config.chain_id();
        let router = self.config.router_address().to_string();
        if router.is_empty() {
            bail!(
                "router_address not set in config. Set ROUTER_ADDRESS or update ~/.pay/config.toml"
            );
        }
        let key = self.load_key()?;
        auth::build_auth_headers(key, method, path, chain_id, &router)
    }

    /// Make an authenticated GET request to the API.
    pub async fn get(&mut self, path: &str) -> Result<serde_json::Value> {
        let url = format!("{}{}", self.api_url(), path);
        let headers = self.auth_headers("GET", path)?;
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
        Ok(resp.json().await?)
    }

    /// Make an authenticated POST request to the API.
    pub async fn post(
        &mut self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let url = format!("{}{}", self.api_url(), path);
        let headers = self.auth_headers("POST", path)?;
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
        Ok(resp.json().await?)
    }

    /// Make an authenticated DELETE request to the API.
    pub async fn del(&mut self, path: &str) -> Result<()> {
        let url = format!("{}{}", self.api_url(), path);
        let headers = self.auth_headers("DELETE", path)?;
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
        Ok(())
    }
}

/// Require that `pay init` has been run.
pub fn require_init() -> Result<()> {
    if !Config::is_initialized() {
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
}
