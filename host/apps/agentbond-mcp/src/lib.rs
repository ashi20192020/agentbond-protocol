use std::sync::Arc;

use agentbond_app::{
    AcceptWorkRequest, AppConfig, ChallengeRequest, CreateJobRequest, FundJobRequest,
    ServiceCatalog, SubmitReceiptRequest, TimeoutRequest, build_accept_work_plan,
    build_challenge_plan, build_create_job_plan, build_fund_job_plan, build_submit_receipt_plan_uc,
    build_timeout_plan, inspect_job_dto, inspect_provider_dto, list_services,
};
use agentbond_sdk::{ChainReader, parse_pubkey};
use rmcp::ErrorData as McpError;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, ProtocolVersion, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use serde_json::{Map, Value, json};

pub struct AgentBondMcp {
    pub cfg: AppConfig,
    pub catalog: ServiceCatalog,
    pub reader: Arc<dyn ChainReader>,
}

impl AgentBondMcp {
    pub fn tools() -> Vec<Tool> {
        vec![
            Tool::new(
                "discover_services",
                "List catalog services",
                Arc::new(object_schema(&[], &[])),
            ),
            Tool::new(
                "inspect_provider",
                "Inspect a provider account",
                Arc::new(object_schema(&[("address", "string")], &["address"])),
            ),
            Tool::new(
                "inspect_job",
                "Inspect a job account",
                Arc::new(object_schema(&[("address", "string")], &["address"])),
            ),
            Tool::new(
                "build_create_job",
                "Build an unsigned create-job plan",
                Arc::new(object_schema(
                    &[
                        ("buyer", "string"),
                        ("provider", "string"),
                        ("job_nonce", "integer"),
                        ("amount", "integer"),
                        ("request_hash_hex", "string"),
                        ("fund_deadline", "integer"),
                        ("accept_deadline", "integer"),
                        ("work_deadline", "integer"),
                        ("auto_settle_deadline", "integer"),
                    ],
                    &[
                        "buyer",
                        "provider",
                        "job_nonce",
                        "amount",
                        "request_hash_hex",
                        "fund_deadline",
                        "accept_deadline",
                        "work_deadline",
                        "auto_settle_deadline",
                    ],
                )),
            ),
            Tool::new(
                "build_fund_job",
                "Build an unsigned fund-job plan",
                Arc::new(object_schema(
                    &[
                        ("buyer", "string"),
                        ("provider", "string"),
                        ("job_nonce", "integer"),
                    ],
                    &["buyer", "provider", "job_nonce"],
                )),
            ),
            Tool::new(
                "build_submit_receipt",
                "Build Ed25519 + SubmitReceipt plan from signature material",
                Arc::new(submit_schema()),
            ),
            Tool::new(
                "build_accept_work",
                "Build an unsigned accept-work plan",
                Arc::new(object_schema(
                    &[
                        ("buyer", "string"),
                        ("provider", "string"),
                        ("job_nonce", "integer"),
                    ],
                    &["buyer", "provider", "job_nonce"],
                )),
            ),
            Tool::new(
                "build_challenge",
                "Build an unsigned challenge plan",
                Arc::new(object_schema(
                    &[
                        ("buyer", "string"),
                        ("provider", "string"),
                        ("job_nonce", "integer"),
                        ("reason_hash_hex", "string"),
                    ],
                    &["buyer", "provider", "job_nonce", "reason_hash_hex"],
                )),
            ),
            Tool::new(
                "build_timeout_resolution",
                "Build an eligible timeout/refund/expire plan",
                Arc::new(object_schema(
                    &[
                        ("payer", "string"),
                        ("buyer", "string"),
                        ("provider", "string"),
                        ("job_nonce", "integer"),
                    ],
                    &["payer", "buyer", "provider", "job_nonce"],
                )),
            ),
        ]
    }

    pub async fn dispatch(&self, name: &str, args: Value) -> CallToolResult {
        let result = async {
            match name {
                "discover_services" => Ok(json!({ "services": list_services(&self.catalog) })),
                "inspect_provider" => {
                    let address = str_arg(&args, "address")?;
                    let pk = parse_pubkey(address).map_err(|e| e.to_string())?;
                    let program = self.cfg.program_pubkey().map_err(|e| e.to_string())?;
                    let provider = inspect_provider_dto(self.reader.as_ref(), &program, &pk)
                        .await
                        .map_err(|e| e.to_string())?;
                    serde_json::to_value(provider).map_err(|e| e.to_string())
                }
                "inspect_job" => {
                    let address = str_arg(&args, "address")?;
                    let pk = parse_pubkey(address).map_err(|e| e.to_string())?;
                    let program = self.cfg.program_pubkey().map_err(|e| e.to_string())?;
                    let job = inspect_job_dto(self.reader.as_ref(), &program, &pk)
                        .await
                        .map_err(|e| e.to_string())?;
                    serde_json::to_value(job).map_err(|e| e.to_string())
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
                    let plan = build_submit_receipt_plan_uc(&self.cfg, self.reader.as_ref(), &req)
                        .await
                        .map_err(|e| e.to_string())?;
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
                "AgentBond MCP builds unsigned instruction plans. It never signs or submits transactions and never accepts private keys.",
            )
    }

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

fn object_schema(fields: &[(&str, &str)], required: &[&str]) -> Map<String, Value> {
    let mut props = Map::new();
    for (name, ty) in fields {
        let mut prop = Map::new();
        prop.insert("type".into(), json!(ty));
        if *ty == "integer" {
            prop.insert("minimum".into(), json!(0));
        }
        props.insert((*name).into(), Value::Object(prop));
    }
    let mut schema = Map::new();
    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), Value::Object(props));
    schema.insert(
        "required".into(),
        Value::Array(required.iter().map(|s| json!(s)).collect()),
    );
    schema.insert("additionalProperties".into(), json!(false));
    schema
}

fn submit_schema() -> Map<String, Value> {
    let mut receipt_props = Map::new();
    for key in [
        "program_id_hex",
        "genesis_hash_hex",
        "job_hex",
        "buyer_hex",
        "provider_hex",
        "request_hash_hex",
        "result_hash_hex",
        "artifact_hash_hex",
        "software_hash_hex",
    ] {
        receipt_props.insert(key.into(), json!({ "type": "string" }));
    }
    receipt_props.insert(
        "job_nonce".into(),
        json!({ "type": "integer", "minimum": 0 }),
    );
    receipt_props.insert("created_at".into(), json!({ "type": "integer" }));
    receipt_props.insert("expires_at".into(), json!({ "type": "integer" }));
    let mut receipt = Map::new();
    receipt.insert("type".into(), json!("object"));
    receipt.insert("properties".into(), Value::Object(receipt_props));
    receipt.insert("additionalProperties".into(), json!(false));

    let mut props = Map::new();
    props.insert("job".into(), json!({ "type": "string" }));
    props.insert("provider".into(), json!({ "type": "string" }));
    props.insert("receipt".into(), Value::Object(receipt));
    props.insert("execution_pubkey_hex".into(), json!({ "type": "string" }));
    props.insert("signature_hex".into(), json!({ "type": "string" }));
    let mut schema = Map::new();
    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), Value::Object(props));
    schema.insert(
        "required".into(),
        json!([
            "job",
            "provider",
            "receipt",
            "execution_pubkey_hex",
            "signature_hex"
        ]),
    );
    schema.insert("additionalProperties".into(), json!(false));
    schema
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing {key}"))
}

pub fn schemas_forbid_private_keys(tools: &[Tool]) -> bool {
    !format!("{tools:?}")
        .to_ascii_lowercase()
        .contains("private")
}
