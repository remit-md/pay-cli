//! OS-level identity verification before sensitive key operations.
//!
//! Triggers Windows Hello (fingerprint/face/PIN), macOS Touch ID, or
//! a PAM-verified password prompt on Linux.
//!
//! Every path either *cryptographically* verifies the user's identity
//! or hard-fails. There are no silent fallbacks.

use anyhow::{bail, Result};
use std::io::IsTerminal;

/// Verify the user's identity via OS biometric or password.
///
/// Returns `Ok(())` if verified, `Err` if denied or unavailable.
pub fn verify_identity(reason: &str) -> Result<()> {
    if !std::io::stderr().is_terminal() {
        bail!("Identity verification requires an interactive terminal.");
    }

    #[cfg(target_os = "windows")]
    {
        return verify_windows(reason);
    }

    #[cfg(target_os = "macos")]
    {
        return verify_macos(reason);
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        verify_linux(reason)
    }
}

// ── Windows: Windows Hello → LogonUserW fallback ─────────────────

#[cfg(target_os = "windows")]
fn verify_windows(reason: &str) -> Result<()> {
    use windows::core::factory;
    use windows::Security::Credentials::UI::*;
    use windows::Win32::System::Console::GetConsoleWindow;
    use windows::Win32::System::WinRT::IUserConsentVerifierInterop;

    // Check if Windows Hello is available and usable from this console.
    // WinRT UI APIs often fail in console contexts (no proper HWND, no
    // COM STA), so any failure falls through to password prompt.
    let avail = UserConsentVerifier::CheckAvailabilityAsync()
        .and_then(|op| op.GetResults())
        .ok();

    if avail != Some(UserConsentVerifierAvailability::Available) {
        return verify_windows_password(reason);
    }

    let interop = match factory::<UserConsentVerifier, IUserConsentVerifierInterop>() {
        Ok(i) => i,
        Err(_) => return verify_windows_password(reason),
    };

    let hwnd = unsafe { GetConsoleWindow() };
    let message: windows::core::HSTRING = reason.into();

    let result = unsafe { interop.RequestVerificationForWindowAsync(hwnd, &message) }
        .and_then(|op: windows_future::IAsyncOperation<UserConsentVerificationResult>| {
            op.GetResults()
        });

    match result {
        Ok(UserConsentVerificationResult::Verified) => Ok(()),
        Ok(_) => bail!("Identity verification denied."),
        Err(_) => verify_windows_password(reason),
    }
}

/// Windows password fallback: verify via LogonUserW.
#[cfg(target_os = "windows")]
fn verify_windows_password(reason: &str) -> Result<()> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Security::{LogonUserW, LOGON32_LOGON_INTERACTIVE, LOGON32_PROVIDER_DEFAULT};

    eprintln!("Identity verification required: {reason}");
    eprintln!();

    let username = whoami();
    eprintln!("Enter your login password for '{username}' to continue:");
    let password = rpassword::prompt_password("Password: ")
        .map_err(|e| anyhow::anyhow!("failed to read password: {e}"))?;

    if password.is_empty() {
        bail!("Identity verification cancelled.");
    }

    let user_wide: Vec<u16> = username.encode_utf16().chain(std::iter::once(0)).collect();
    // "." = local machine
    let domain_wide: Vec<u16> = ".".encode_utf16().chain(std::iter::once(0)).collect();
    let pass_wide: Vec<u16> = password.encode_utf16().chain(std::iter::once(0)).collect();

    let mut token = windows::Win32::Foundation::HANDLE::default();

    let ok = unsafe {
        LogonUserW(
            PCWSTR(user_wide.as_ptr()),
            PCWSTR(domain_wide.as_ptr()),
            PCWSTR(pass_wide.as_ptr()),
            LOGON32_LOGON_INTERACTIVE,
            LOGON32_PROVIDER_DEFAULT,
            &mut token,
        )
    };

    match ok {
        Ok(()) => {
            let _ = unsafe { CloseHandle(token) };
            Ok(())
        }
        Err(_) => bail!("Wrong password."),
    }
}

// ── macOS: Touch ID → dscl password verification fallback ────────

#[cfg(target_os = "macos")]
fn verify_macos(reason: &str) -> Result<()> {
    use std::process::Command;

    // Try Touch ID via LAContext (osascript JXA)
    let output = Command::new("osascript")
        .args(["-l", "JavaScript"])
        .arg("-e")
        .arg(&format!(
            r#"
            ObjC.import('LocalAuthentication');
            var context = $.LAContext.new;
            var reason = $['{}'];
            var result = context.evaluatePolicy_localizedReason_error(1, reason, null);
            result ? 'ok' : 'denied';
            "#,
            reason.replace('\'', r"\'")
        ))
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            if stdout.trim() == "ok" {
                return Ok(());
            }
            // Touch ID denied — don't fall back, user explicitly denied
            if stdout.trim() == "denied" {
                bail!("Identity verification denied.");
            }
        }
        _ => {
            // Touch ID not available (no hardware, not enrolled, etc.)
        }
    }

    eprintln!("Touch ID not available. Falling back to password prompt.");
    verify_macos_password(reason)
}

/// macOS password fallback: verify via `dscl . -authonly`.
#[cfg(target_os = "macos")]
fn verify_macos_password(reason: &str) -> Result<()> {
    use std::process::Command;

    eprintln!("Identity verification required: {reason}");
    eprintln!();

    let username = whoami();
    eprintln!("Enter your login password for '{username}' to continue:");
    let password = rpassword::prompt_password("Password: ")
        .map_err(|e| anyhow::anyhow!("failed to read password: {e}"))?;

    if password.is_empty() {
        bail!("Identity verification cancelled.");
    }

    // `dscl . -authonly <user> <password>` validates against the local directory
    let status = Command::new("dscl")
        .args([".", "-authonly", &username, &password])
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run dscl: {e}"))?;

    if status.status.success() {
        Ok(())
    } else {
        bail!("Wrong password.");
    }
}

// ── Linux: PAM verification via `su` ─────────────────────────────

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn verify_linux(reason: &str) -> Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    eprintln!("Identity verification required: {reason}");
    eprintln!();

    let username = whoami();
    eprintln!("Enter your login password for '{username}' to continue:");
    let password = rpassword::prompt_password("Password: ")
        .map_err(|e| anyhow::anyhow!("failed to read password: {e}"))?;

    if password.is_empty() {
        bail!("Identity verification cancelled.");
    }

    let mut child = Command::new("su")
        .args(["-c", "true", &username])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn su: {e}"))?;

    if let Some(ref mut stdin) = child.stdin {
        let _ = writeln!(stdin, "{password}");
    }

    let status = child.wait()?;
    if !status.success() {
        bail!("Wrong password.");
    }
    Ok(())
}

fn whoami() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".to_string())
}

// ── Clipboard utilities ──────────────────────────────────────────

/// Copy text to clipboard and schedule auto-clear after `clear_after_secs`.
pub fn clipboard_copy_and_clear(text: &str, clear_after_secs: u64) -> bool {
    if !clipboard_copy(text) {
        return false;
    }

    let secs = clear_after_secs;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(secs));
        clipboard_copy("");
    });

    true
}

fn clipboard_copy(text: &str) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};

    #[cfg(target_os = "windows")]
    {
        if let Ok(mut child) = Command::new("cmd")
            .args(["/C", "clip"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(ref mut stdin) = child.stdin {
                let _ = stdin.write_all(text.as_bytes());
            }
            return child.wait().map(|s| s.success()).unwrap_or(false);
        }
        return false;
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(mut child) = Command::new("pbcopy")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(ref mut stdin) = child.stdin {
                let _ = stdin.write_all(text.as_bytes());
            }
            return child.wait().map(|s| s.success()).unwrap_or(false);
        }
        return false;
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        for cmd in &["xclip", "xsel"] {
            let args: &[&str] = if *cmd == "xclip" {
                &["-selection", "clipboard"]
            } else {
                &["--clipboard", "--input"]
            };
            if let Ok(mut child) = Command::new(cmd)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(text.as_bytes());
                }
                if child.wait().map(|s| s.success()).unwrap_or(false) {
                    return true;
                }
            }
        }
        false
    }
}
