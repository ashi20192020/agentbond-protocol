use std::sync::Arc;

use agentbond_app::{AppConfig, ServiceCatalog, ServiceEntry};
use agentbond_mcp::{AgentBondMcp, schemas_forbid_private_keys};
use agentbond_sdk::{MockChainReader, program_id};
use rmcp::handler::server::ServerHandler;
use serde_json::json;

fn cfg() -> AppConfig {
    AppConfig {
        program_id: program_id().to_string(),
        rpc_url: "http://127.0.0.1:8899".into(),
        genesis_hash: "07".repeat(32),
        settlement_mint: "11111111111111111111111111111111".into(),
        token_program: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".into(),
        facilitator_url: "http://127.0.0.1:9090".into(),
        merchant_pay_to: "11111111111111111111111111111112".into(),
        x402_fee_payer: "11111111111111111111111111111113".into(),
        x402_amount: "1000".into(),
        x402_network: "solana:localnet".into(),
        request_timeout_ms: 5000,
        max_request_bytes: 65536,
        bind_address: "127.0.0.1:8080".into(),
        catalog_path: "config/example.catalog.json".into(),
    }
}

fn catalog() -> ServiceCatalog {
    ServiceCatalog::from_entries(vec![ServiceEntry {
        service_id: "hash-demo".into(),
        provider: "11111111111111111111111111111113".into(),
        name: "Hash Demo".into(),
        description: "demo".into(),
        request_schema: "demo".into(),
        x402_demo_route: Some("/v1/x402/services/hash-demo/invoke".into()),
    }])
    .expect("catalog")
}

#[tokio::test]
async fn tool_schemas_and_numeric_args() {
    let reader = MockChainReader::new();
    reader.set_timestamp(1_700_000_000).await;
    let server = AgentBondMcp {
        cfg: cfg(),
        catalog: catalog(),
        reader: Arc::new(reader),
    };
    let info = server.get_info();
    let text = format!("{info:?}").to_ascii_lowercase();
    assert!(text.contains("never signs") || text.contains("never sign"));
    assert!(!text.contains("submits transactions") || text.contains("never"));
    assert!(schemas_forbid_private_keys(&AgentBondMcp::tools()));
    assert_eq!(AgentBondMcp::tools().len(), 9);

    let listed = server.dispatch("discover_services", json!({})).await;
    assert_eq!(listed.is_error, Some(false));

    let create = server
        .dispatch(
            "build_create_job",
            json!({
                "buyer": "11111111111111111111111111111111",
                "provider": "11111111111111111111111111111112",
                "job_nonce": 1,
                "amount": 1000,
                "request_hash_hex": "09".repeat(32),
                "fund_deadline": 1_700_000_100i64,
                "accept_deadline": 1_700_000_200i64,
                "work_deadline": 1_700_000_300i64,
                "auto_settle_deadline": 1_700_000_400i64
            }),
        )
        .await;
    // Mock clock available → structured plan or clear error, never private key.
    let dump = format!("{create:?}").to_ascii_lowercase();
    assert!(!dump.contains("private_key"));

    let bad_type = server
        .dispatch(
            "build_fund_job",
            json!({
                "buyer": "11111111111111111111111111111111",
                "provider": "11111111111111111111111111111112",
                "job_nonce": "not-int"
            }),
        )
        .await;
    assert_eq!(bad_type.is_error, Some(true));

    let unknown = server.dispatch("nope", json!({})).await;
    assert_eq!(unknown.is_error, Some(true));
}
