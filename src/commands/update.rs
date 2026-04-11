use anyhow::{bail, Context, Result};
use clap::Args;
use serde::Deserialize;
use std::path::PathBuf;

use crate::error;

/// Update pay to the latest version
#[derive(Args)]
#[command(
    long_about = "Check GitHub Releases for the latest version. Downloads the matching binary \
        for your platform and replaces the current executable. Auto-detects package managers \
        (brew, scoop, choco, snap) and delegates to them if installed via one."
)]
pub struct UpdateArgs {
    /// Only check for updates, don't install
    #[arg(long)]
    check: bool,

    /// Skip confirmation (auto-yes). Also triggered in non-TTY environments.
    #[arg(long, short = 'y')]
    yes: bool,
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

const GITHUB_API_URL: &str = "https://api.github.com/repos/pay-skill/pay-cli/releases/latest";

/// Minimum plausible binary size (100 KB). Anything smaller is likely an error page or truncated.
const MIN_BINARY_SIZE: usize = 100_000;

pub async fn run(args: UpdateArgs, ctx: super::Context) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");

    // Check if installed via a package manager -- delegate update to it
    if let Some(mgr) = detect_package_manager() {
        let cmd = manager_update_command(&mgr);
        if ctx.json {
            error::print_json(&serde_json::json!({
                "installed_via": mgr,
                "update_command": cmd,
            }));
        } else {
            eprintln!("pay was installed via {mgr}. Update with:");
            eprintln!("  {cmd}");
        }
        return Ok(());
    }

    eprintln!("Checking for updates...");
    let client = reqwest::Client::builder().user_agent("pay-cli").build()?;

    let resp = client
        .get(GITHUB_API_URL)
        .send()
        .await
        .context("failed to reach GitHub API")?;

    // Handle rate limiting explicitly (60 req/hr unauthenticated)
    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        let reset = resp
            .headers()
            .get("x-ratelimit-reset")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown");
        bail!("GitHub API rate limited. Resets at Unix timestamp {reset}. Try again later.");
    }

    let release: GitHubRelease = resp
        .error_for_status()
        .context("GitHub API error")?
        .json()
        .await
        .context("invalid release response")?;

    let latest = release.tag_name.trim_start_matches('v');

    if !is_newer(latest, current) {
        if ctx.json {
            error::print_json(&serde_json::json!({
                "current": current,
                "latest": latest,
                "up_to_date": true,
            }));
        } else {
            eprintln!("Already up to date (v{current})");
        }
        return Ok(());
    }

    // Determine the correct asset for this platform
    let asset_name = platform_asset_name()
        .context("unsupported platform -- download manually from GitHub Releases")?;

    let asset = release.assets.iter().find(|a| a.name == asset_name);

    if args.check {
        if ctx.json {
            error::print_json(&serde_json::json!({
                "current": current,
                "latest": latest,
                "up_to_date": false,
                "asset": asset.map(|a| &a.name),
            }));
        } else {
            eprintln!("Update available: v{current} -> v{latest}");
        }
        // Exit code 1 signals "outdated" for scripting (`pay update --check && echo up-to-date`)
        std::process::exit(1);
    }

    let asset = asset.ok_or_else(|| {
        anyhow::anyhow!(
            "release v{latest} has no asset '{asset_name}' -- download manually from https://github.com/pay-skill/pay-cli/releases/latest"
        )
    })?;

    // Sanity check: GitHub reports the asset size. If it's tiny, something is wrong
    // with the release (e.g., placeholder file or build failure).
    if asset.size < MIN_BINARY_SIZE as u64 {
        bail!(
            "asset '{asset_name}' is only {} bytes -- likely a broken release. \
             Download manually from https://github.com/pay-skill/pay-cli/releases/latest",
            asset.size
        );
    }

    // Confirmation: skip if -y, or if not a TTY (agent/pipe)
    if !args.yes && error::is_terminal() {
        use std::io::Write;
        eprint!("Download and install v{latest}? [y/N] ");
        std::io::stderr().flush().ok();
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .context("failed to read input")?;
        if !matches!(input.trim().to_lowercase().as_str(), "y" | "yes") {
            eprintln!("Cancelled.");
            return Ok(());
        }
    }

    eprintln!(
        "Downloading {asset_name} ({:.1} MB)...",
        asset.size as f64 / 1_048_576.0
    );
    let bytes = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .context("download failed")?
        .error_for_status()
        .context("download failed")?
        .bytes()
        .await
        .context("failed to read download")?;

    // Validate downloaded bytes
    if bytes.len() < MIN_BINARY_SIZE {
        bail!(
            "downloaded binary is only {} bytes (expected ~{} bytes) -- aborting",
            bytes.len(),
            asset.size
        );
    }

    // Replace current binary
    let current_exe =
        std::env::current_exe().context("cannot determine current executable path")?;
    replace_binary(&current_exe, &bytes)?;

    if ctx.json {
        error::print_json(&serde_json::json!({
            "previous": current,
            "updated_to": latest,
            "success": true,
        }));
    } else {
        eprintln!("Updated pay v{current} -> v{latest}");
    }
    Ok(())
}

/// Compare semver strings. Returns true if `latest` is newer than `current`.
fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> (u64, u64, u64) {
        let parts: Vec<&str> = s.split('.').collect();
        let major = parts.first().and_then(|p| p.parse().ok()).unwrap_or(0);
        let minor = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = parts
            .get(2)
            .and_then(|p| {
                // Strip pre-release suffix (e.g. "3-rc.1" -> "3")
                p.split('-').next().and_then(|v| v.parse().ok())
            })
            .unwrap_or(0);
        (major, minor, patch)
    };
    parse(latest) > parse(current)
}

/// Map current OS + arch to the expected GitHub Release asset filename.
/// Pay CLI publishes plain binaries (no archives), unlike Remit which used tar.gz/zip.
fn platform_asset_name() -> Option<String> {
    let (os, arch) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => ("linux", "amd64"),
        ("linux", "aarch64") => ("linux", "arm64"),
        ("macos", "x86_64") => ("macos", "amd64"),
        ("macos", "aarch64") => ("macos", "arm64"),
        ("windows", "x86_64") => ("windows", "amd64"),
        _ => return None,
    };
    if cfg!(target_os = "windows") {
        Some(format!("pay-{os}-{arch}.exe"))
    } else {
        Some(format!("pay-{os}-{arch}"))
    }
}

/// Detect if the CLI was installed via a package manager.
fn detect_package_manager() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let path = exe.to_string_lossy();

    if std::env::var("SNAP").is_ok() {
        return Some("snap".into());
    }
    if std::env::var("HOMEBREW_PREFIX").is_ok() || path.contains("/Cellar/") {
        return Some("brew".into());
    }
    if path.contains("/scoop/") || path.contains("\\scoop\\") {
        return Some("scoop".into());
    }
    if path.contains("\\chocolatey\\") || path.contains("/chocolatey/") {
        return Some("choco".into());
    }
    // NOTE: We intentionally do NOT auto-detect .cargo/bin/ as "cargo".
    // The binary may have been placed there manually (e.g., downloaded from
    // GitHub Releases). Users who prefer cargo can run
    // `cargo install pay-cli --force` directly.

    None
}

fn manager_update_command(manager: &str) -> &'static str {
    match manager {
        "brew" => "brew upgrade pay-skill/tap/pay",
        "scoop" => "scoop update pay",
        "choco" => "choco upgrade pay-cli",
        "cargo" => "cargo install pay-cli --force",
        "snap" => "snap refresh pay",
        _ => "see https://github.com/pay-skill/pay-cli/releases/latest",
    }
}

/// Replace the current binary with new bytes.
/// On Windows: rename-to-.old (works around exe lock), write new, best-effort cleanup.
/// On Unix: write to temp file, chmod +x, atomic rename.
fn replace_binary(current_exe: &PathBuf, new_bytes: &[u8]) -> Result<()> {
    let dir = current_exe
        .parent()
        .context("cannot determine executable directory")?;

    if cfg!(target_os = "windows") {
        // Windows locks running executables -- rename current to .old, write new
        let old = current_exe.with_extension("exe.old");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(current_exe, &old).context("failed to rename current binary")?;
        if let Err(e) = std::fs::write(current_exe, new_bytes) {
            // Rollback: try to restore the old binary
            let _ = std::fs::rename(&old, current_exe);
            return Err(e).context("failed to write new binary (rolled back)");
        }
        // Best-effort cleanup of .old (may fail if still locked)
        let _ = std::fs::remove_file(&old);
    } else {
        // Unix: write to temp, chmod, atomic rename
        let tmp = dir.join(".pay-update-tmp");
        std::fs::write(&tmp, new_bytes).context("failed to write temp binary")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
                .context("failed to set executable permission")?;
        }
        std::fs::rename(&tmp, current_exe).context("failed to replace binary")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer_basic() {
        assert!(is_newer("1.0.0", "0.9.0"));
        assert!(is_newer("0.3.0", "0.2.5"));
        assert!(is_newer("0.2.6", "0.2.5"));
        assert!(is_newer("1.0.0", "0.99.99"));
    }

    #[test]
    fn test_is_newer_equal() {
        assert!(!is_newer("0.2.5", "0.2.5"));
        assert!(!is_newer("1.0.0", "1.0.0"));
    }

    #[test]
    fn test_is_newer_older() {
        assert!(!is_newer("0.2.4", "0.2.5"));
        assert!(!is_newer("0.1.0", "0.2.5"));
    }

    #[test]
    fn test_is_newer_prerelease_stripped() {
        assert!(is_newer("0.3.0", "0.2.5-rc.1"));
        assert!(!is_newer("0.2.5", "0.2.5-rc.1"));
    }

    #[test]
    fn test_platform_asset_name_returns_something() {
        let name = platform_asset_name();
        assert!(name.is_some(), "platform_asset_name() returned None");
        let name = name.unwrap();
        assert!(name.starts_with("pay-"));
    }

    #[test]
    fn test_platform_asset_name_format() {
        let name = platform_asset_name().expect("unsupported platform");
        if cfg!(target_os = "windows") {
            assert!(name.ends_with(".exe"));
        } else {
            assert!(!name.contains('.'));
        }
    }

    #[test]
    fn test_detect_package_manager_none_by_default() {
        // In a dev/CI env, just verify it doesn't panic
        let _ = detect_package_manager();
    }

    #[test]
    fn test_manager_update_command_known() {
        assert_eq!(
            manager_update_command("brew"),
            "brew upgrade pay-skill/tap/pay"
        );
        assert_eq!(
            manager_update_command("cargo"),
            "cargo install pay-cli --force"
        );
        assert_eq!(manager_update_command("snap"), "snap refresh pay");
        assert_eq!(manager_update_command("choco"), "choco upgrade pay-cli");
    }

    #[test]
    fn test_manager_update_command_unknown() {
        let cmd = manager_update_command("unknown");
        assert!(cmd.contains("github.com"));
    }
}
