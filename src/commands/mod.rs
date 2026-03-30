pub mod direct;
pub mod init;
pub mod request;
pub mod sign;
pub mod status;
pub mod tab;
pub mod webhook;

use anyhow::{bail, Result};

use crate::config::Config;

/// Shared context passed to all command handlers.
pub struct Context {
    pub json: bool,
    pub config: Config,
    pub http: reqwest::Client,
}

impl Context {
    pub fn new(json: bool, config: Config) -> Self {
        Self {
            json,
            config,
            http: reqwest::Client::new(),
        }
    }

    /// Get the effective API URL.
    pub fn api_url(&self) -> &str {
        self.config.api_url()
    }

    /// Make a GET request to the API.
    pub async fn get(&self, path: &str) -> Result<serde_json::Value> {
        let url = format!("{}{}", self.api_url(), path);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("API error ({status}): {body}");
        }
        Ok(resp.json().await?)
    }

    /// Make a POST request to the API.
    pub async fn post(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
        let url = format!("{}{}", self.api_url(), path);
        let resp = self.http.post(&url).json(body).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("API error ({status}): {body}");
        }
        Ok(resp.json().await?)
    }

    /// Make a DELETE request to the API.
    pub async fn del(&self, path: &str) -> Result<()> {
        let url = format!("{}{}", self.api_url(), path);
        let resp = self.http.delete(&url).send().await?;
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
    // Convert to micro-units, rounding to nearest integer
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
