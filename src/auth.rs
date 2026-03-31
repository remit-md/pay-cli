//! EIP-712 authentication for pay API requests.
//!
//! Adds X-Pay-* headers to every authenticated API request.
//! Port of the server's auth module — must produce identical hashes.

use anyhow::{Context, Result};
use k256::ecdsa::SigningKey;

/// Build auth headers for an API request.
///
/// Returns a vec of (header_name, header_value) pairs.
pub fn build_auth_headers(
    key: &SigningKey,
    method: &str,
    path: &str,
    chain_id: u64,
    router_address: &str,
) -> Result<Vec<(String, String)>> {
    let address = derive_address(key);

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut nonce_bytes = [0u8; 32];
    getrandom::fill(&mut nonce_bytes).map_err(|e| anyhow::anyhow!("rng failed: {e}"))?;
    let nonce_hex = format!("0x{}", hex::encode(nonce_bytes));

    let hash = compute_eip712_hash(
        &method.to_uppercase(),
        path,
        timestamp,
        &nonce_hex,
        chain_id,
        router_address,
    )?;

    let sig_hex = sign_hash(key, &hash)?;

    Ok(vec![
        ("X-Pay-Agent".to_string(), address),
        ("X-Pay-Signature".to_string(), sig_hex),
        ("X-Pay-Timestamp".to_string(), timestamp.to_string()),
        ("X-Pay-Nonce".to_string(), nonce_hex),
    ])
}

/// Derive Ethereum address from a signing key.
pub fn derive_address(key: &SigningKey) -> String {
    let vk = key.verifying_key();
    let point = vk.to_encoded_point(false);
    let pub_bytes = &point.as_bytes()[1..]; // skip 0x04 prefix
    let hash = keccak256(pub_bytes);
    format!("0x{}", hex::encode(&hash[12..]))
}

/// Sign a 32-byte hash. Returns 0x-prefixed hex signature (65 bytes).
fn sign_hash(key: &SigningKey, hash: &[u8; 32]) -> Result<String> {
    let (sig, recid) = key
        .sign_prehash_recoverable(hash)
        .map_err(|e| anyhow::anyhow!("signing failed: {e}"))?;
    let mut result = Vec::with_capacity(65);
    result.extend_from_slice(&sig.r().to_bytes());
    result.extend_from_slice(&sig.s().to_bytes());
    result.push(recid.to_byte() + 27);
    Ok(format!("0x{}", hex::encode(result)))
}

/// Compute EIP-712 hash for an APIRequest.
/// Must match the server's computation exactly.
fn compute_eip712_hash(
    method: &str,
    path: &str,
    timestamp: u64,
    nonce_hex: &str,
    chain_id: u64,
    verifying_contract: &str,
) -> Result<[u8; 32]> {
    let domain_typehash = keccak256(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );
    let struct_typehash =
        keccak256(b"APIRequest(string method,string path,uint256 timestamp,bytes32 nonce)");

    let name_hash = keccak256(b"pay");
    let version_hash = keccak256(b"0.1");

    let contract_bytes = parse_address(verifying_contract)
        .context("invalid router address for EIP-712 domain")?;

    // Domain separator
    let mut domain_enc = [0u8; 160];
    domain_enc[0..32].copy_from_slice(&domain_typehash);
    domain_enc[32..64].copy_from_slice(&name_hash);
    domain_enc[64..96].copy_from_slice(&version_hash);
    domain_enc[120..128].copy_from_slice(&chain_id.to_be_bytes());
    domain_enc[140..160].copy_from_slice(&contract_bytes);
    let domain_separator = keccak256(&domain_enc);

    // Parse nonce
    let nonce_clean = nonce_hex.strip_prefix("0x").unwrap_or(nonce_hex);
    let nonce_bytes = hex::decode(nonce_clean)
        .map_err(|e| anyhow::anyhow!("invalid nonce hex: {e}"))?;
    let mut nonce32 = [0u8; 32];
    let len = nonce_bytes.len().min(32);
    nonce32[..len].copy_from_slice(&nonce_bytes[..len]);

    // Struct hash
    let method_hash = keccak256(method.as_bytes());
    let path_hash = keccak256(path.as_bytes());

    let mut struct_enc = [0u8; 160];
    struct_enc[0..32].copy_from_slice(&struct_typehash);
    struct_enc[32..64].copy_from_slice(&method_hash);
    struct_enc[64..96].copy_from_slice(&path_hash);
    struct_enc[120..128].copy_from_slice(&timestamp.to_be_bytes());
    struct_enc[128..160].copy_from_slice(&nonce32);
    let struct_hash = keccak256(&struct_enc);

    // Final hash
    let mut final_enc = [0u8; 66];
    final_enc[0] = 0x19;
    final_enc[1] = 0x01;
    final_enc[2..34].copy_from_slice(&domain_separator);
    final_enc[34..66].copy_from_slice(&struct_hash);

    Ok(keccak256(&final_enc))
}

fn keccak256(data: &[u8]) -> [u8; 32] {
    use sha3::{Digest, Keccak256};
    Keccak256::digest(data).into()
}

fn parse_address(addr: &str) -> Result<[u8; 20]> {
    let clean = addr.strip_prefix("0x").unwrap_or(addr);
    let bytes = hex::decode(clean)
        .map_err(|e| anyhow::anyhow!("invalid address hex: {e}"))?;
    if bytes.len() != 20 {
        return Err(anyhow::anyhow!("address must be 20 bytes, got {}", bytes.len()));
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
    const TEST_CONTRACT: &str = "0xf39Fd6e51aad88F6f4ce6aB8827279cffFb92266";

    #[test]
    fn derive_address_anvil() {
        let key = SigningKey::from_slice(&hex::decode(ANVIL_PK).unwrap()).unwrap();
        let addr = derive_address(&key);
        assert_eq!(addr.to_lowercase(), ANVIL_ADDR.to_lowercase());
    }

    #[test]
    fn build_and_verify_auth_headers() {
        let key = SigningKey::from_slice(&hex::decode(ANVIL_PK).unwrap()).unwrap();
        let headers = build_auth_headers(&key, "POST", "/api/v1/direct", 8453, TEST_CONTRACT).unwrap();

        assert_eq!(headers.len(), 4);

        let agent = &headers[0].1;
        assert_eq!(agent.to_lowercase(), ANVIL_ADDR.to_lowercase());

        let sig = &headers[1].1;
        assert!(sig.starts_with("0x"));
        assert_eq!(sig.len(), 132); // 0x + 130 hex chars = 65 bytes
    }

    #[test]
    fn sign_and_recover_round_trip() {
        let key = SigningKey::from_slice(&hex::decode(ANVIL_PK).unwrap()).unwrap();
        let hash = [0xdeu8; 32];
        let sig_hex = sign_hash(&key, &hash).unwrap();

        // Decode and recover
        let sig_clean = sig_hex.strip_prefix("0x").unwrap();
        let sig_bytes = hex::decode(sig_clean).unwrap();
        assert_eq!(sig_bytes.len(), 65);

        let r_s = &sig_bytes[0..64];
        let v = sig_bytes[64];
        let rid = if v >= 27 { v - 27 } else { v };

        let sig = k256::ecdsa::Signature::from_slice(r_s).unwrap();
        let recid = k256::ecdsa::RecoveryId::try_from(rid).unwrap();
        let vk = k256::ecdsa::VerifyingKey::recover_from_prehash(&hash, &sig, recid).unwrap();

        let point = vk.to_encoded_point(false);
        let pub_bytes = &point.as_bytes()[1..];
        let addr_hash = keccak256(pub_bytes);
        let recovered = format!("0x{}", hex::encode(&addr_hash[12..]));

        assert_eq!(recovered.to_lowercase(), ANVIL_ADDR.to_lowercase());
    }
}
