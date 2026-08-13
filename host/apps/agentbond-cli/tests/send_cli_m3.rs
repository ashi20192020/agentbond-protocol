use std::os::unix::fs::PermissionsExt;

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use solana_hash::Hash;
use solana_keypair::Keypair;
use solana_signer::Signer;
use tempfile::tempdir;
use tokio::runtime::Runtime;
use wiremock::matchers::{body_partial_json, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn rpc_ok(result: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": result
    }))
}

#[test]
fn send_cli_success_against_mock_rpc() {
    let rt = Runtime::new().expect("rt");
    rt.block_on(async {
        let server = MockServer::start().await;
        let signature = "AbCdEf1234567890".to_string();
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"method":"getGenesisHash"})))
            .respond_with(rpc_ok(json!("LocalGenesisHash111111111111111111111111111")))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"method":"getSlot"})))
            .respond_with(rpc_ok(json!(10)))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"method":"getBlockTime"})))
            .respond_with(rpc_ok(json!(1_700_000_000i64)))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"method":"getLatestBlockhash"})))
            .respond_with(rpc_ok(json!({
                "value": {
                    "blockhash": Hash::new_from_array([3u8; 32]).to_string(),
                    "lastValidBlockHeight": 100
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"method":"simulateTransaction"})))
            .respond_with(rpc_ok(json!({"value":{"err":null,"logs":[]}})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"method":"sendTransaction"})))
            .respond_with(rpc_ok(json!(signature)))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"method":"getSignatureStatuses"})))
            .respond_with(rpc_ok(json!({
                "value": [{"confirmationStatus":"confirmed","err":null}]
            })))
            .mount(&server)
            .await;

        let payer = Keypair::new();
        let program = agentbond_sdk::program_id();
        let plan = agentbond_sdk::plan_set_paused(&program, &payer.pubkey(), true).expect("plan");
        let dir = tempdir().expect("tmp");
        let plan_path = dir.path().join("plan.json");
        let key_path = dir.path().join("payer.json");
        std::fs::write(&plan_path, plan.to_json().expect("json")).expect("plan");
        let key_bytes = payer.to_bytes();
        std::fs::write(
            &key_path,
            serde_json::to_vec(&key_bytes.to_vec()).expect("key json"),
        )
        .expect("key");
        let mut perms = std::fs::metadata(&key_path).expect("meta").permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&key_path, perms).expect("chmod");

        let secret_b58 = bs58::encode(key_bytes).into_string();
        let rpc_url = server.uri();

        tokio::task::spawn_blocking(move || {
            let mut cmd = Command::cargo_bin("agentbond").expect("bin");
            cmd.args([
                "send",
                "--rpc-url",
                &rpc_url,
                "--payer",
                key_path.to_str().unwrap(),
                "--plan",
                plan_path.to_str().unwrap(),
                "--yes",
            ]);
            cmd.assert()
                .success()
                .stdout(predicate::str::contains("\"signature\""))
                .stdout(predicate::str::contains("confirmed"))
                .stdout(predicate::str::contains(&secret_b58).not())
                .stdout(predicate::str::contains("private").not());
        })
        .await
        .expect("join");
    });
}
