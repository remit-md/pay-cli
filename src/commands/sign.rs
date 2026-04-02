use std::io::{self, BufRead, Write};

use anyhow::{bail, Context, Result};
use clap::Args;
use k256::ecdsa::SigningKey;

#[derive(Args)]
pub struct SignArgs;

/// Signer subprocess: reads hex hash from stdin, writes hex signature to stdout.
/// This is the protocol used by SDKs to delegate signing to the CLI.
///
/// Key is loaded from PAYSKILL_SIGNER_KEY environment variable (hex-encoded private key).
pub async fn run(_args: SignArgs, _ctx: super::Context) -> Result<()> {
    let key = load_signing_key()?;

    let stdin = io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    let hash_hex = line.trim().strip_prefix("0x").unwrap_or(line.trim());

    if hash_hex.is_empty() {
        bail!("No hash provided on stdin");
    }

    let hash_bytes = hex::decode(hash_hex).context("Invalid hex hash on stdin")?;

    if hash_bytes.len() != 32 {
        bail!("Hash must be exactly 32 bytes, got {}", hash_bytes.len());
    }

    let sig = sign_hash(&key, &hash_bytes)?;
    write_signature(&sig)?;

    Ok(())
}

/// Load the signing key via the unified signer resolution chain.
fn load_signing_key() -> Result<SigningKey> {
    crate::signer::resolve_key()
}

/// Sign a 32-byte hash and return the 65-byte Ethereum signature (r || s || v).
fn sign_hash(key: &SigningKey, hash: &[u8]) -> Result<String> {
    let (sig, recid) = key
        .sign_prehash_recoverable(hash)
        .map_err(|e| anyhow::anyhow!("signing failed: {e}"))?;

    let r = sig.r().to_bytes();
    let s = sig.s().to_bytes();
    // Ethereum v = recovery_id + 27
    let v = recid.to_byte() + 27;

    let mut result = Vec::with_capacity(65);
    result.extend_from_slice(&r);
    result.extend_from_slice(&s);
    result.push(v);

    Ok(hex::encode(result))
}

/// Write hex signature to stdout and flush.
fn write_signature(sig_hex: &str) -> Result<()> {
    let mut stdout = io::stdout();
    write!(stdout, "{sig_hex}")?;
    stdout.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_and_verify_length() {
        // Generate a test key
        let key = SigningKey::from_slice(
            &hex::decode("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")
                .unwrap(),
        )
        .unwrap();

        let hash = [0u8; 32]; // Zero hash for testing
        let sig_hex = sign_hash(&key, &hash).unwrap();

        // Signature should be 65 bytes = 130 hex chars
        assert_eq!(sig_hex.len(), 130);

        // v should be 27 or 28
        let sig_bytes = hex::decode(&sig_hex).unwrap();
        assert!(sig_bytes[64] == 27 || sig_bytes[64] == 28);
    }

    #[test]
    fn test_sign_deterministic() {
        let key = SigningKey::from_slice(
            &hex::decode("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")
                .unwrap(),
        )
        .unwrap();

        let hash = [1u8; 32];
        let sig1 = sign_hash(&key, &hash).unwrap();
        let sig2 = sign_hash(&key, &hash).unwrap();

        // RFC 6979 deterministic signatures
        assert_eq!(sig1, sig2);
    }
}
