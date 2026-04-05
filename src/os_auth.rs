//! OS-level identity verification before sensitive key operations.
//!
//! Triggers Windows Hello (fingerprint/face/PIN), macOS Touch ID (with
//! password fallback), or a CLI password prompt on Linux / fallback.
//!
//! Every operation that extracts the raw private key from the keychain
//! MUST call `verify_identity()` first.

use anyhow::{bail, Result};
use std::io::IsTerminal;

/// Prompt the user to verify their identity via OS biometric/password.
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
        return verify_password_fallback(reason);
    }
}

// ── Windows: Windows Hello ────────────────────────────────────────

#[cfg(target_os = "windows")]
fn verify_windows(reason: &str) -> Result<()> {
    use windows::core::factory;
    use windows::Security::Credentials::UI::*;
    use windows::Win32::System::Console::GetConsoleWindow;
    use windows::Win32::System::WinRT::IUserConsentVerifierInterop;

    // Check if Windows Hello is available
    let avail = UserConsentVerifier::CheckAvailabilityAsync()
        .map_err(|e| anyhow::anyhow!("Windows Hello check failed: {e}"))?
        .get()
        .map_err(|e| anyhow::anyhow!("Windows Hello check failed: {e}"))?;

    if avail != UserConsentVerifierAvailability::Available {
        eprintln!("Windows Hello not available. Falling back to password prompt.");
        return verify_password_fallback(reason);
    }

    let interop: IUserConsentVerifierInterop =
        factory::<UserConsentVerifier, IUserConsentVerifierInterop>()
            .map_err(|e| anyhow::anyhow!("Windows Hello interop failed: {e}"))?;

    let hwnd = unsafe { GetConsoleWindow() };
    let message: windows::core::HSTRING = reason.into();

    let op: windows::Foundation::IAsyncOperation<UserConsentVerificationResult> =
        unsafe { interop.RequestVerificationForWindowAsync(hwnd, &message) }
            .map_err(|e| anyhow::anyhow!("Windows Hello request failed: {e}"))?;
    let result = op
        .get()
        .map_err(|e| anyhow::anyhow!("Windows Hello verification failed: {e}"))?;

    if result == UserConsentVerificationResult::Verified {
        Ok(())
    } else {
        bail!("Identity verification denied.");
    }
}

// ── macOS: Touch ID / password ────────────────────────────────────

#[cfg(target_os = "macos")]
fn verify_macos(reason: &str) -> Result<()> {
    // Use `biometric-auth` crate or shell out to osascript as fallback.
    // For now, use the `security` command to trigger keychain auth prompt.
    use std::process::Command;

    // LAContext via osascript — triggers system auth dialog (Touch ID or password)
    let script = format!(
        r#"
        use framework "LocalAuthentication"
        set authContext to current application's LAContext's new()
        set authResult to authContext's evaluatePolicy:1 localizedReason:"{}" |error|:(missing value)
        if authResult then
            return "ok"
        else
            return "denied"
        end if
        "#,
        reason.replace('"', r#"\""#)
    );

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
                Ok(())
            } else {
                bail!("Identity verification denied.");
            }
        }
        _ => {
            eprintln!("Touch ID not available. Falling back to password prompt.");
            verify_password_fallback(reason)
        }
    }
}

// ── Fallback: password re-entry ───────────────────────────────────

#[allow(dead_code)]
fn verify_password_fallback(reason: &str) -> Result<()> {
    eprintln!("Identity verification required: {reason}");
    eprintln!();

    // Get current OS username
    let username = whoami();

    eprintln!("Enter your login password for '{username}' to continue:");
    let password = rpassword::prompt_password("Password: ")
        .map_err(|e| anyhow::anyhow!("failed to read password: {e}"))?;

    if password.is_empty() {
        bail!("Identity verification cancelled.");
    }

    // On Linux, try PAM verification via `su -c true`
    #[cfg(target_os = "linux")]
    {
        use std::process::{Command, Stdio};
        let mut child = Command::new("su")
            .args(["-c", "true", &username])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to spawn su: {e}"))?;

        if let Some(ref mut stdin) = child.stdin {
            use std::io::Write;
            let _ = writeln!(stdin, "{password}");
        }

        let status = child.wait()?;
        if !status.success() {
            bail!("Wrong password.");
        }
        return Ok(());
    }

    // On Windows (Hello unavailable), verify via LogonUserW.
    #[cfg(target_os = "windows")]
    {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::Security::LogonUserW;
        use windows::Win32::Security::LOGON32_LOGON_NETWORK;
        use windows::Win32::Security::LOGON32_PROVIDER_DEFAULT;

        let user_wide: Vec<u16> = username.encode_utf16().chain(std::iter::once(0)).collect();
        // Empty domain = local machine
        let domain_wide: Vec<u16> = ".".encode_utf16().chain(std::iter::once(0)).collect();
        let pass_wide: Vec<u16> = password.encode_utf16().chain(std::iter::once(0)).collect();

        let mut token = windows::Win32::Foundation::HANDLE::default();

        let ok = unsafe {
            LogonUserW(
                PCWSTR(user_wide.as_ptr()),
                PCWSTR(domain_wide.as_ptr()),
                PCWSTR(pass_wide.as_ptr()),
                LOGON32_LOGON_NETWORK,
                LOGON32_PROVIDER_DEFAULT,
                &mut token,
            )
        };

        match ok {
            Ok(()) => {
                // Close the token handle — we only needed to verify
                let _ = unsafe { CloseHandle(token) };
                return Ok(());
            }
            Err(_) => {
                bail!("Wrong password.");
            }
        }
    }

    // On macOS (Touch ID unavailable), there is no reliable non-PAM
    // verification. Refuse the operation rather than pretending to verify.
    #[cfg(target_os = "macos")]
    {
        let _ = password;
        bail!(
            "Cannot verify identity: Touch ID unavailable and no fallback \
             verifier on macOS. Please enable Touch ID or use an encrypted \
             key file (.enc) instead."
        );
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = password;
        bail!("Cannot verify identity: no supported verifier on this platform.");
    }
}

fn whoami() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".to_string())
}

// ── Clipboard utilities ───────────────────────────────────────────

/// Copy text to clipboard and schedule auto-clear after `clear_after_secs`.
/// Returns true if clipboard copy succeeded.
pub fn clipboard_copy_and_clear(text: &str, clear_after_secs: u64) -> bool {
    if !clipboard_copy(text) {
        return false;
    }

    // Spawn background thread to clear clipboard
    let secs = clear_after_secs;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(secs));
        // Overwrite clipboard with empty string
        clipboard_copy("");
    });

    true
}

fn clipboard_copy(text: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::process::{Command, Stdio};
        if let Ok(mut child) = Command::new("cmd")
            .args(["/C", "clip"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            use std::io::Write;
            if let Some(ref mut stdin) = child.stdin {
                let _ = stdin.write_all(text.as_bytes());
            }
            return child.wait().map(|s| s.success()).unwrap_or(false);
        }
        false
    }

    #[cfg(target_os = "macos")]
    {
        use std::process::{Command, Stdio};
        if let Ok(mut child) = Command::new("pbcopy")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            use std::io::Write;
            if let Some(ref mut stdin) = child.stdin {
                let _ = stdin.write_all(text.as_bytes());
            }
            return child.wait().map(|s| s.success()).unwrap_or(false);
        }
        false
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::{Command, Stdio};
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
                use std::io::Write;
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

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = text;
        false
    }
}
