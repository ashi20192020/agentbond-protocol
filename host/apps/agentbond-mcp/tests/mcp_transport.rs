use std::collections::HashMap;
use std::sync::Arc;

use agentbond_app::{AppConfig, ServiceCatalog, ServiceEntry};
use agentbond_mcp::{AgentBondMcp, schemas_forbid_private_keys};
use agentbond_sdk::{AccountData, MockChainReader, job_pda, program_id, provider_pda};
use agentbond_types::{JobAccount, JobState, PROVIDER_STATUS_ACTIVE, ProviderAccount};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientInfo, ProtocolVersion};
use rmcp::service::RunningService;
use serde_json::{Map, Value, json};
use solana_pubkey::Pubkey;
use tokio::io::duplex;
use tracing_subscriber::util::SubscriberInitExt;

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

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn args_map(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => panic!("expected object args"),
    }
}

fn structured(result: &CallToolResult) -> Value {
    result
        .structured_content
        .clone()
        .expect("structured content")
}

async fn seed_reader() -> Arc<MockChainReader> {
    let reader = MockChainReader::new();
    reader.set_timestamp(1_700_000_000).await;
    let program = program_id();
    let buyer: Pubkey = "11111111111111111111111111111111".parse().expect("buyer");
    let provider: Pubkey = "11111111111111111111111111111112"
        .parse()
        .expect("provider");
    let provider_addr = provider_pda(&program, &provider).expect("pda").address;
    let provider_acc = ProviderAccount {
        bump: 1,
        status: PROVIDER_STATUS_ACTIVE,
        authority: provider.to_bytes(),
        execution_key_count: 0,
        execution_keys: [[0u8; 32]; 4],
    };
    reader
        .set_account(
            provider_addr,
            AccountData {
                owner: program,
                data: provider_acc.encode().expect("enc").to_vec(),
                lamports: 1,
            },
        )
        .await;
    let job_addr = job_pda(&program, &buyer, &provider, 1)
        .expect("job")
        .address;
    let job = JobAccount {
        bump: 1,
        state: JobState::Created,
        buyer: buyer.to_bytes(),
        provider: provider.to_bytes(),
        mint: Pubkey::default().to_bytes(),
        token_program: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
            .parse::<Pubkey>()
            .expect("token")
            .to_bytes(),
        amount: 1000,
        job_nonce: 1,
        fund_deadline: 1_700_000_100,
        accept_deadline: 1_700_000_200,
        work_deadline: 1_700_000_300,
        auto_settle_deadline: 1_700_000_400,
        receipt_digest: [0u8; 32],
        request_hash: [9u8; 32],
        locked_bond: 0,
        mint_decimals: 6,
    };
    reader
        .set_account(
            job_addr,
            AccountData {
                owner: program,
                data: job.encode().to_vec(),
                lamports: 1,
            },
        )
        .await;
    Arc::new(reader)
}

async fn start_pair(
    reader: Arc<MockChainReader>,
) -> (
    RunningService<rmcp::RoleClient, ClientInfo>,
    tokio::task::JoinHandle<anyhow::Result<()>>,
) {
    let (server_transport, client_transport) = duplex(64 * 1024);
    let server = AgentBondMcp {
        cfg: cfg(),
        catalog: catalog(),
        reader: reader as Arc<dyn agentbond_sdk::ChainReader>,
    };
    let server_task = tokio::spawn(async move {
        let running = server.serve(server_transport).await?;
        running.waiting().await?;
        Ok(())
    });
    let mut client_info = ClientInfo::default();
    client_info.protocol_version = ProtocolVersion::V_2026_07_28;
    let client = client_info
        .serve(client_transport)
        .await
        .expect("client init");
    (client, server_task)
}

async fn shutdown(
    client: RunningService<rmcp::RoleClient, ClientInfo>,
    server_task: tokio::task::JoinHandle<anyhow::Result<()>>,
) {
    client.cancel().await.expect("cancel client");
    let _ = server_task.await;
}

#[tokio::test]
async fn transport_init_list_and_all_tools() {
    let _guard = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter("info")
        .set_default();
    tracing::info!("mcp transport test log on stderr only");

    let reader = seed_reader().await;
    let (client, server_task) = start_pair(reader.clone()).await;

    let peer = client.peer_info().expect("peer info");
    assert_eq!(peer.protocol_version, ProtocolVersion::V_2026_07_28);

    let tools = client.list_all_tools().await.expect("list tools");
    let names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
    for expected in [
        "discover_services",
        "inspect_provider",
        "inspect_job",
        "build_create_job",
        "build_fund_job",
        "build_submit_receipt",
        "build_accept_work",
        "build_challenge",
        "build_timeout_resolution",
    ] {
        assert!(names.contains(&expected.to_string()), "missing {expected}");
    }
    assert!(schemas_forbid_private_keys(&tools));
    let schema_dump = format!("{tools:?}").to_ascii_lowercase();
    for banned in [
        "private_key",
        "privatekey",
        "seed_phrase",
        "seedphrase",
        "keypair",
        "keypair_path",
        "signing",
    ] {
        assert!(!schema_dump.contains(banned), "schema contains {banned}");
    }
    for tool in &tools {
        let text = format!("{:?}", tool.input_schema);
        assert!(
            !text.contains("\"job_nonce\": String(\"string\")"),
            "numeric fields must stay integers in schema"
        );
        let props = tool.input_schema.get("properties");
        if let Some(Value::Object(map)) = props {
            if let Some(nonce) = map.get("job_nonce") {
                assert_eq!(nonce["type"], "integer");
            }
            if let Some(amount) = map.get("amount") {
                assert_eq!(amount["type"], "integer");
            }
        }
    }

    let discovered = client
        .call_tool(CallToolRequestParams::new("discover_services").with_arguments(Map::new()))
        .await
        .expect("discover");
    assert_eq!(discovered.is_error, Some(false));
    let services = structured(&discovered);
    assert!(!services["services"].as_array().expect("arr").is_empty());

    let program = program_id();
    let buyer: Pubkey = "11111111111111111111111111111111".parse().expect("b");
    let provider: Pubkey = "11111111111111111111111111111112".parse().expect("p");
    let provider_addr = provider_pda(&program, &provider).expect("pda").address;
    let job_addr = job_pda(&program, &buyer, &provider, 1)
        .expect("job")
        .address;

    let inspect_provider = client
        .call_tool(
            CallToolRequestParams::new("inspect_provider").with_arguments(args_map(json!({
                "address": provider_addr.to_string()
            }))),
        )
        .await
        .expect("inspect provider");
    assert_eq!(inspect_provider.is_error, Some(false));

    let inspect_job = client
        .call_tool(
            CallToolRequestParams::new("inspect_job").with_arguments(args_map(json!({
                "address": job_addr.to_string()
            }))),
        )
        .await
        .expect("inspect job");
    assert_eq!(inspect_job.is_error, Some(false));

    let create_args = args_map(json!({
        "buyer": buyer.to_string(),
        "provider": provider.to_string(),
        "job_nonce": 1,
        "amount": 1000,
        "request_hash_hex": "09".repeat(32),
        "fund_deadline": 1_700_000_100i64,
        "accept_deadline": 1_700_000_200i64,
        "work_deadline": 1_700_000_300i64,
        "auto_settle_deadline": 1_700_000_400i64
    }));
    assert!(matches!(
        create_args.get("job_nonce"),
        Some(Value::Number(_))
    ));
    assert!(matches!(create_args.get("amount"), Some(Value::Number(_))));

    let create = client
        .call_tool(CallToolRequestParams::new("build_create_job").with_arguments(create_args))
        .await
        .expect("create");
    assert_eq!(create.is_error, Some(false));
    let create_plan = structured(&create);
    assert_eq!(create_plan["action"], "create_job");
    assert_eq!(create_plan["program_id"], program.to_string());
    assert!(create_plan.get("private_key").is_none());

    let fund = client
        .call_tool(
            CallToolRequestParams::new("build_fund_job").with_arguments(args_map(json!({
                "buyer": buyer.to_string(),
                "provider": provider.to_string(),
                "job_nonce": 1
            }))),
        )
        .await
        .expect("fund");
    assert_eq!(fund.is_error, Some(false));
    assert_eq!(structured(&fund)["action"], "fund_job");

    let accept_work = client
        .call_tool(
            CallToolRequestParams::new("build_accept_work").with_arguments(args_map(json!({
                "buyer": buyer.to_string(),
                "provider": provider.to_string(),
                "job_nonce": 1
            }))),
        )
        .await
        .expect("accept work");
    assert_eq!(accept_work.is_error, Some(false));
    assert_eq!(structured(&accept_work)["action"], "accept_work");

    let challenge = client
        .call_tool(
            CallToolRequestParams::new("build_challenge").with_arguments(args_map(json!({
                "buyer": buyer.to_string(),
                "provider": provider.to_string(),
                "job_nonce": 1,
                "reason_hash_hex": "0a".repeat(32)
            }))),
        )
        .await
        .expect("challenge");
    assert_eq!(challenge.is_error, Some(false));
    assert_eq!(structured(&challenge)["action"], "challenge_work");

    let submit = client
        .call_tool(
            CallToolRequestParams::new("build_submit_receipt").with_arguments(args_map(json!({
                "job": job_addr.to_string(),
                "provider": provider.to_string(),
                "receipt": {
                    "program_id_hex": hex_encode(&program.to_bytes()),
                    "genesis_hash_hex": "07".repeat(32),
                    "job_hex": hex_encode(&job_addr.to_bytes()),
                    "buyer_hex": hex_encode(&buyer.to_bytes()),
                    "provider_hex": hex_encode(&provider.to_bytes()),
                    "request_hash_hex": "09".repeat(32),
                    "result_hash_hex": "04".repeat(32),
                    "artifact_hash_hex": "05".repeat(32),
                    "software_hash_hex": "06".repeat(32),
                    "job_nonce": 1,
                    "created_at": 1_700_000_000i64,
                    "expires_at": 1_700_000_400i64
                },
                "execution_pubkey_hex": "0b".repeat(32),
                "signature_hex": "0c".repeat(64)
            }))),
        )
        .await
        .expect("submit");
    assert_eq!(submit.is_error, Some(false));
    let submit_plan = structured(&submit);
    assert_eq!(submit_plan["action"], "submit_receipt");
    assert!(submit_plan["instructions"].as_array().expect("ixs").len() >= 2);

    reader.set_timestamp(1_700_000_100).await;
    let timeout = client
        .call_tool(
            CallToolRequestParams::new("build_timeout_resolution").with_arguments(args_map(
                json!({
                    "payer": buyer.to_string(),
                    "buyer": buyer.to_string(),
                    "provider": provider.to_string(),
                    "job_nonce": 1
                }),
            )),
        )
        .await
        .expect("timeout");
    assert_eq!(timeout.is_error, Some(false));
    assert_eq!(structured(&timeout)["action"], "expire_unfunded");

    let missing = client
        .call_tool(
            CallToolRequestParams::new("build_fund_job").with_arguments(args_map(json!({
                "buyer": buyer.to_string()
            }))),
        )
        .await
        .expect("missing args tool result");
    assert_eq!(missing.is_error, Some(true));

    let bad_type = client
        .call_tool(
            CallToolRequestParams::new("build_fund_job").with_arguments(args_map(json!({
                "buyer": buyer.to_string(),
                "provider": provider.to_string(),
                "job_nonce": "not-int"
            }))),
        )
        .await
        .expect("bad type tool result");
    assert_eq!(bad_type.is_error, Some(true));

    let unknown = client
        .call_tool(CallToolRequestParams::new("no_such_tool").with_arguments(Map::new()))
        .await;
    assert!(unknown.is_err(), "unknown tool must be MCP protocol error");

    shutdown(client, server_task).await;
}

#[tokio::test]
async fn direct_dispatch_still_useful_for_unit_checks() {
    let reader = seed_reader().await;
    let server = AgentBondMcp {
        cfg: cfg(),
        catalog: catalog(),
        reader: reader as Arc<dyn agentbond_sdk::ChainReader>,
    };
    let listed = server.dispatch("discover_services", json!({})).await;
    assert_eq!(listed.is_error, Some(false));
    let mut seen = HashMap::new();
    for tool in AgentBondMcp::tools() {
        seen.insert(tool.name.to_string(), true);
    }
    assert_eq!(seen.len(), 9);
}
