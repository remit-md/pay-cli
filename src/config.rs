use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const CONFIG_DIR: &str = ".pay";
const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    pub api_url: Option<String>,
    pub testnet: Option<bool>,
    pub chain_id: Option<u64>,
    pub router_address: Option<String>,
}

impl Config {
    /// Get chain_id, defaulting to Base mainnet.
    pub fn chain_id(&self) -> u64 {
        self.chain_id.unwrap_or(8453)
    }

    /// Get router address. Returns empty string if not set.
    pub fn router_address(&self) -> &str {
        self.router_address.as_deref().unwrap_or("")
    }

    /// Load config from ~/.pay/config.toml. Returns default if file doesn't exist.
    pub fn load() -> Result<Self> {
        let path = config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config: {}", path.display()))?;
        let config: Config =
            toml::from_str(&content).with_context(|| "Failed to parse config.toml")?;
        Ok(config)
    }

    /// Save config to ~/.pay/config.toml.
    pub fn save(&self) -> Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config dir: {}", parent.display()))?;
        }
        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;
        fs::write(&path, content)
            .with_context(|| format!("Failed to write config: {}", path.display()))?;
        Ok(())
    }

    /// Get the effective API URL.
    pub fn api_url(&self) -> &str {
        self.api_url
            .as_deref()
            .unwrap_or("https://pay-skill.com/api/v1")
    }

    pub fn is_testnet(&self) -> bool {
        self.testnet.unwrap_or(false)
            || self.chain_id == Some(84532)
            || self.api_url.as_deref().is_some_and(|u| u.contains("testnet"))
    }

    /// Check if config file exists (i.e., `pay init` has been run).
    pub fn is_initialized() -> bool {
        config_path().exists()
    }
}

fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(CONFIG_DIR)
        .join(CONFIG_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_api_url() {
        let config = Config::default();
        assert_eq!(config.api_url(), "https://pay-skill.com/api/v1");
    }

    #[test]
    fn test_custom_api_url() {
        let config = Config {
            api_url: Some("http://localhost:3000".to_string()),
            ..Config::default()
        };
        assert_eq!(config.api_url(), "http://localhost:3000");
    }
}
