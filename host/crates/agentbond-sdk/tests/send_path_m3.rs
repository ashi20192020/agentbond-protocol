use std::time::{Duration, Instant};

use agentbond_sdk::{
    ConfirmPolicy, HttpChainReader, MAINNET_GENESIS_HASH, plan_set_paused, program_id,
    simulate_and_send_plan,
};
use serde_json::json;
use solana_hash::Hash;
use solana_keypair::Keypair;
use solana_signer::Signer;
use wiremock::matchers::{body_partial_json, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn local_genesis() -> &'static str {
    "LocalGenesisHash111111111111111111111111111"
}

fn rpc_ok(result: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": result
    }))
}

async fn mount_clock(server: &MockServer) {
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"getSlot"})))
        .respond_with(rpc_ok(json!(10)))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"getBlockTime"})))
        .respond_with(rpc_ok(json!(1_700_000_000i64)))
        .mount(server)
        .await;
}

async fn mount_blockhash(server: &MockServer) {
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"getLatestBlockhash"})))
        .respond_with(rpc_ok(json!({
            "value": {
                "blockhash": Hash::new_from_array([3u8; 32]).to_string(),
                "lastValidBlockHeight": 100
            }
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn send_path_success_simulates_before_submit() {
    let server = MockServer::start().await;
    let signature = "5".repeat(64);
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"getGenesisHash"})))
        .respond_with(rpc_ok(json!(local_genesis())))
        .mount(&server)
        .await;
    mount_clock(&server).await;
    mount_blockhash(&server).await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"simulateTransaction"})))
        .respond_with(rpc_ok(json!({
            "value": { "err": null, "logs": ["ok"] }
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"sendTransaction"})))
        .respond_with(rpc_ok(json!(signature)))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"getSignatureStatuses"})))
        .respond_with(rpc_ok(json!({
            "value": [{
                "confirmationStatus": "confirmed",
                "err": null
            }]
        })))
        .mount(&server)
        .await;

    let payer = Keypair::new();
    let program = program_id();
    let plan = plan_set_paused(&program, &payer.pubkey(), true).expect("plan");
    let rpc = HttpChainReader::new(server.uri(), Duration::from_secs(2)).expect("rpc");

    let result = simulate_and_send_plan(
        &rpc,
        &plan,
        &program,
        &payer,
        &[],
        false,
        ConfirmPolicy::fast(Duration::from_millis(200)),
    )
    .await
    .expect("send");

    assert_eq!(result.signature, signature);
    assert_eq!(result.status, "confirmed");
    let secret = bs58::encode(payer.to_bytes()).into_string();
    let dump = format!("{result:?}");
    assert!(!dump.contains(&secret));
}

#[tokio::test]
async fn simulation_failure_never_sends() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"getGenesisHash"})))
        .respond_with(rpc_ok(json!(local_genesis())))
        .mount(&server)
        .await;
    mount_clock(&server).await;
    mount_blockhash(&server).await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"simulateTransaction"})))
        .respond_with(rpc_ok(json!({
            "value": { "err": {"InstructionError":[0,"Custom"]}, "logs": [] }
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"sendTransaction"})))
        .respond_with(rpc_ok(json!("should-not-send")))
        .expect(0)
        .mount(&server)
        .await;

    let payer = Keypair::new();
    let program = program_id();
    let plan = plan_set_paused(&program, &payer.pubkey(), false).expect("plan");
    let rpc = HttpChainReader::new(server.uri(), Duration::from_secs(2)).expect("rpc");
    let err = simulate_and_send_plan(
        &rpc,
        &plan,
        &program,
        &payer,
        &[],
        false,
        ConfirmPolicy::fast(Duration::from_millis(50)),
    )
    .await
    .expect_err("sim fail");
    assert!(err.to_string().contains("simulation"));
}

#[tokio::test]
async fn unsupported_program_missing_signer_expired_mainnet() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"getGenesisHash"})))
        .respond_with(rpc_ok(json!(local_genesis())))
        .mount(&server)
        .await;
    mount_clock(&server).await;
    mount_blockhash(&server).await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"simulateTransaction"})))
        .respond_with(rpc_ok(json!({"value":{"err":null}})))
        .expect(0)
        .mount(&server)
        .await;

    let payer = Keypair::new();
    let program = program_id();
    let plan = plan_set_paused(&program, &payer.pubkey(), true).expect("plan");
    let rpc = HttpChainReader::new(server.uri(), Duration::from_secs(2)).expect("rpc");

    let wrong_program = solana_pubkey::Pubkey::new_from_array([9u8; 32]);
    let err = simulate_and_send_plan(
        &rpc,
        &plan,
        &wrong_program,
        &payer,
        &[],
        false,
        ConfirmPolicy::fast(Duration::from_millis(50)),
    )
    .await
    .expect_err("program");
    assert!(err.to_string().contains("program"));

    let other = Keypair::new();
    let err = simulate_and_send_plan(
        &rpc,
        &plan,
        &program,
        &other,
        &[],
        false,
        ConfirmPolicy::fast(Duration::from_millis(50)),
    )
    .await
    .expect_err("signer");
    assert!(err.to_string().contains("missing required signer"));

    let mut expired = plan.clone();
    expired.expires_at = Some(1_600_000_000);
    let err = simulate_and_send_plan(
        &rpc,
        &expired,
        &program,
        &payer,
        &[],
        false,
        ConfirmPolicy::fast(Duration::from_millis(50)),
    )
    .await
    .expect_err("expired");
    assert!(err.to_string().contains("expired"));

    let mainnet_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"getGenesisHash"})))
        .respond_with(rpc_ok(json!(MAINNET_GENESIS_HASH)))
        .mount(&mainnet_server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"simulateTransaction"})))
        .respond_with(rpc_ok(json!({"value":{"err":null}})))
        .expect(0)
        .mount(&mainnet_server)
        .await;
    let rpc = HttpChainReader::new(mainnet_server.uri(), Duration::from_secs(2)).expect("rpc");
    let err = simulate_and_send_plan(
        &rpc,
        &plan,
        &program,
        &payer,
        &[],
        false,
        ConfirmPolicy::fast(Duration::from_millis(50)),
    )
    .await
    .expect_err("mainnet");
    assert!(err.to_string().contains("mainnet"));
}

#[tokio::test]
async fn send_error_confirm_error_and_bounded_timeout() {
    let payer = Keypair::new();
    let program = program_id();
    let plan = plan_set_paused(&program, &payer.pubkey(), true).expect("plan");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"getGenesisHash"})))
        .respond_with(rpc_ok(json!(local_genesis())))
        .mount(&server)
        .await;
    mount_clock(&server).await;
    mount_blockhash(&server).await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"simulateTransaction"})))
        .respond_with(rpc_ok(json!({"value":{"err":null,"logs":[]}})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"sendTransaction"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"send failed"}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let rpc = HttpChainReader::new(server.uri(), Duration::from_secs(2)).expect("rpc");
    let err = simulate_and_send_plan(
        &rpc,
        &plan,
        &program,
        &payer,
        &[],
        false,
        ConfirmPolicy::fast(Duration::from_millis(50)),
    )
    .await
    .expect_err("send");
    assert!(err.to_string().contains("send") || err.to_string().contains("-32000"));

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"getGenesisHash"})))
        .respond_with(rpc_ok(json!(local_genesis())))
        .mount(&server)
        .await;
    mount_clock(&server).await;
    mount_blockhash(&server).await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"simulateTransaction"})))
        .respond_with(rpc_ok(json!({"value":{"err":null}})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"sendTransaction"})))
        .respond_with(rpc_ok(json!("sig-err")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"getSignatureStatuses"})))
        .respond_with(rpc_ok(json!({
            "value": [{
                "confirmationStatus": "confirmed",
                "err": {"InstructionError":[0,"Custom"]}
            }]
        })))
        .mount(&server)
        .await;
    let rpc = HttpChainReader::new(server.uri(), Duration::from_secs(2)).expect("rpc");
    let err = simulate_and_send_plan(
        &rpc,
        &plan,
        &program,
        &payer,
        &[],
        false,
        ConfirmPolicy::fast(Duration::from_millis(200)),
    )
    .await
    .expect_err("tx err");
    assert!(err.to_string().contains("transaction failed"));

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"getGenesisHash"})))
        .respond_with(rpc_ok(json!(local_genesis())))
        .mount(&server)
        .await;
    mount_clock(&server).await;
    mount_blockhash(&server).await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"simulateTransaction"})))
        .respond_with(rpc_ok(json!({"value":{"err":null}})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"sendTransaction"})))
        .respond_with(rpc_ok(json!("sigtimeout")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method":"getSignatureStatuses"})))
        .respond_with(rpc_ok(json!({ "value": [null] })))
        .mount(&server)
        .await;
    let rpc = HttpChainReader::new(server.uri(), Duration::from_secs(2)).expect("rpc");
    let started = Instant::now();
    let err = simulate_and_send_plan(
        &rpc,
        &plan,
        &program,
        &payer,
        &[],
        false,
        ConfirmPolicy::fast(Duration::from_millis(30)),
    )
    .await
    .expect_err("timeout");
    assert!(err.to_string().contains("confirmation deadline"));
    assert!(started.elapsed() < Duration::from_secs(2));
}
