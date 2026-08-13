use std::sync::Arc;

use agentbond_app::{
    AcceptWorkRequest, AppConfig, ChallengeRequest, CreateJobRequest, FundJobRequest,
    ServiceCatalog, SubmitReceiptRequest, TimeoutRequest, build_accept_work_plan,
    build_challenge_plan, build_create_job_plan, build_fund_job_plan, build_submit_receipt_plan_uc,
    build_timeout_plan, inspect_job, inspect_provider, list_services,
};
use agentbond_sdk::{ChainReader, MockChainReader, parse_pubkey};
use rmcp::ErrorData as McpError;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, ProtocolVersion, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer, ServiceExt};
use serde_json::{Map, Value, json};

struct AgentBondMcp {
    cfg: AppConfig,
    catalog: ServiceCatalog,
    reader: Arc<dyn ChainReader>,
}

impl AgentBondMcp {
    fn tools() -> Vec<Tool> {
        let empty = Arc::new(Map::new());
        vec![
            Tool::new("discover_services", "List catalog services", empty.clone()),
            Tool::new(
                "inspect_provider",
                "Inspect a provider account",
                schema_object(&["address"]),
            ),
            Tool::new(
                "inspect_job",
                "Inspect a job account",
                schema_object(&["address"]),
            ),
            Tool::new(
                "build_create_job",
                "Build an unsigned create-job plan",
                schema_object(&[
                    "buyer",
                    "provider",
                    "job_nonce",
                    "amount",
                    "request_hash_hex",
                    "fund_deadline",
                    "accept_deadline",
                    "work_deadline",
                    "auto_settle_deadline",
                ]),
            ),
            Tool::new(
                "build_fund_job",
                "Build an unsigned fund-job plan",
                schema_object(&["buyer", "provider", "job_nonce"]),
            ),
            Tool::new(
                "build_submit_receipt",
                "Build Ed25519 + SubmitReceipt plan from signature material",
                schema_object(&[
                    "job",
                    "provider",
                    "receipt",
                    "execution_pubkey_hex",
                    "signature_hex",
                ]),
            ),
            Tool::new(
                "build_accept_work",
                "Build an unsigned accept-work plan",
                schema_object(&["buyer", "provider", "job_nonce"]),
            ),
            Tool::new(
                "build_challenge",
                "Build an unsigned challenge plan",
                schema_object(&["buyer", "provider", "job_nonce", "reason_hash_hex"]),
            ),
            Tool::new(
                "build_timeout_resolution",
                "Build an eligible timeout/refund/expire plan",
                schema_object(&["payer", "buyer", "provider", "job_nonce"]),
            ),
        ]
    }

    async fn dispatch(&self, name: &str, args: Value) -> CallToolResult {
        let result = async {
            match name {
                "discover_services" => Ok(json!({ "services": list_services(&self.catalog) })),
                "inspect_provider" => {
                    let address = str_arg(&args, "address")?;
                    let pk = parse_pubkey(address).map_err(|e| e.to_string())?;
                    let program = self.cfg.program_pubkey().map_err(|e| e.to_string())?;
                    let provider = inspect_provider(self.reader.as_ref(), &program, &pk)
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok(json!({ "provider": format!("{provider:?}") }))
                }
                "inspect_job" => {
                    let address = str_arg(&args, "address")?;
                    let pk = parse_pubkey(address).map_err(|e| e.to_string())?;
                    let program = self.cfg.program_pubkey().map_err(|e| e.to_string())?;
                    let job = inspect_job(self.reader.as_ref(), &program, &pk)
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok(json!({ "job": format!("{job:?}") }))
                }
                "build_create_job" => {
                    let req: CreateJobRequest =
                        serde_json::from_value(args).map_err(|e| e.to_string())?;
                    let plan = build_create_job_plan(&self.cfg, self.reader.as_ref(), &req)
                        .await
                        .map_err(|e| e.to_string())?;
                    serde_json::to_value(plan).map_err(|e| e.to_string())
                }
                "build_fund_job" => {
                    let req: FundJobRequest =
                        serde_json::from_value(args).map_err(|e| e.to_string())?;
                    let plan = build_fund_job_plan(&self.cfg, &req).map_err(|e| e.to_string())?;
                    serde_json::to_value(plan).map_err(|e| e.to_string())
                }
                "build_submit_receipt" => {
                    let req: SubmitReceiptRequest =
                        serde_json::from_value(args).map_err(|e| e.to_string())?;
                    let plan =
                        build_submit_receipt_plan_uc(&self.cfg, &req).map_err(|e| e.to_string())?;
                    serde_json::to_value(plan).map_err(|e| e.to_string())
                }
                "build_accept_work" => {
                    let req: AcceptWorkRequest =
                        serde_json::from_value(args).map_err(|e| e.to_string())?;
                    let plan =
                        build_accept_work_plan(&self.cfg, &req).map_err(|e| e.to_string())?;
                    serde_json::to_value(plan).map_err(|e| e.to_string())
                }
                "build_challenge" => {
                    let req: ChallengeRequest =
                        serde_json::from_value(args).map_err(|e| e.to_string())?;
                    let plan = build_challenge_plan(&self.cfg, &req).map_err(|e| e.to_string())?;
                    serde_json::to_value(plan).map_err(|e| e.to_string())
                }
                "build_timeout_resolution" => {
                    let req: TimeoutRequest =
                        serde_json::from_value(args).map_err(|e| e.to_string())?;
                    let plan = build_timeout_plan(&self.cfg, self.reader.as_ref(), &req)
                        .await
                        .map_err(|e| e.to_string())?;
                    serde_json::to_value(plan).map_err(|e| e.to_string())
                }
                _ => Err(format!("unknown tool: {name}")),
            }
        }
        .await;

        match result {
            Ok(value) => CallToolResult::structured(value),
            Err(message) => CallToolResult::error(vec![ContentBlock::text(message)]),
        }
    }
}

impl ServerHandler for AgentBondMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2026_07_28)
            .with_server_info(Implementation::from_build_env())
            .with_instructions(
                "AgentBond MCP builds unsigned instruction plans. It never signs or submits transactions.",
            )
    }

    // Trait returns `impl Future`; keep explicit future shape for RMCP ServerHandler.
    #[allow(clippy::manual_async_fn)]
    fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        async move { Ok(ListToolsResult::with_all_items(Self::tools())) }
    }

    #[allow(clippy::manual_async_fn)]
    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, McpError>> + Send + '_ {
        async move {
            if Self::tools().iter().all(|t| t.name != request.name) {
                return Err(McpError::invalid_params(
                    format!("unknown tool {}", request.name),
                    None,
                ));
            }
            let args = request
                .arguments
                .map(Value::Object)
                .unwrap_or_else(|| json!({}));
            let result = self.dispatch(&request.name, args).await;
            Ok(CallToolResponse::Complete(result))
        }
    }
}

fn schema_object(required: &[&str]) -> Arc<Map<String, Value>> {
    let mut props = Map::new();
    for key in required {
        props.insert((*key).into(), json!({ "type": "string" }));
    }
    let mut schema = Map::new();
    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), Value::Object(props));
    schema.insert(
        "required".into(),
        Value::Array(required.iter().map(|s| json!(s)).collect()),
    );
    Arc::new(schema)
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing {key}"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter("info")
        .init();
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config/example.config.json".into());
    let cfg = AppConfig::load_file(&config_path)?;
    let catalog = ServiceCatalog::load_file(&cfg.catalog_path)?;
    let reader: Arc<dyn ChainReader> = Arc::new(MockChainReader::new());
    let server = AgentBondMcp {
        cfg,
        catalog,
        reader,
    };
    let transport = rmcp::transport::io::stdio();
    let service = server.serve(transport).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentbond_app::ServiceEntry;
    use agentbond_sdk::{AccountData, job_pda, program_id};
    use agentbond_types::{JobAccount, JobState};
    use solana_pubkey::Pubkey;

    fn test_config() -> AppConfig {
        AppConfig {
            program_id: agentbond_sdk::program_id().to_string(),
            rpc_url: "http://127.0.0.1:8899".into(),
            genesis_hash: "07".repeat(32),
            settlement_mint: "11111111111111111111111111111111".into(),
            token_program: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".into(),
            facilitator_url: "http://127.0.0.1:9090".into(),
            merchant_pay_to: "11111111111111111111111111111112".into(),
            x402_amount: "1000".into(),
            x402_network: "solana:localnet".into(),
            request_timeout_ms: 5000,
            max_request_bytes: 65536,
            bind_address: "127.0.0.1:8080".into(),
            catalog_path: "config/example.catalog.json".into(),
        }
    }

    fn test_catalog() -> ServiceCatalog {
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

    fn assert_plan_result(result: &CallToolResult, tool: &str) {
        assert_eq!(result.is_error, Some(false), "{tool} erred: {result:?}");
        let content = result
            .structured_content
            .as_ref()
            .expect("structured plan content");
        assert!(
            content.get("action").is_some() || content.get("instructions").is_some(),
            "{tool} missing plan fields: {content}"
        );
        let text = format!("{result:?}").to_ascii_lowercase();
        assert!(!text.contains("private_key"), "{tool}");
        assert!(!text.contains("sign transaction"), "{tool}");
        assert!(!text.contains("submit transaction"), "{tool}");
    }

    #[tokio::test]
    async fn tool_list_and_every_tool_via_dispatch() {
        let reader = MockChainReader::new();
        reader.set_timestamp(1_700_000_000).await;

        let program = program_id();
        let buyer: Pubkey = "11111111111111111111111111111111".parse().expect("buyer");
        let provider: Pubkey = "11111111111111111111111111111112"
            .parse()
            .expect("provider");
        let job_addr = job_pda(&program, &buyer, &provider, 1)
            .expect("job")
            .address;
        let job = JobAccount {
            bump: 1,
            state: JobState::Created,
            buyer: buyer.to_bytes(),
            provider: provider.to_bytes(),
            mint: [0u8; 32],
            token_program: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
                .parse::<Pubkey>()
                .expect("token")
                .to_bytes(),
            amount: 1000,
            job_nonce: 1,
            fund_deadline: 1_699_999_000,
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

        let server = AgentBondMcp {
            cfg: test_config(),
            catalog: test_catalog(),
            reader: Arc::new(reader),
        };

        let info = server.get_info();
        let instructions = format!("{:?}", info);
        assert!(
            instructions.to_ascii_lowercase().contains("never signs")
                || instructions.contains("never signs or submits"),
            "get_info must state no signing/submission: {instructions}"
        );
        assert!(
            !instructions
                .to_ascii_lowercase()
                .contains("submits transactions")
                || instructions.contains("never signs or submits")
        );

        assert_eq!(AgentBondMcp::tools().len(), 9);
        let tool_names: Vec<_> = AgentBondMcp::tools()
            .iter()
            .map(|t| t.name.clone())
            .collect();
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
            assert!(
                tool_names.iter().any(|n| n == expected),
                "missing {expected}"
            );
        }

        // Invalid args for every tool that requires them.
        for name in [
            "inspect_provider",
            "inspect_job",
            "build_fund_job",
            "build_accept_work",
            "build_challenge",
            "build_create_job",
            "build_submit_receipt",
            "build_timeout_resolution",
        ] {
            let bad = server.dispatch(name, json!({})).await;
            assert_eq!(bad.is_error, Some(true), "{name} should reject empty args");
        }

        let listed = server.dispatch("discover_services", json!({})).await;
        assert_eq!(listed.is_error, Some(false));
        assert!(listed.structured_content.is_some());

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
        assert_plan_result(&create, "build_create_job");

        let fund = server
            .dispatch(
                "build_fund_job",
                json!({
                    "buyer": "11111111111111111111111111111111",
                    "provider": "11111111111111111111111111111112",
                    "job_nonce": 1
                }),
            )
            .await;
        assert_plan_result(&fund, "build_fund_job");

        let accept_work = server
            .dispatch(
                "build_accept_work",
                json!({
                    "buyer": "11111111111111111111111111111111",
                    "provider": "11111111111111111111111111111112",
                    "job_nonce": 1
                }),
            )
            .await;
        assert_plan_result(&accept_work, "build_accept_work");

        let challenge = server
            .dispatch(
                "build_challenge",
                json!({
                    "buyer": "11111111111111111111111111111111",
                    "provider": "11111111111111111111111111111112",
                    "job_nonce": 1,
                    "reason_hash_hex": "0a".repeat(32)
                }),
            )
            .await;
        assert_plan_result(&challenge, "build_challenge");

        let timeout = server
            .dispatch(
                "build_timeout_resolution",
                json!({
                    "payer": "11111111111111111111111111111111",
                    "buyer": "11111111111111111111111111111111",
                    "provider": "11111111111111111111111111111112",
                    "job_nonce": 1
                }),
            )
            .await;
        assert_plan_result(&timeout, "build_timeout_resolution");

        let inspect_job = server
            .dispatch("inspect_job", json!({ "address": job_addr.to_string() }))
            .await;
        assert_eq!(inspect_job.is_error, Some(false), "{inspect_job:?}");

        // Missing provider account -> structured error, not panic.
        let inspect_provider = server
            .dispatch(
                "inspect_provider",
                json!({ "address": "11111111111111111111111111111113" }),
            )
            .await;
        assert_eq!(inspect_provider.is_error, Some(true));

        let submit = server
            .dispatch(
                "build_submit_receipt",
                json!({
                    "job": job_addr.to_string(),
                    "provider": "11111111111111111111111111111112",
                    "receipt": {
                        "program_id_hex": hex::encode(&program_id().to_bytes()),
                        "genesis_hash_hex": "07".repeat(32),
                        "job_hex": hex::encode(&job_addr.to_bytes()),
                        "buyer_hex": "02".repeat(32),
                        "provider_hex": hex::encode(&provider.to_bytes()),
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
                }),
            )
            .await;
        assert_plan_result(&submit, "build_submit_receipt");
    }

    mod hex {
        pub fn encode(bytes: &[u8]) -> String {
            bytes.iter().map(|b| format!("{b:02x}")).collect()
        }
    }
}
