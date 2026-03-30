use std::io::{self, BufRead, Write};

use anyhow::{bail, Result};
use clap::Args;

#[derive(Args)]
pub struct SignArgs;

/// Signer subprocess: reads hex hash from stdin, writes hex signature to stdout.
/// This is the protocol used by SDKs to delegate signing to the CLI.
pub async fn run(_args: SignArgs, _ctx: super::Context) -> Result<()> {
    let stdin = io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    let hash_hex = line.trim();

    if hash_hex.is_empty() {
        bail!("No hash provided on stdin");
    }

    // Stub: actual signing will use keychain/encrypted key
    // For now, return an error indicating not yet implemented
    bail!("Signer not yet implemented. Key management will be added in a future task.");
}

/// Flush stdout after writing signature (important for subprocess protocol).
#[allow(dead_code)]
fn write_signature(sig_hex: &str) -> Result<()> {
    let mut stdout = io::stdout();
    write!(stdout, "{sig_hex}")?;
    stdout.flush()?;
    Ok(())
}
