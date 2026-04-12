//! EIP-2612 permit signing for USDC approvals.
//!
//! Flow:
//! 1. POST /permit/prepare { amount, spender } — server returns EIP-712 hash, nonce, deadline
//! 2. CLI signs the hash with the agent's signing key
//! 3. CLI includes (nonce, deadline, v, r, s) in the payment request body

use anyhow::{bail, Context, Result};

use crate::commands;

/// Permit signature fields required by the server's payment endpoints.
#[derive(serde::Serialize)]
pub struct PermitSignature {
    pub nonce: String,
    pub deadline: u64,
    pub v: u8,
    pub r: String,
    pub s: String,
}

impl PermitSignature {
    /// Convert to a serde_json::Value for embedding in request bodies.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "nonce": self.nonce,
            "deadline": self.deadline,
            "v": self.v,
            "r": self.r,
            "s": self.s,
        })
    }
}

/// Prepare and sign an EIP-2612 permit via the server's /permit/prepare endpoint.
///
/// The server computes the EIP-712 hash (including the USDC nonce from the contract),
/// and the CLI signs it locally. This avoids the CLI needing direct RPC access.
pub async fn prepare_and_sign(
    ctx: &mut commands::Context,
    amount: u64,
    spender: &str,
) -> Result<PermitSignature> {
    // 1. Ask server to prepare the permit hash
    let body = serde_json::json!({ "amount": amount, "spender": spender });
    let resp = ctx
        .post("/permit/prepare", &body)
        .await
        .context("failed to prepare permit — is the server running?")?;

    let hash_hex = resp["hash"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("server did not return permit hash"))?;
    let nonce = resp["nonce"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("server did not return permit nonce"))?
        .to_string();
    let deadline = resp["deadline"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("server did not return permit deadline"))?;

    // 2. Decode the 32-byte hash
    let hash_clean = hash_hex.strip_prefix("0x").unwrap_or(hash_hex);
    let hash_bytes = hex::decode(hash_clean).context("invalid permit hash hex")?;
    if hash_bytes.len() != 32 {
        bail!("permit hash must be 32 bytes, got {}", hash_bytes.len());
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&hash_bytes);

    // 3. Sign the hash with the agent's key
    let key = ctx.load_key()?;
    let (sig, recid) = key
        .sign_prehash_recoverable(&hash)
        .map_err(|e| anyhow::anyhow!("permit signing failed: {e}"))?;

    let r_bytes = sig.r().to_bytes();
    let s_bytes = sig.s().to_bytes();
    let v = recid.to_byte() + 27;

    Ok(PermitSignature {
        nonce,
        deadline,
        v,
        r: format!("0x{}", hex::encode(r_bytes)),
        s: format!("0x{}", hex::encode(s_bytes)),
    })
}

/// Ensure the PayDirect contract has a stored approval for this wallet.
/// Signs a max-value permit (off-chain, free) and stores it server-side.
/// The permit is submitted on-chain only at first withdrawal.
pub async fn ensure_relayer_approved(ctx: &mut commands::Context) -> Result<()> {
    let contracts = get_contracts(ctx).await?;

    // Sign permit: spender = PayDirect, value = max u64.
    // Deadline 1 year from now — effectively permanent for dashboard use.
    let max_value: u64 = u64::MAX;
    let far_deadline: u64 = now_secs() + 365 * 24 * 60 * 60;
    let permit = prepare_and_sign_with_deadline(
        ctx,
        max_value,
        &contracts.direct,
        far_deadline,
    )
    .await?;

    // Store server-side (no gas, just DB). Use the ACTUAL signed deadline.
    ctx.post(
        "/relayer-approval",
        &serde_json::json!({
            "value": max_value,
            "deadline": permit.deadline,
            "v": permit.v,
            "r": permit.r,
            "s": permit.s,
        }),
    )
    .await
    .context("failed to store relayer approval")?;

    Ok(())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time before epoch")
        .as_secs()
}

/// Like `prepare_and_sign` but with a custom deadline.
pub async fn prepare_and_sign_with_deadline(
    ctx: &mut commands::Context,
    amount: u64,
    spender: &str,
    deadline: u64,
) -> Result<PermitSignature> {
    let body = serde_json::json!({ "amount": amount, "spender": spender, "deadline": deadline });
    let resp = ctx
        .post("/permit/prepare", &body)
        .await
        .context("failed to prepare permit")?;

    let hash_hex = resp["hash"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("server did not return permit hash"))?;
    let nonce = resp["nonce"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("server did not return permit nonce"))?
        .to_string();
    let resp_deadline = resp["deadline"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("server did not return permit deadline"))?;

    let hash_clean = hash_hex.strip_prefix("0x").unwrap_or(hash_hex);
    let hash_bytes = hex::decode(hash_clean).context("invalid permit hash hex")?;
    if hash_bytes.len() != 32 {
        bail!("permit hash must be 32 bytes, got {}", hash_bytes.len());
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&hash_bytes);

    let key = ctx.load_key()?;
    let (sig, recid) = key
        .sign_prehash_recoverable(&hash)
        .map_err(|e| anyhow::anyhow!("permit signing failed: {e}"))?;

    let r_bytes = sig.r().to_bytes();
    let s_bytes = sig.s().to_bytes();
    let v = recid.to_byte() + 27;

    Ok(PermitSignature {
        nonce,
        deadline: resp_deadline,
        v,
        r: format!("0x{}", hex::encode(r_bytes)),
        s: format!("0x{}", hex::encode(s_bytes)),
    })
}

/// Fetch contract addresses from the server's /contracts endpoint.
/// Returns (router, tab, direct, usdc) addresses.
pub async fn get_contracts(ctx: &mut commands::Context) -> Result<ContractAddresses> {
    let resp = ctx
        .get("/contracts")
        .await
        .context("failed to fetch contract addresses")?;

    Ok(ContractAddresses {
        router: resp["router"].as_str().unwrap_or_default().to_string(),
        tab: resp["tab"].as_str().unwrap_or_default().to_string(),
        tab_v2: resp["tab_v2"].as_str().unwrap_or_default().to_string(),
        direct: resp["direct"].as_str().unwrap_or_default().to_string(),
        usdc: resp["usdc"].as_str().unwrap_or_default().to_string(),
        relayer: resp["relayer"].as_str().unwrap_or_default().to_string(),
    })
}

pub struct ContractAddresses {
    #[allow(dead_code)]
    pub router: String,
    pub tab: String,
    pub tab_v2: String,
    pub direct: String,
    pub usdc: String,
    pub relayer: String,
}

impl ContractAddresses {
    /// Active tab contract: prefers tab_v2 (PayTabV3+) over v1.
    pub fn active_tab(&self) -> &str {
        if self.tab_v2.is_empty() {
            &self.tab
        } else {
            &self.tab_v2
        }
    }
}
