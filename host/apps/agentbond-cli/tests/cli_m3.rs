//! Milestone 3 CLI integration tests (offline; no network).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use assert_cmd::assert::OutputAssertExt;
use assert_cmd::cargo::CommandCargoExt;
use predicates::prelude::*;
use tempfile::tempdir;

fn bin() -> Command {
    Command::cargo_bin("agentbond").expect("binary agentbond")
}

#[test]
fn help_output() {
    bin()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("AgentBond CLI"))
        .stdout(predicate::str::contains("address"))
        .stdout(predicate::str::contains("plan"))
        .stdout(predicate::str::contains("send"));
}

#[test]
fn invalid_address_rejected() {
    bin()
        .args(["address", "provider", "not-a-valid-pubkey"])
        .assert()
        .failure();
}

#[test]
fn invalid_amount_rejected() {
    bin()
        .args([
            "plan",
            "create-job",
            "--buyer",
            "11111111111111111111111111111111",
            "--provider",
            "11111111111111111111111111111112",
            "--nonce",
            "1",
            "--amount",
            "0",
            "--request-hash",
            &"09".repeat(32),
            "--fund-deadline",
            "100",
            "--accept-deadline",
            "200",
            "--work-deadline",
            "300",
            "--auto-settle-deadline",
            "400",
            "--now",
            "50",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid amount"));
}

#[test]
fn invalid_deadline_order_rejected() {
    bin()
        .args([
            "plan",
            "create-job",
            "--buyer",
            "11111111111111111111111111111111",
            "--provider",
            "11111111111111111111111111111112",
            "--nonce",
            "1",
            "--amount",
            "100",
            "--request-hash",
            &"09".repeat(32),
            "--fund-deadline",
            "400",
            "--accept-deadline",
            "300",
            "--work-deadline",
            "200",
            "--auto-settle-deadline",
            "100",
            "--now",
            "50",
        ])
        .assert()
        .failure();
}

#[test]
fn json_plan_output() {
    let assert = bin()
        .args([
            "--json",
            "plan",
            "create-job",
            "--buyer",
            "11111111111111111111111111111111",
            "--provider",
            "11111111111111111111111111111112",
            "--nonce",
            "1",
            "--amount",
            "100",
            "--request-hash",
            &"09".repeat(32),
            "--fund-deadline",
            "100",
            "--accept-deadline",
            "200",
            "--work-deadline",
            "300",
            "--auto-settle-deadline",
            "400",
            "--now",
            "50",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(assert.get_output().stdout.as_slice());
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("json plan");
    assert_eq!(value["action"], "create_job");
    assert!(!value["instructions"].as_array().expect("ixs").is_empty());
    assert!(!stdout.to_ascii_lowercase().contains("private"));
    assert!(!stdout.to_ascii_lowercase().contains("secret"));
}

#[test]
fn mainnet_safety_guard_via_send() {
    let dir = tempdir().expect("tempdir");
    let plan_path = dir.path().join("plan.json");
    let payer_path = dir.path().join("payer.json");

    // Minimal valid plan JSON (empty instructions ok for guard test — fails earlier).
    fs::write(
        &plan_path,
        r#"{"action":"create_job","program_id":"11111111111111111111111111111111","instructions":[],"required_signers":[]}"#,
    )
    .expect("plan");
    // 64-byte solana keypair JSON with restrictive perms.
    let key_bytes: Vec<u8> = (0u8..64).collect();
    fs::write(
        &payer_path,
        serde_json::to_string(&key_bytes).expect("key json"),
    )
    .expect("payer");
    let mut perms = fs::metadata(&payer_path).expect("meta").permissions();
    perms.set_mode(0o600);
    fs::set_permissions(&payer_path, perms).expect("chmod");

    bin()
        .args([
            "send",
            "--rpc-url",
            "https://api.mainnet-beta.solana.com",
            "--payer",
            payer_path.to_str().expect("path"),
            "--plan",
            plan_path.to_str().expect("path"),
            "--yes",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("mainnet blocked"));
}

#[test]
fn receipt_sign_never_prints_secret_material() {
    let dir = tempdir().expect("tempdir");
    let receipt_path = dir.path().join("receipt.json");
    let key_path = dir.path().join("key.bin");

    bin()
        .args([
            "receipt",
            "create",
            "--file",
            receipt_path.to_str().expect("p"),
        ])
        .assert()
        .success();

    // 32-byte ed25519 seed with secure perms.
    fs::write(&key_path, [7u8; 32]).expect("key");
    let mut perms = fs::metadata(&key_path).expect("meta").permissions();
    perms.set_mode(0o600);
    fs::set_permissions(&key_path, perms).expect("chmod");

    let assert = bin()
        .args([
            "receipt",
            "sign",
            "--file",
            receipt_path.to_str().expect("p"),
            "--key-file",
            key_path.to_str().expect("p"),
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(assert.get_output().stdout.as_slice());
    assert!(stdout.contains("signature"));
    assert!(stdout.contains("public_key"));
    assert!(!stdout.to_ascii_lowercase().contains("secret"));
    assert!(!stdout.to_ascii_lowercase().contains("private"));
    // Raw seed bytes must not appear.
    assert!(!stdout.contains(&"07".repeat(32)));
}
