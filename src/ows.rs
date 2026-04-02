//! OWS (Open Wallet Standard) helpers — subprocess-based, no compiled ows crates.

use anyhow::{bail, Result};
use serde_json::Value;
use std::process::Command;

/// Map a chain name to a CAIP-2 identifier.
pub fn chain_to_caip2(chain: &str) -> Result<String> {
    match chain {
        "base" => Ok("eip155:8453".to_string()),
        "base-sepolia" => Ok("eip155:84532".to_string()),
        _ => bail!("unknown chain: {chain}. Supported: base, base-sepolia"),
    }
}

/// Map a chain name to a numeric chain ID.
pub fn chain_to_id(chain: &str) -> Result<u64> {
    match chain {
        "base" => Ok(8453),
        "base-sepolia" => Ok(84532),
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

/// Run the `ows` CLI with given args, returning stdout.
/// Fails loud with install instructions if `ows` is not found.
pub fn run_ows(args: &[&str]) -> Result<String> {
    let output = Command::new("ows")
        .args(args)
        .output()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => anyhow::anyhow!(
                "ows CLI not found. Install with: npm install -g @open-wallet-standard/cli\n\
                 Or run: pay ows init --install"
            ),
            _ => anyhow::anyhow!("failed to run ows: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("ows command failed: {stderr}");
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Run the `ows` CLI and parse stdout as JSON.
pub fn run_ows_json(args: &[&str]) -> Result<Value> {
    let raw = run_ows(args)?;
    serde_json::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("failed to parse ows JSON output: {e}\nRaw: {raw}"))
}

/// Check if the `ows` CLI is available.
pub fn is_ows_available() -> bool {
    Command::new("ows")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Install OWS CLI via npm.
pub fn install_ows_via_npm() -> Result<()> {
    let output = Command::new("npm")
        .args(["install", "-g", "@open-wallet-standard/cli"])
        .output()
        .map_err(|e| anyhow::anyhow!("npm not found — install Node.js first: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("npm install failed: {stderr}");
    }

    Ok(())
}

/// Create an OWS wallet with the given name.
pub fn create_wallet(name: &str) -> Result<Value> {
    run_ows_json(&["wallet", "create", "--name", name, "--json"])
}

/// Get a wallet by name or ID (lists all, then filters).
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

/// List all OWS wallets.
pub fn list_wallets() -> Result<Vec<Value>> {
    let val = run_ows_json(&["wallet", "list", "--json"])?;
    match val {
        Value::Array(arr) => Ok(arr),
        _ => Ok(vec![val]),
    }
}

/// Extract the EVM address from a wallet JSON value.
/// Checks both `chainId` and `chain_id` keys in accounts.
pub fn wallet_evm_address(wallet: &Value) -> Option<String> {
    let accounts = wallet["accounts"].as_array()?;
    for acct in accounts {
        let chain_key = acct
            .get("chainId")
            .or_else(|| acct.get("chain_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if chain_key.starts_with("eip155:") || chain_key.starts_with("evm:") {
            return acct["address"].as_str().map(|s| s.to_string());
        }
    }
    None
}

/// Create a chain policy for the given chain.
pub fn create_chain_policy(chain: &str) -> Result<Value> {
    let caip2 = chain_to_caip2(chain)?;
    run_ows_json(&["policy", "create", "--chain", &caip2, "--json"])
}

/// Create a spending policy with max-per-tx and daily limits.
pub fn create_spending_policy(chain: &str, max_tx: &str, daily: &str) -> Result<Value> {
    let caip2 = chain_to_caip2(chain)?;
    run_ows_json(&[
        "policy",
        "create",
        "--chain",
        &caip2,
        "--max-per-tx",
        max_tx,
        "--daily-limit",
        daily,
        "--executable",
        "@pay-skill/ows-policy",
        "--json",
    ])
}

/// Create an API key for a wallet+policy pair.
pub fn create_api_key(wallet_id: &str, policy_id: &str) -> Result<Value> {
    run_ows_json(&[
        "key", "create", "--wallet", wallet_id, "--policy", policy_id, "--json",
    ])
}

/// Generate MCP config JSON for a wallet.
pub fn mcp_config_json(wallet_name: &str, chain: &str) -> String {
    let caip2 = chain_to_caip2(chain).unwrap_or_else(|_| "eip155:8453".to_string());
    serde_json::json!({
        "mcpServers": {
            "pay": {
                "command": "npx",
                "args": ["-y", "@pay-skill/mcp-server"],
                "env": {
                    "OWS_WALLET_NAME": wallet_name,
                    "OWS_CHAIN": caip2,
                    "PAYSKILL_SIGNER": "ows"
                }
            }
        }
    })
    .to_string()
}

/// Display the vault path (for informational output).
pub fn vault_path_display() -> String {
    dirs::home_dir()
        .map(|h| h.join(".ows").join("vault").display().to_string())
        .unwrap_or_else(|| "~/.ows/vault".to_string())
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
    fn test_chain_to_id_base() {
        assert_eq!(chain_to_id("base").unwrap(), 8453);
    }

    #[test]
    fn test_chain_to_id_base_sepolia() {
        assert_eq!(chain_to_id("base-sepolia").unwrap(), 84532);
    }

    #[test]
    fn test_chain_to_id_unknown() {
        assert!(chain_to_id("polygon").is_err());
    }

    #[test]
    fn test_default_wallet_name_starts_with_pay() {
        let name = default_wallet_name();
        assert!(
            name.starts_with("pay-"),
            "wallet name should start with 'pay-', got: {name}"
        );
    }

    #[test]
    fn test_mcp_config_json_contains_wallet() {
        let config = mcp_config_json("my-wallet", "base");
        assert!(config.contains("my-wallet"));
        assert!(config.contains("@pay-skill/mcp-server"));
        assert!(config.contains("eip155:8453"));
    }

    #[test]
    fn test_wallet_evm_address_chain_id() {
        let wallet: Value = serde_json::json!({
            "accounts": [{
                "chainId": "eip155:8453",
                "address": "0xabc123"
            }]
        });
        assert_eq!(wallet_evm_address(&wallet), Some("0xabc123".to_string()));
    }

    #[test]
    fn test_wallet_evm_address_chain_id_snake() {
        let wallet: Value = serde_json::json!({
            "accounts": [{
                "chain_id": "eip155:84532",
                "address": "0xdef456"
            }]
        });
        assert_eq!(wallet_evm_address(&wallet), Some("0xdef456".to_string()));
    }

    #[test]
    fn test_wallet_evm_address_empty_accounts() {
        let wallet: Value = serde_json::json!({
            "accounts": []
        });
        assert_eq!(wallet_evm_address(&wallet), None);
    }

    #[test]
    fn test_wallet_evm_address_no_evm() {
        let wallet: Value = serde_json::json!({
            "accounts": [{
                "chainId": "solana:mainnet",
                "address": "SolAddr"
            }]
        });
        assert_eq!(wallet_evm_address(&wallet), None);
    }
}
