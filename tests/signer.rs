//! Signer acceptance tests -- test the full signer system via CLI.
//!
//! These tests exercise `pay init`, `pay signer import`, `pay signer export`,
//! `pay sign`, and `pay address` using isolated temp directories.
//!
//! Run: cargo test --test signer

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::PathBuf;

const ANVIL_KEY: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const ANVIL_KEY_0X: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const ANVIL_ADDR: &str = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";
const TEST_PASSWORD: &str = "test-password-signer";

/// Build a `pay` command with isolated keys directory and HOME override.
fn pay_cmd(keys_dir: &std::path::Path, home_dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("pay").expect("binary not found");
    cmd.env("PAY_KEYS_DIR", keys_dir.to_str().unwrap());
    // Override HOME so config goes to temp dir too
    cmd.env("HOME", home_dir.to_str().unwrap());
    cmd.env("USERPROFILE", home_dir.to_str().unwrap());
    cmd
}

/// Create a temp dir pair (keys_dir, home_dir).
fn temp_dirs() -> (tempfile::TempDir, tempfile::TempDir, PathBuf) {
    let home = tempfile::TempDir::new().unwrap();
    let keys = tempfile::TempDir::new().unwrap();
    let keys_path = keys.path().to_path_buf();
    (home, keys, keys_path)
}

// -- pay init --no-keychain ---------------------------------------------------

#[test]
fn init_no_keychain_creates_enc_file() {
    let (home, _keys, keys_path) = temp_dirs();

    let mut cmd = pay_cmd(&keys_path, home.path());
    cmd.env("PAYSKILL_SIGNER_KEY", TEST_PASSWORD);
    cmd.args(["init", "--no-keychain"]);

    cmd.assert().success();

    // Verify .enc file was created
    let enc_path = keys_path.join("default.enc");
    assert!(enc_path.exists(), ".enc file should be created");

    // Verify it's valid JSON with scrypt format
    let contents = std::fs::read_to_string(&enc_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
    assert_eq!(parsed["version"], 2);
    assert_eq!(parsed["encryption"]["kdf"], "scrypt");
    assert_eq!(parsed["encryption"]["kdf_params"]["n"], 32768);
}

#[test]
fn init_no_keychain_address_works() {
    let (home, _keys, keys_path) = temp_dirs();

    // Init
    let mut cmd = pay_cmd(&keys_path, home.path());
    cmd.env("PAYSKILL_SIGNER_KEY", TEST_PASSWORD);
    cmd.args(["init", "--no-keychain"]);
    cmd.assert().success();

    // Address should work
    let mut cmd = pay_cmd(&keys_path, home.path());
    cmd.env("PAYSKILL_SIGNER_KEY", TEST_PASSWORD);
    cmd.args(["--plain", "address"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::starts_with("0x"));
}

#[test]
fn init_twice_is_idempotent() {
    let (home, _keys, keys_path) = temp_dirs();

    // First init
    let mut cmd = pay_cmd(&keys_path, home.path());
    cmd.env("PAYSKILL_SIGNER_KEY", TEST_PASSWORD);
    cmd.args(["init", "--no-keychain"]);
    cmd.assert().success();

    // Second init should succeed (idempotent)
    let mut cmd = pay_cmd(&keys_path, home.path());
    cmd.env("PAYSKILL_SIGNER_KEY", TEST_PASSWORD);
    cmd.args(["init", "--no-keychain"]);
    cmd.assert().success();
}

// -- pay signer import --------------------------------------------------------

#[test]
fn signer_import_no_keychain_correct_address() {
    let (home, _keys, keys_path) = temp_dirs();

    let mut cmd = pay_cmd(&keys_path, home.path());
    cmd.env("PAYSKILL_SIGNER_KEY", TEST_PASSWORD);
    cmd.args(["signer", "import", "--key", ANVIL_KEY, "--no-keychain"]);
    cmd.assert().success();

    // Verify address matches
    let mut cmd = pay_cmd(&keys_path, home.path());
    cmd.env("PAYSKILL_SIGNER_KEY", TEST_PASSWORD);
    cmd.args(["--plain", "address"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains(ANVIL_ADDR));
}

#[test]
fn signer_import_invalid_hex_fails() {
    let (home, _keys, keys_path) = temp_dirs();

    let mut cmd = pay_cmd(&keys_path, home.path());
    cmd.env("PAYSKILL_SIGNER_KEY", TEST_PASSWORD);
    cmd.args([
        "signer",
        "import",
        "--key",
        "not-hex-at-all",
        "--no-keychain",
    ]);
    cmd.assert().failure();
}

#[test]
fn signer_import_wrong_length_fails() {
    let (home, _keys, keys_path) = temp_dirs();

    let mut cmd = pay_cmd(&keys_path, home.path());
    cmd.env("PAYSKILL_SIGNER_KEY", TEST_PASSWORD);
    cmd.args(["signer", "import", "--key", "0xdeadbeef", "--no-keychain"]);
    cmd.assert().failure();
}

#[test]
fn signer_import_duplicate_fails() {
    let (home, _keys, keys_path) = temp_dirs();

    // First import
    let mut cmd = pay_cmd(&keys_path, home.path());
    cmd.env("PAYSKILL_SIGNER_KEY", TEST_PASSWORD);
    cmd.args(["signer", "import", "--key", ANVIL_KEY, "--no-keychain"]);
    cmd.assert().success();

    // Second import with same name should fail
    let mut cmd = pay_cmd(&keys_path, home.path());
    cmd.env("PAYSKILL_SIGNER_KEY", TEST_PASSWORD);
    cmd.args(["signer", "import", "--key", ANVIL_KEY, "--no-keychain"]);
    cmd.assert().failure();
}

// -- pay sign (subprocess protocol) -------------------------------------------

#[test]
fn sign_with_raw_hex_key() {
    let (home, _keys, keys_path) = temp_dirs();

    // pay sign uses resolve_key() which picks up raw hex from env
    let hash = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    let mut cmd = pay_cmd(&keys_path, home.path());
    cmd.env("PAYSKILL_SIGNER_KEY", ANVIL_KEY_0X);
    cmd.arg("sign");
    cmd.write_stdin(format!("{hash}\n"));

    let output = cmd.assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    // Signature should be 130 hex chars (65 bytes)
    assert_eq!(
        stdout.trim().len(),
        130,
        "signature should be 130 hex chars"
    );
}

#[test]
fn sign_with_password_enc_file() {
    let (home, _keys, keys_path) = temp_dirs();

    // Import key first
    let mut cmd = pay_cmd(&keys_path, home.path());
    cmd.env("PAYSKILL_SIGNER_KEY", TEST_PASSWORD);
    cmd.args(["signer", "import", "--key", ANVIL_KEY, "--no-keychain"]);
    cmd.assert().success();

    // Sign using password-based resolution
    let hash = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    let mut cmd = pay_cmd(&keys_path, home.path());
    cmd.env("PAYSKILL_SIGNER_KEY", TEST_PASSWORD);
    cmd.arg("sign");
    cmd.write_stdin(format!("{hash}\n"));

    let output = cmd.assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert_eq!(stdout.trim().len(), 130);
}

// -- Error cases --------------------------------------------------------------

#[test]
fn no_key_configured_shows_init_message() {
    let (home, _keys, keys_path) = temp_dirs();

    let mut cmd = pay_cmd(&keys_path, home.path());
    cmd.env_remove("PAYSKILL_SIGNER_KEY");
    cmd.arg("address");

    cmd.assert().failure().stderr(
        predicate::str::contains("pay init").or(predicate::str::contains("not initialized")),
    );
}

#[test]
fn wrong_password_fails_cleanly() {
    let (home, _keys, keys_path) = temp_dirs();

    // Import with one password
    let mut cmd = pay_cmd(&keys_path, home.path());
    cmd.env("PAYSKILL_SIGNER_KEY", TEST_PASSWORD);
    cmd.args(["signer", "import", "--key", ANVIL_KEY, "--no-keychain"]);
    cmd.assert().success();

    // Try to use with wrong password
    let mut cmd = pay_cmd(&keys_path, home.path());
    cmd.env("PAYSKILL_SIGNER_KEY", "wrong-password");
    cmd.arg("address");

    cmd.assert().failure();
}
