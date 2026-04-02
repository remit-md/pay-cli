//! OWS (Open Wallet Standard) helpers — subprocess + vault file reads.
//!
//! Mutations run `ows` CLI subprocess. Queries read JSON from `~/.ows/`.
//! Nothing compiled in. If OWS isn't installed, we fail loud.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

// ── Chain helpers ────────────────────────────────────────────────────

/// Map a chain name to its CAIP-2 identifier.
pub fn chain_to_caip2(chain: &str) -> Result<String> {
    match chain {
        "base" => Ok("eip155:8453".to_string()),
        "base-sepolia" => Ok("eip155:84532".to_string()),
        _ => bail!("unknown chain: {chain}. Supported: base, base-sepolia"),
    }
}

/// Detect chain from PAYSKILL_CHAIN env var, defaulting to "base".
pub fn detect_chain() -> String {
    std::env::var("PAYSKILL_CHAIN").unwrap_or_else(|_| "base".to_string())
}

/// Default wallet name: "pay-{hostname}".
pub fn default_wallet_name() -> String {
    let host = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    format!("pay-{host}")
}

// ── Vault paths ─────────────────────────────────────────────────────

fn vault_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ows")
}

/// Display the vault path.
pub fn vault_path_display() -> String {
    vault_dir().display().to_string()
}

// ── OWS CLI subprocess ──────────────────────────────────────────────

/// Run `ows` CLI, returning stdout. Fails loud if not installed.
fn run_ows(args: &[&str]) -> Result<String> {
    let output = Command::new("ows")
        .args(args)
        .output()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => anyhow::anyhow!(
                "ows CLI not found. Install with:\n  \
                 npm install -g @open-wallet-standard/core\n  \
                 Or build from source: https://github.com/open-wallet-standard/core"
            ),
            _ => anyhow::anyhow!("failed to run ows: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("ows {} failed: {}", args.join(" "), stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Check if `ows` CLI is available.
pub fn is_ows_available() -> bool {
    Command::new("ows")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Install OWS CLI via npm.
pub fn install_ows_via_npm() -> Result<()> {
    let output = Command::new("npm")
        .args(["install", "-g", "@open-wallet-standard/core"])
        .output()
        .map_err(|e| anyhow::anyhow!("npm not found — install Node.js first: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("npm install failed: {stderr}");
    }

    Ok(())
}

// ── Vault file reads ────────────────────────────────────────────────

/// Read all wallet JSON files from `~/.ows/wallets/`.
pub fn list_wallets() -> Result<Vec<Value>> {
    let dir = vault_dir().join("wallets");
    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut wallets = Vec::new();
    for entry in std::fs::read_dir(&dir).context("failed to read ~/.ows/wallets/")? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let wallet: Value = serde_json::from_str(&content)
                .with_context(|| format!("failed to parse {}", path.display()))?;
            wallets.push(wallet);
        }
    }

    Ok(wallets)
}

/// Get a wallet by name or ID from vault files.
pub fn get_wallet(name_or_id: &str) -> Result<Value> {
    let wallets = list_wallets()?;
    for w in &wallets {
        let wname = w["name"].as_str().unwrap_or("");
        let wid = w["id"].as_str().unwrap_or("");
        if wname == name_or_id || wid == name_or_id {
            return Ok(w.clone());
        }
    }
    bail!("wallet not found: {name_or_id}")
}

/// Extract the EVM address from a wallet JSON value.
pub fn wallet_evm_address(wallet: &Value) -> Option<String> {
    let accounts = wallet["accounts"].as_array()?;
    for acct in accounts {
        let chain = acct["chain_id"]
            .as_str()
            .or_else(|| acct["chainId"].as_str())
            .unwrap_or("");
        if chain.starts_with("eip155:") {
            return acct["address"].as_str().map(|s| s.to_string());
        }
    }
    None
}

// ── Create wallet ───────────────────────────────────────────────────

/// Create a wallet via `ows wallet create`, then read it from vault.
pub fn create_wallet(name: &str) -> Result<Value> {
    let stdout = run_ows(&["wallet", "create", "--name", name])?;

    // Parse wallet ID from "Wallet created: {uuid}"
    let id = stdout
        .lines()
        .find(|l| l.starts_with("Wallet created:"))
        .and_then(|l| l.strip_prefix("Wallet created:"))
        .map(|s| s.trim().to_string())
        .ok_or_else(|| anyhow::anyhow!("failed to parse wallet ID from ows output: {stdout}"))?;

    // Read the wallet JSON from vault
    let path = vault_dir().join("wallets").join(format!("{id}.json"));
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("wallet created but file not found: {}", path.display()))?;
    serde_json::from_str(&content).context("failed to parse wallet JSON")
}

// ── Create policy ───────────────────────────────────────────────────

/// Create a chain-lock policy via `ows policy create --file`.
pub fn create_chain_policy(chain: &str) -> Result<Value> {
    let caip2 = chain_to_caip2(chain)?;
    let id = format!("pay-{chain}");
    let now = chrono::Utc::now().to_rfc3339();

    let policy = serde_json::json!({
        "id": id,
        "name": format!("Pay {chain} chain lock"),
        "version": 1,
        "created_at": now,
        "rules": [{
            "type": "allowed_chains",
            "chain_ids": [caip2]
        }],
        "action": "deny"
    });

    let tmp = tempfile::NamedTempFile::with_suffix(".json")
        .context("failed to create temp file")?;
    std::fs::write(tmp.path(), serde_json::to_string_pretty(&policy)?)
        .context("failed to write policy file")?;

    run_ows(&["policy", "create", "--file", &tmp.path().to_string_lossy()])?;

    Ok(policy)
}

/// Create a spending policy with limits.
pub fn create_spending_policy(
    chain: &str,
    max_tx_usdc: Option<f64>,
    daily_limit_usdc: Option<f64>,
) -> Result<Value> {
    let caip2 = chain_to_caip2(chain)?;
    let id = format!("pay-{chain}-limits");
    let now = chrono::Utc::now().to_rfc3339();

    let mut config = serde_json::Map::new();
    config.insert("chain_ids".to_string(), serde_json::json!([&caip2]));
    if let Some(max_tx) = max_tx_usdc {
        config.insert("max_tx_usdc".to_string(), serde_json::json!(max_tx));
    }
    if let Some(daily) = daily_limit_usdc {
        config.insert(
            "daily_limit_usdc".to_string(),
            serde_json::json!(daily),
        );
    }

    let policy = serde_json::json!({
        "id": id,
        "name": format!("Pay {chain} spending policy"),
        "version": 1,
        "created_at": now,
        "rules": [{
            "type": "allowed_chains",
            "chain_ids": [caip2]
        }],
        "executable": "npx @pay-skill/ows-policy",
        "config": config,
        "action": "deny"
    });

    let tmp = tempfile::NamedTempFile::with_suffix(".json")
        .context("failed to create temp file")?;
    std::fs::write(tmp.path(), serde_json::to_string_pretty(&policy)?)
        .context("failed to write policy file")?;

    run_ows(&["policy", "create", "--file", &tmp.path().to_string_lossy()])?;

    Ok(policy)
}

// ── Create API key ──────────────────────────────────────────────────

/// Create an API key. Returns the full stdout (contains the token shown once).
pub fn create_api_key(wallet_id: &str, policy_id: &str) -> Result<String> {
    run_ows(&[
        "key",
        "create",
        "--name",
        "pay-agent",
        "--wallet",
        wallet_id,
        "--policy",
        policy_id,
    ])
}

/// Parse the API token from `ows key create` stdout.
/// Looks for "ows_key_..." in the output.
pub fn parse_api_token(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .find_map(|line| {
            line.split_whitespace()
                .find(|word| word.starts_with("ows_key_"))
                .map(|s| s.to_string())
        })
}

// ── MCP config ──────────────────────────────────────────────────────

/// Generate MCP config JSON for the user.
pub fn mcp_config_json(wallet_name: &str, chain: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": {
            "pay": {
                "command": "npx",
                "args": ["@pay-skill/mcp-server"],
                "env": {
                    "OWS_WALLET_ID": wallet_name,
                    "OWS_API_KEY": "$OWS_API_KEY",
                    "PAYSKILL_CHAIN": chain,
                }
            }
        }
    }))
    .expect("JSON serialization cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_to_caip2_base() {
        assert_eq!(chain_to_caip2("base").unwrap(), "eip155:8453");
    }

    #[test]
    fn test_chain_to_caip2_base_sepolia() {
        assert_eq!(chain_to_caip2("base-sepolia").unwrap(), "eip155:84532");
    }

    #[test]
    fn test_chain_to_caip2_unknown() {
        assert!(chain_to_caip2("ethereum").is_err());
    }

    #[test]
    fn test_default_wallet_name() {
        let name = default_wallet_name();
        assert!(
            name.starts_with("pay-"),
            "should start with pay-, got: {name}"
        );
    }

    #[test]
    fn test_mcp_config_json() {
        let config = mcp_config_json("test-wallet", "base");
        assert!(config.contains("test-wallet"));
        assert!(config.contains("@pay-skill/mcp-server"));
    }

    #[test]
    fn test_wallet_evm_address_found() {
        let wallet = serde_json::json!({
            "accounts": [{
                "chain_id": "eip155:8453",
                "address": "0xdeadbeef"
            }]
        });
        assert_eq!(wallet_evm_address(&wallet), Some("0xdeadbeef".to_string()));
    }

    #[test]
    fn test_wallet_evm_address_not_found() {
        let wallet = serde_json::json!({
            "accounts": [{
                "chain_id": "solana:mainnet",
                "address": "SolAddr123"
            }]
        });
        assert_eq!(wallet_evm_address(&wallet), None);
    }

    #[test]
    fn test_wallet_evm_address_empty() {
        let wallet = serde_json::json!({ "accounts": [] });
        assert_eq!(wallet_evm_address(&wallet), None);
    }

    #[test]
    fn test_parse_api_token() {
        let stdout = "API key created: abc-123\nName: pay-agent\n\n\
                       TOKEN (shown once — save it now):\n\
                       ows_key_8aaaadd6d21d41901dd5aff3969f03de70162057";
        assert_eq!(
            parse_api_token(stdout),
            Some("ows_key_8aaaadd6d21d41901dd5aff3969f03de70162057".to_string())
        );
    }

    #[test]
    fn test_parse_api_token_missing() {
        assert_eq!(parse_api_token("no token here"), None);
    }

    #[test]
    fn test_vault_path_display() {
        let path = vault_path_display();
        assert!(path.contains(".ows"), "should contain .ows: {path}");
    }

    #[test]
    fn test_ows_availability_does_not_panic() {
        let _ = is_ows_available();
    }
}
