use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn help_lists_plan_commands() {
    let mut cmd = Command::cargo_bin("agentbond").expect("bin");
    cmd.arg("plan").arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("create-job"))
        .stdout(predicate::str::contains("fund-job"))
        .stdout(predicate::str::contains("submit-receipt"))
        .stdout(predicate::str::contains("slash-bond"));
}

#[test]
fn invalid_address_rejected() {
    let mut cmd = Command::cargo_bin("agentbond").expect("bin");
    cmd.args(["address", "provider", "not-a-key"]);
    cmd.assert().failure();
}

#[test]
fn create_job_plan_json() {
    let mut cmd = Command::cargo_bin("agentbond").expect("bin");
    cmd.args([
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
        "1000",
        "--request-hash",
        &"09".repeat(32),
        "--fund-deadline",
        "1700000100",
        "--accept-deadline",
        "1700000200",
        "--work-deadline",
        "1700000300",
        "--auto-settle-deadline",
        "1700000400",
        "--now",
        "1700000000",
    ]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"action\": \"create_job\""))
        .stdout(predicate::str::contains("private").not());
}

#[test]
fn receipt_create_requires_fields_no_defaults() {
    let dir = tempdir().expect("tmp");
    let file = dir.path().join("receipt.json");
    let mut cmd = Command::cargo_bin("agentbond").expect("bin");
    cmd.args(["receipt", "create", "--file", file.to_str().unwrap()]);
    cmd.assert().failure();
}

#[test]
fn send_requires_yes() {
    let dir = tempdir().expect("tmp");
    let plan = dir.path().join("plan.json");
    std::fs::write(
        &plan,
        r#"{"action":"set_paused","program_id":"iTRhr3SWUeJAjjSzeegGuGei5gDdCxaxhs93sotsCjo","instructions":[],"required_signers":[]}"#,
    )
    .expect("write");
    let mut cmd = Command::cargo_bin("agentbond").expect("bin");
    cmd.args([
        "send",
        "--rpc-url",
        "http://127.0.0.1:8899",
        "--payer",
        plan.to_str().unwrap(),
        "--plan",
        plan.to_str().unwrap(),
    ]);
    cmd.assert().failure();
}
