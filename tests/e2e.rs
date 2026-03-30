//! E2E acceptance tests — run against live testnet.
//!
//! Skipped unless `PAYSKILL_TESTNET_KEY` is set. These build the real CLI
//! binary and exercise it against the testnet server.
//!
//! Run:
//!   PAYSKILL_TESTNET_KEY=0xdead... PAYSKILL_API_URL=https://testnet.pay-skill.com/api/v1 \
//!     cargo test --test e2e -- --ignored

use assert_cmd::Command;
use predicates::prelude::*;
use std::env;
use std::sync::Once;

static INIT: Once = Once::new();

/// Ensure `pay init` has been run (idempotent, runs once per test suite).
fn ensure_init() {
    INIT.call_once(|| {
        Command::cargo_bin("pay")
            .expect("binary not found")
            .arg("init")
            .assert()
            .success();
    });
}

/// Return true if testnet env vars are set.
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
    env::var("PAYSKILL_TESTNET_PROVIDER").unwrap_or_else(|_| format!("0x{}", "b2".repeat(20)))
}

/// Build a `pay` command pre-configured for testnet.
fn pay() -> Command {
    ensure_init();
    let mut cmd = Command::cargo_bin("pay").expect("binary not found");
    cmd.arg("--api-url").arg(testnet_url());
    cmd
}

// ── Init ────────────────────────────────────────────────────────────

#[test]
fn init_creates_config() {
    ensure_init();
}

// ── Validation (no testnet key needed) ──────────────────────────────

#[test]
fn direct_rejects_bad_address() {
    pay()
        .args(["direct", "not-an-address", "1.00"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid address"));
}

#[test]
fn direct_rejects_below_minimum() {
    let addr = format!("0x{}", "a1".repeat(20));
    pay()
        .args(["direct", &addr, "0.50"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Minimum"));
}

#[test]
fn tab_open_rejects_below_minimum() {
    let addr = format!("0x{}", "a1".repeat(20));
    pay()
        .args(["tab", "open", &addr, "2.00", "--max-charge", "0.50"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Minimum"));
}

// ── Status ──────────────────────────────────────────────────────────

#[test]
#[ignore = "requires PAYSKILL_TESTNET_KEY"]
fn status_returns_balance() {
    if !has_testnet_key() {
        return;
    }
    pay()
        .args(["--json", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("balance"));
}

// ── Direct Payment ──────────────────────────────────────────────────

#[test]
#[ignore = "requires PAYSKILL_TESTNET_KEY"]
fn direct_payment_succeeds() {
    if !has_testnet_key() {
        return;
    }
    pay()
        .args([
            "--json",
            "direct",
            &provider_addr(),
            "1.00",
            "--memo",
            "e2e-test",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("tx_hash"));
}

// ── Tab Lifecycle ───────────────────────────────────────────────────

#[test]
#[ignore = "requires PAYSKILL_TESTNET_KEY"]
fn tab_lifecycle() {
    if !has_testnet_key() {
        return;
    }

    // 1. Open tab
    let open_output = pay()
        .args([
            "--json",
            "tab",
            "open",
            &provider_addr(),
            "5.00",
            "--max-charge",
            "0.50",
        ])
        .output()
        .expect("failed to run tab open");
    assert!(open_output.status.success(), "tab open failed");
    let open_json: serde_json::Value =
        serde_json::from_slice(&open_output.stdout).expect("invalid JSON from tab open");
    let tab_id = open_json["tab_id"].as_str().expect("no tab_id in response");
    assert!(!tab_id.is_empty());

    // 2. List tabs — new tab should appear
    let list_output = pay()
        .args(["--json", "tab", "list"])
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
        .args(["--json", "tab", "topup", tab_id, "5.00"])
        .output()
        .expect("failed to run tab topup");
    assert!(topup_output.status.success(), "tab topup failed");

    // 4. Close tab
    pay()
        .args(["--json", "tab", "close", tab_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("total_charged").or(predicate::str::contains("closed")));
}

// ── Webhooks ────────────────────────────────────────────────────────

#[test]
#[ignore = "requires PAYSKILL_TESTNET_KEY"]
fn webhook_crud() {
    if !has_testnet_key() {
        return;
    }

    let hook_url = format!(
        "https://example.com/hook/e2e-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );

    // 1. Register
    let reg_output = pay()
        .args(["--json", "webhook", "register", &hook_url])
        .output()
        .expect("failed to run webhook register");
    assert!(reg_output.status.success(), "webhook register failed");
    let reg_json: serde_json::Value =
        serde_json::from_slice(&reg_output.stdout).expect("invalid JSON from webhook register");
    let wh_id = reg_json["id"]
        .as_str()
        .or_else(|| reg_json["webhook_id"].as_str())
        .expect("no id in register response");

    // 2. List — should include new webhook
    let list_output = pay()
        .args(["--json", "webhook", "list"])
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

// ── Fund / Withdraw Links ───────────────────────────────────────────

#[test]
#[ignore = "requires PAYSKILL_TESTNET_KEY"]
fn fund_link() {
    if !has_testnet_key() {
        return;
    }
    pay()
        .args(["--json", "fund"])
        .assert()
        .success()
        .stdout(predicate::str::contains("url"));
}

#[test]
#[ignore = "requires PAYSKILL_TESTNET_KEY"]
fn withdraw_link() {
    if !has_testnet_key() {
        return;
    }
    let addr = format!("0x{}", "a1".repeat(20));
    pay()
        .args(["--json", "withdraw", &addr, "5.00"])
        .assert()
        .success()
        .stdout(predicate::str::contains("url"));
}
