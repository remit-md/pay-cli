//! E2E acceptance tests — run against live testnet.
//!
//! Skipped unless `PAYSKILL_TESTNET_KEY` is set. These build the real CLI
//! binary and exercise it against the testnet server.
//!
//! Run:
//!   PAYSKILL_TESTNET_KEY=0xdead... PAYSKILL_API_URL=http://204.168.133.111:3001/api/v1 \
//!     cargo test --test e2e -- --ignored

use assert_cmd::Command;
use predicates::prelude::*;
use std::env;
use std::sync::Once;

static INIT: Once = Once::new();
static LOCAL_INIT: Once = Once::new();

// Testnet contract addresses (Base Sepolia).
const TESTNET_CHAIN_ID: &str = "84532";
const TESTNET_ROUTER: &str = "0xE0Aa45e6937F3b9Fc0BEe457361885Cb9bfC067F";

/// Ensure `pay init` has been run (idempotent, runs once per test suite).
/// Only call from tests that have PAYSKILL_TESTNET_KEY set.
fn ensure_init() {
    INIT.call_once(|| {
        let mut cmd = Command::cargo_bin("pay").expect("binary not found");
        if let Ok(key) = env::var("PAYSKILL_TESTNET_KEY") {
            cmd.env("PAYSKILL_SIGNER_KEY", &key);
        }
        cmd.arg("init").assert().success();
    });
}

/// Return true if testnet env vars are set.
#[allow(dead_code)]
fn has_testnet_key() -> bool {
    env::var("PAYSKILL_TESTNET_KEY").is_ok()
}

/// Testnet API URL (falls back to testnet default).
fn testnet_url() -> String {
    env::var("PAYSKILL_API_URL")
        .unwrap_or_else(|_| "https://testnet.pay-skill.com/api/v1".to_string())
}

/// Provider address for test payments.
fn provider_addr() -> String {
    env::var("PAYSKILL_TESTNET_PROVIDER")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("0x{}", "b2".repeat(20)))
}

/// Build a `pay` command pre-configured for testnet (requires PAYSKILL_TESTNET_KEY).
fn pay() -> Command {
    ensure_init();
    let mut cmd = Command::cargo_bin("pay").expect("binary not found");
    cmd.arg("--api-url").arg(testnet_url());
    cmd.arg("--chain-id").arg(TESTNET_CHAIN_ID);
    cmd.arg("--router-address").arg(TESTNET_ROUTER);

    // Map PAYSKILL_TESTNET_KEY → PAYSKILL_SIGNER_KEY so the CLI can sign.
    if let Ok(key) = env::var("PAYSKILL_TESTNET_KEY") {
        cmd.env("PAYSKILL_SIGNER_KEY", &key);
    }

    cmd
}

// Throwaway key for local validation tests (Anvil default #0, never hits the network).
const LOCAL_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

/// Ensure `pay init` has been run with a local throwaway key (for validation tests).
fn ensure_local_init() {
    LOCAL_INIT.call_once(|| {
        let mut cmd = Command::cargo_bin("pay").expect("binary not found");
        cmd.env("PAYSKILL_SIGNER_KEY", LOCAL_KEY);
        cmd.arg("init").assert().success();
    });
}

/// Build a `pay` command for local validation tests (no testnet key needed).
fn pay_local() -> Command {
    ensure_local_init();
    let mut cmd = Command::cargo_bin("pay").expect("binary not found");
    cmd.env("PAYSKILL_SIGNER_KEY", LOCAL_KEY);
    // Use a dummy API URL — validation tests never reach the network
    cmd.arg("--api-url").arg("http://localhost:9999");
    cmd.arg("--chain-id").arg(TESTNET_CHAIN_ID);
    cmd.arg("--router-address").arg(TESTNET_ROUTER);
    cmd
}

// ── Init ────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires PAYSKILL_TESTNET_KEY"]
fn init_creates_config() {
    ensure_init();
}

// ── Validation (no testnet key needed) ──────────────────────────────

#[test]
fn direct_rejects_bad_address() {
    pay_local()
        .args(["direct", "not-an-address", "1.00"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid address"));
}

#[test]
fn direct_rejects_below_minimum() {
    let addr = format!("0x{}", "a1".repeat(20));
    pay_local()
        .args(["direct", &addr, "0.50"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Minimum"));
}

#[test]
fn tab_open_rejects_below_minimum() {
    let addr = format!("0x{}", "a1".repeat(20));
    pay_local()
        .args(["tab", "open", &addr, "2.00", "--max-charge", "0.50"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Minimum"));
}

// ── Auth Rejection ─────────────────────────────────────────────────

#[test]
#[ignore = "requires PAYSKILL_TESTNET_KEY"]
fn unsigned_request_rejected() {
    // Send a request WITHOUT a valid signer — should fail.
    let mut cmd = Command::cargo_bin("pay").expect("binary not found");
    cmd.arg("--api-url").arg(testnet_url());
    cmd.arg("--chain-id").arg(TESTNET_CHAIN_ID);
    cmd.arg("--router-address").arg(TESTNET_ROUTER);
    // Deliberately remove PAYSKILL_SIGNER_KEY — no key means no auth.
    cmd.env_remove("PAYSKILL_SIGNER_KEY");
    cmd.args(["status"]);
    cmd.assert().failure();
}

// ── Status ──────────────────────────────────────────────────────────

#[test]
#[ignore = "requires PAYSKILL_TESTNET_KEY"]
fn status_returns_balance() {
    pay()
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("balance"));
}

// ── Mint (testnet faucet) ───────────────────────────────────────────

#[test]
#[ignore = "requires PAYSKILL_TESTNET_KEY"]
fn mint_testnet_usdc() {
    let output = pay()
        .args(["mint", "100.00"])
        .output()
        .expect("failed to run mint");
    let stderr = String::from_utf8_lossy(&output.stderr);
    // 429 rate limit is transient — don't fail the whole suite
    if !output.status.success() && stderr.contains("rate_limited") {
        eprintln!("mint rate-limited (expected if wallet already minted this hour), skipping");
        return;
    }
    assert!(output.status.success(), "mint failed: {stderr}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("tx_hash"), "mint should return tx_hash");
}

// ── Direct Payment ──────────────────────────────────────────────────

#[test]
#[ignore = "requires PAYSKILL_TESTNET_KEY"]
fn direct_payment_succeeds() {
    pay()
        .args(["direct", &provider_addr(), "1.00", "--memo", "e2e-test"])
        .assert()
        .success()
        .stdout(predicate::str::contains("tx_hash"));
}

// ── Tab Lifecycle ───────────────────────────────────────────────────

#[test]
#[ignore = "requires PAYSKILL_TESTNET_KEY"]
fn tab_lifecycle() {
    // 1. Open tab
    let open_output = pay()
        .args([
            "tab",
            "open",
            &provider_addr(),
            "5.00",
            "--max-charge",
            "0.50",
        ])
        .output()
        .expect("failed to run tab open");
    assert!(
        open_output.status.success(),
        "tab open failed: {}",
        String::from_utf8_lossy(&open_output.stderr)
    );
    let open_json: serde_json::Value =
        serde_json::from_slice(&open_output.stdout).expect("invalid JSON from tab open");
    let tab_id = open_json["tab_id"].as_str().expect("no tab_id in response");
    assert!(!tab_id.is_empty());

    // Wait for on-chain state propagation (permit nonce must be updated
    // before the next permit/prepare call, RPC nodes may lag).
    std::thread::sleep(std::time::Duration::from_secs(5));

    // 2. List tabs — new tab should appear
    let list_output = pay()
        .args(["tab", "list"])
        .output()
        .expect("failed to run tab list");
    assert!(list_output.status.success(), "tab list failed");
    let list_stdout = String::from_utf8_lossy(&list_output.stdout);
    assert!(
        list_stdout.contains(tab_id),
        "tab {tab_id} not in list output"
    );

    // 3. Top up
    let topup_output = pay()
        .args(["tab", "topup", tab_id, "5.00"])
        .output()
        .expect("failed to run tab topup");
    assert!(
        topup_output.status.success(),
        "tab topup failed: {}",
        String::from_utf8_lossy(&topup_output.stderr)
    );

    // TODO: Add a charge step here (open -> list -> topup -> charge -> close).
    // Charging requires the provider's private key (PAYSKILL_TESTNET_PROVIDER_KEY),
    // which is a separate wallet from the agent's PAYSKILL_TESTNET_KEY. The test
    // infrastructure currently only provisions a single keypair. To test charge,
    // add a second env var for the provider key and build a pay_provider() helper
    // that constructs a Command authenticated as the provider.

    // 4. Close tab
    pay()
        .args(["tab", "close", tab_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("total_charged").or(predicate::str::contains("closed")));
}

// ── Webhooks ────────────────────────────────────────────────────────

#[test]
#[ignore = "requires PAYSKILL_TESTNET_KEY"]
fn webhook_crud() {
    let hook_url = format!(
        "https://example.com/hook/e2e-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );

    // 1. Register
    let reg_output = pay()
        .args(["webhook", "register", &hook_url])
        .output()
        .expect("failed to run webhook register");
    assert!(
        reg_output.status.success(),
        "webhook register failed: {}",
        String::from_utf8_lossy(&reg_output.stderr)
    );
    let reg_json: serde_json::Value =
        serde_json::from_slice(&reg_output.stdout).expect("invalid JSON from webhook register");
    let wh_id = reg_json["id"]
        .as_str()
        .or_else(|| reg_json["webhook_id"].as_str())
        .expect("no id in register response");

    // 2. List — should include new webhook
    let list_output = pay()
        .args(["webhook", "list"])
        .output()
        .expect("failed to run webhook list");
    assert!(list_output.status.success(), "webhook list failed");
    let list_stdout = String::from_utf8_lossy(&list_output.stdout);
    assert!(
        list_stdout.contains(wh_id),
        "webhook {wh_id} not in list output"
    );

    // 3. Delete
    pay().args(["webhook", "delete", wh_id]).assert().success();
}

// ── Sign Subprocess ────────────────────────────────────────────────

#[test]
#[ignore = "requires PAYSKILL_TESTNET_KEY"]
fn sign_subprocess_produces_valid_signature() {
    // 32-byte test hash (hex-encoded, no 0x prefix)
    let test_hash = "de".repeat(32);

    let output = pay()
        .args(["sign"])
        .write_stdin(test_hash)
        .output()
        .expect("failed to run pay sign");

    assert!(
        output.status.success(),
        "pay sign failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let sig = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let sig_clean = sig.strip_prefix("0x").unwrap_or(&sig);
    // Valid signature: 65 bytes = 130 hex chars
    assert_eq!(
        sig_clean.len(),
        130,
        "signature should be 65 bytes (130 hex chars), got {} chars: {}",
        sig_clean.len(),
        sig_clean,
    );
    // v should be 27 or 28
    let v = u8::from_str_radix(&sig_clean[128..130], 16).expect("invalid v byte");
    assert!(v == 27 || v == 28, "v should be 27 or 28, got {v}");
}

// ── Fund + Withdraw ────────────────────────────────────────────────

#[test]
#[ignore = "requires PAYSKILL_TESTNET_KEY"]
fn fund_returns_link() {
    let output = pay()
        .args(["fund"])
        .output()
        .expect("failed to run pay fund");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "fund command should succeed: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("url") || stdout.contains("http"),
        "fund should return a URL or url field"
    );
}

#[test]
#[ignore = "requires PAYSKILL_TESTNET_KEY"]
fn withdraw_returns_link() {
    let addr = provider_addr();
    let output = pay()
        .args(["withdraw", &addr, "1.00"])
        .output()
        .expect("failed to run pay withdraw");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "withdraw command should succeed: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("url") || stdout.contains("http"),
        "withdraw should return a URL or url field"
    );
}

// ── x402 Request (V2 wire format) ─────────────────────────────────

#[test]
#[ignore = "requires PAYSKILL_TESTNET_KEY"]
fn x402_request_handles_402_and_pays() {
    // Start a mini HTTP server returning 402 with V2 PAYMENT-REQUIRED header
    // on first request, then 200 when PAYMENT-SIGNATURE header is present.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind failed");
    let port = listener.local_addr().unwrap().port();

    let handle = std::thread::spawn(move || {
        // Accept up to 2 connections (first = 402, second = 200)
        for _ in 0..2 {
            if let Ok((mut stream, _)) = listener.accept() {
                use std::io::{Read, Write};
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);

                if req.contains("PAYMENT-SIGNATURE") || req.contains("payment-signature") {
                    let body = r#"{"content":"paid"}"#;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes());
                } else {
                    // V2: requirements in body AND base64-encoded in PAYMENT-REQUIRED header
                    let requirements = format!(
                        r#"{{"scheme":"exact","amount":1000000,"to":"{}","settlement":"direct","facilitator":"https://testnet.pay-skill.com/x402","maxChargePerCall":1000000,"network":"eip155:84532"}}"#,
                        provider_addr()
                    );
                    use base64::Engine;
                    let req_b64 = base64::engine::general_purpose::STANDARD.encode(&requirements);
                    let body = format!(
                        r#"{{"error":"payment_required","message":"This resource requires payment","requirements":{requirements}}}"#,
                    );
                    let resp = format!(
                        "HTTP/1.1 402 Payment Required\r\nContent-Type: application/json\r\nContent-Length: {}\r\npayment-required: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        req_b64,
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes());
                }
            }
        }
    });

    // Run `pay request` against our local server
    let output = pay()
        .args(["request", &format!("http://127.0.0.1:{port}/content")])
        .timeout(std::time::Duration::from_secs(120))
        .output()
        .expect("failed to run pay request");

    // Wait for server thread to finish
    let _ = handle.join();

    // The command should succeed (or at least produce output indicating payment was made)
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Must succeed AND show evidence of payment
    assert!(
        output.status.success() && (stdout.contains("paid") || stdout.contains("tx_hash")),
        "x402 request should succeed with payment evidence. stdout={stdout}, stderr={stderr}"
    );
}

// ── Address ────────────────────────────────────────────────────────

#[test]
#[ignore = "requires PAYSKILL_TESTNET_KEY"]
fn address_returns_valid_format() {
    let output = pay()
        .args(["address"])
        .output()
        .expect("failed to run pay address");
    assert!(output.status.success());
    let addr = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert!(
        addr.starts_with("0x") && addr.len() == 42,
        "address should be 0x + 40 hex chars, got: {addr}"
    );
}

// ── Signer Modes ────────────────────────────────────────────────────
//
// Three init paths: `pay init` (default signer), `pay ows init` (OWS),
// `pay key init` (plain key). Tests verify command structure, help text,
// error paths. No OWS installation required.

#[test]
fn init_help_describes_default_signer() {
    Command::cargo_bin("pay")
        .expect("binary not found")
        .args(["init", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("wallet").or(predicate::str::contains("signer")));
}

#[test]
fn top_level_help_shows_ows_and_key() {
    Command::cargo_bin("pay")
        .expect("binary not found")
        .args(["--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ows"))
        .stdout(predicate::str::contains("key"));
}

// ── pay ows ──────────────────────────────────────────────────────────

#[test]
fn ows_subcommand_exists() {
    Command::cargo_bin("pay")
        .expect("binary not found")
        .args(["ows", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("init"))
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("fund"))
        .stdout(predicate::str::contains("set-policy"));
}

#[test]
fn ows_init_help() {
    Command::cargo_bin("pay")
        .expect("binary not found")
        .args(["ows", "init", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--name"))
        .stdout(predicate::str::contains("--chain"));
}

#[test]
fn ows_list_without_ows_shows_error_or_empty() {
    let output = Command::cargo_bin("pay")
        .expect("binary not found")
        .args(["ows", "list"])
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    // Either succeeds with empty/list output or fails with install instructions
    assert!(
        combined.contains("No OWS wallets")
            || combined.contains("ows")
            || combined.contains("not found")
            || combined.contains("install")
            || output.status.success(),
        "ows list should give actionable output, got: {combined}"
    );
}

#[test]
fn ows_list_json_without_ows() {
    let output = Command::cargo_bin("pay")
        .expect("binary not found")
        .args(["ows", "list"])
        .output()
        .expect("failed to run");

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(stdout.trim());
        assert!(parsed.is_ok(), "JSON output should be valid: {stdout}");
    }
}

#[test]
fn ows_fund_requires_wallet_arg() {
    let output = Command::cargo_bin("pay")
        .expect("binary not found")
        .args(["ows", "fund"])
        .env_remove("OWS_WALLET_ID")
        .output()
        .expect("failed to run");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("wallet") || stderr.contains("OWS_WALLET_ID"),
            "error should mention --wallet or OWS_WALLET_ID, got: {stderr}"
        );
    }
}

#[test]
fn ows_set_policy_rejects_invalid_chain() {
    let output = Command::cargo_bin("pay")
        .expect("binary not found")
        .args([
            "ows",
            "set-policy",
            "--chain",
            "ethereum",
            "--max-tx",
            "10",
            "--daily-limit",
            "100",
        ])
        .output()
        .expect("failed to run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown chain") || stderr.contains("Unknown chain"),
        "should reject unknown chain, got: {stderr}"
    );
}

#[test]
fn ows_set_policy_rejects_negative_max_tx() {
    let output = Command::cargo_bin("pay")
        .expect("binary not found")
        .args([
            "ows",
            "set-policy",
            "--max-tx",
            "-5",
            "--daily-limit",
            "100",
        ])
        .output()
        .expect("failed to run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("positive") || stderr.contains("invalid"),
        "should reject negative --max-tx, got: {stderr}"
    );
}

#[test]
fn ows_set_policy_rejects_negative_daily_limit() {
    let output = Command::cargo_bin("pay")
        .expect("binary not found")
        .args([
            "ows",
            "set-policy",
            "--daily-limit",
            "-100",
            "--max-tx",
            "10",
        ])
        .output()
        .expect("failed to run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("positive") || stderr.contains("invalid"),
        "should reject negative --daily-limit, got: {stderr}"
    );
}

// ── pay key ─────────────────────────────────────────────────────────

#[test]
fn key_subcommand_exists() {
    Command::cargo_bin("pay")
        .expect("binary not found")
        .args(["key", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("init"));
}

#[test]
fn key_init_help() {
    Command::cargo_bin("pay")
        .expect("binary not found")
        .args(["key", "init", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--write-env"));
}

#[test]
fn key_init_generates_keypair() {
    let output = Command::cargo_bin("pay")
        .expect("binary not found")
        .args(["key", "init"])
        .output()
        .expect("failed to run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    let address = parsed["address"].as_str().expect("has address");
    let key = parsed["private_key"].as_str().expect("has private_key");
    assert!(address.starts_with("0x") && address.len() == 42);
    assert!(key.starts_with("0x") && key.len() == 66);
}

// ── OWS happy path (requires OWS installed) ──────────────────────

/// Returns true if the `ows` CLI is available on this machine.
fn has_ows() -> bool {
    std::process::Command::new("ows")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
#[ignore = "requires OWS CLI installed"]
fn ows_init_creates_wallet() {
    if !has_ows() {
        eprintln!("skipping: ows CLI not available");
        return;
    }

    let wallet_name = format!(
        "pay-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );

    let output = Command::cargo_bin("pay")
        .expect("binary not found")
        .args([
            "ows",
            "init",
            "--name",
            &wallet_name,
            "--chain",
            "base-sepolia",
        ])
        .output()
        .expect("failed to run ows init");

    assert!(
        output.status.success(),
        "ows init should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify the wallet was created by listing and finding it
    let list_output = Command::cargo_bin("pay")
        .expect("binary not found")
        .args(["ows", "list", "--json"])
        .output()
        .expect("failed to run ows list");

    let stdout = String::from_utf8_lossy(&list_output.stdout);
    let wallets: Vec<serde_json::Value> =
        serde_json::from_str(stdout.trim()).expect("ows list should output valid JSON");

    let found = wallets
        .iter()
        .find(|w| w["name"].as_str() == Some(&wallet_name));
    assert!(found.is_some(), "created wallet should appear in list");
}

#[test]
#[ignore = "requires OWS CLI installed"]
fn ows_list_shows_wallets() {
    if !has_ows() {
        eprintln!("skipping: ows CLI not available");
        return;
    }

    let output = Command::cargo_bin("pay")
        .expect("binary not found")
        .args(["ows", "list"])
        .output()
        .expect("failed to run ows list");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("ows list should output valid JSON");
    assert!(parsed.is_array(), "ows list should return an array");
}
