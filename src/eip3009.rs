//! EIP-3009 TransferWithAuthorization signing for x402 direct settlement.
//!
//! Unlike EIP-2612 permits (which need a nonce from the USDC contract via the server),
//! EIP-3009 authorizations use a random nonce chosen by the signer. This means the
//! CLI can compute and sign the hash entirely locally.
//!
//! The signed authorization is sent as payment proof in x402 retry headers.
//! The provider's facilitator verifies off-chain, then settles on-chain.

use anyhow::{Context, Result};
use k256::ecdsa::SigningKey;
use sha3::{Digest, Keccak256};

use crate::auth;

/// EIP-3009 TransferWithAuthorization signature for x402 payment proof.
#[derive(serde::Serialize)]
pub struct TransferAuthorization {
    pub from: String,
    pub to: String,
    pub amount: u64,
    pub valid_after: String,
    pub valid_before: String,
    pub nonce: String,
    pub v: u8,
    pub r: String,
    pub s: String,
}

impl TransferAuthorization {
    /// Encode as JSON for the x402 payment header.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "from": self.from,
            "to": self.to,
            "amount": self.amount,
            "settlement": "direct",
            "valid_after": self.valid_after,
            "valid_before": self.valid_before,
            "nonce": self.nonce,
            "v": self.v,
            "r": self.r,
            "s": self.s,
        })
    }
}

/// Sign an EIP-3009 TransferWithAuthorization for x402 direct settlement.
///
/// Computes the EIP-712 hash locally using the USDC domain and signs it.
/// No server round-trip required — nonce is random.
pub fn sign_transfer_authorization(
    key: &SigningKey,
    to: &str,
    amount: u64,
    chain_id: u64,
    usdc_address: &str,
) -> Result<TransferAuthorization> {
    let from = auth::derive_address(key);

    // Random nonce (EIP-3009 uses random nonces, not sequential)
    let mut nonce_bytes = [0u8; 32];
    getrandom::fill(&mut nonce_bytes).map_err(|e| anyhow::anyhow!("rng failed: {e}"))?;
    let nonce_hex = format!("0x{}", hex::encode(nonce_bytes));

    // validAfter = 0 (immediately valid), validBefore = 0 (no expiry)
    // Server parses "0" as U256::ZERO which means no constraint
    let valid_after = "0".to_string();
    let valid_before = "0".to_string();

    // Compute EIP-712 hash
    let hash = compute_eip3009_hash(
        &from,
        to,
        amount,
        0, // validAfter
        0, // validBefore
        &nonce_bytes,
        chain_id,
        usdc_address,
    )?;

    // Sign
    let (sig, recid) = key
        .sign_prehash_recoverable(&hash)
        .map_err(|e| anyhow::anyhow!("EIP-3009 signing failed: {e}"))?;

    let r_bytes = sig.r().to_bytes();
    let s_bytes = sig.s().to_bytes();
    let v = recid.to_byte() + 27;

    Ok(TransferAuthorization {
        from,
        to: to.to_string(),
        amount,
        valid_after,
        valid_before,
        nonce: nonce_hex,
        v,
        r: format!("0x{}", hex::encode(r_bytes)),
        s: format!("0x{}", hex::encode(s_bytes)),
    })
}

/// Compute the EIP-712 hash for TransferWithAuthorization.
///
/// Domain: USDC token contract ("USD Coin", version "2").
/// Type: TransferWithAuthorization(address from, address to, uint256 value,
///       uint256 validAfter, uint256 validBefore, bytes32 nonce)
#[allow(clippy::too_many_arguments)]
fn compute_eip3009_hash(
    from: &str,
    to: &str,
    value: u64,
    valid_after: u64,
    valid_before: u64,
    nonce: &[u8; 32],
    chain_id: u64,
    usdc_address: &str,
) -> Result<[u8; 32]> {
    // Domain separator (USDC)
    let domain_typehash = keccak256(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );
    let name_hash = keccak256(b"USD Coin");
    let version_hash = keccak256(b"2");
    let usdc_bytes =
        parse_address(usdc_address).context("invalid USDC address for EIP-3009 domain")?;

    let mut domain_enc = [0u8; 160]; // 5 × 32
    domain_enc[0..32].copy_from_slice(&domain_typehash);
    domain_enc[32..64].copy_from_slice(&name_hash);
    domain_enc[64..96].copy_from_slice(&version_hash);
    // chain_id as uint256 (big-endian, right-aligned in 32-byte slot)
    domain_enc[120..128].copy_from_slice(&chain_id.to_be_bytes());
    // address left-padded to 32 bytes
    domain_enc[140..160].copy_from_slice(&usdc_bytes);
    let domain_separator = keccak256(&domain_enc);

    // Struct hash
    let struct_typehash = keccak256(
        b"TransferWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)",
    );

    let from_bytes = parse_address(from).context("invalid from address")?;
    let to_bytes = parse_address(to).context("invalid to address")?;

    let mut struct_enc = [0u8; 224]; // 7 × 32
    struct_enc[0..32].copy_from_slice(&struct_typehash);
    // from: address in 32-byte slot (right-aligned)
    struct_enc[44..64].copy_from_slice(&from_bytes);
    // to: address in 32-byte slot
    struct_enc[76..96].copy_from_slice(&to_bytes);
    // value: uint256
    struct_enc[120..128].copy_from_slice(&value.to_be_bytes());
    // validAfter: uint256
    struct_enc[152..160].copy_from_slice(&valid_after.to_be_bytes());
    // validBefore: uint256
    struct_enc[184..192].copy_from_slice(&valid_before.to_be_bytes());
    // nonce: bytes32
    struct_enc[192..224].copy_from_slice(nonce);
    let struct_hash = keccak256(&struct_enc);

    // Final EIP-712 hash: 0x19 0x01 || domainSeparator || structHash
    let mut final_enc = [0u8; 66];
    final_enc[0] = 0x19;
    final_enc[1] = 0x01;
    final_enc[2..34].copy_from_slice(&domain_separator);
    final_enc[34..66].copy_from_slice(&struct_hash);

    Ok(keccak256(&final_enc))
}

fn keccak256(data: &[u8]) -> [u8; 32] {
    Keccak256::digest(data).into()
}

fn parse_address(addr: &str) -> Result<[u8; 20]> {
    let clean = addr.strip_prefix("0x").unwrap_or(addr);
    let bytes = hex::decode(clean).map_err(|e| anyhow::anyhow!("invalid address hex: {e}"))?;
    if bytes.len() != 20 {
        return Err(anyhow::anyhow!(
            "address must be 20 bytes, got {}",
            bytes.len()
        ));
    }
    let mut out = [0u8; 20];
    out.copy_from_slice(&bytes);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANVIL_PK: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const ANVIL_ADDR: &str = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";
    const PROVIDER: &str = "0x70997970c51812dc3a010c7d01b50e0d17dc79c8";
    const USDC: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
    const CHAIN_ID: u64 = 8453;

    #[test]
    fn sign_and_recover_round_trip() {
        let key = SigningKey::from_slice(&hex::decode(ANVIL_PK).unwrap()).unwrap();
        let auth = sign_transfer_authorization(&key, PROVIDER, 1_000_000, CHAIN_ID, USDC).unwrap();

        // Verify from address matches
        assert_eq!(auth.from.to_lowercase(), ANVIL_ADDR.to_lowercase());
        assert_eq!(auth.to.to_lowercase(), PROVIDER.to_lowercase());
        assert_eq!(auth.amount, 1_000_000);
        assert!(auth.v == 27 || auth.v == 28);

        // Recover signer from the signature
        let r_bytes = hex::decode(auth.r.strip_prefix("0x").unwrap()).unwrap();
        let s_bytes = hex::decode(auth.s.strip_prefix("0x").unwrap()).unwrap();
        let mut sig_bytes = Vec::with_capacity(64);
        sig_bytes.extend_from_slice(&r_bytes);
        sig_bytes.extend_from_slice(&s_bytes);

        let nonce_clean = auth.nonce.strip_prefix("0x").unwrap();
        let nonce_bytes_vec = hex::decode(nonce_clean).unwrap();
        let mut nonce32 = [0u8; 32];
        nonce32.copy_from_slice(&nonce_bytes_vec);

        // Recompute the hash
        let hash = compute_eip3009_hash(
            &auth.from,
            &auth.to,
            auth.amount,
            0,
            0,
            &nonce32,
            CHAIN_ID,
            USDC,
        )
        .unwrap();

        let sig = k256::ecdsa::Signature::from_slice(&sig_bytes).unwrap();
        let rid = if auth.v >= 27 { auth.v - 27 } else { auth.v };
        let recid = k256::ecdsa::RecoveryId::try_from(rid).unwrap();
        let vk = k256::ecdsa::VerifyingKey::recover_from_prehash(&hash, &sig, recid).unwrap();

        let point = vk.to_encoded_point(false);
        let pub_bytes = &point.as_bytes()[1..];
        let addr_hash = keccak256(pub_bytes);
        let recovered = format!("0x{}", hex::encode(&addr_hash[12..]));

        assert_eq!(recovered.to_lowercase(), ANVIL_ADDR.to_lowercase());
    }

    #[test]
    fn different_amounts_produce_different_hashes() {
        let nonce = [0xabu8; 32];
        let hash1 = compute_eip3009_hash(
            ANVIL_ADDR, PROVIDER, 1_000_000, 0, 0, &nonce, CHAIN_ID, USDC,
        )
        .unwrap();
        let hash2 = compute_eip3009_hash(
            ANVIL_ADDR, PROVIDER, 2_000_000, 0, 0, &nonce, CHAIN_ID, USDC,
        )
        .unwrap();
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn different_recipients_produce_different_hashes() {
        let nonce = [0xabu8; 32];
        let other_provider = "0x3c44cdddb6a900fa2b585dd299e03d12fa4293bc";
        let hash1 = compute_eip3009_hash(
            ANVIL_ADDR, PROVIDER, 1_000_000, 0, 0, &nonce, CHAIN_ID, USDC,
        )
        .unwrap();
        let hash2 = compute_eip3009_hash(
            ANVIL_ADDR,
            other_provider,
            1_000_000,
            0,
            0,
            &nonce,
            CHAIN_ID,
            USDC,
        )
        .unwrap();
        assert_ne!(hash1, hash2);
    }
}
