//! AgentBond HTTP gateway library (routes + state for tests).

pub mod error;
pub mod state;

use std::sync::Arc;
use std::time::Duration;

use agentbond_app::{
    AcceptJobRequest, AcceptWorkRequest, ChallengeRequest, CreateJobRequest, FundJobRequest,
    SubmitReceiptRequest, TimeoutRequest, build_accept_job_plan, build_accept_work_plan,
    build_challenge_plan, build_create_job_plan, build_fund_job_plan, build_submit_receipt_plan_uc,
    build_timeout_plan, get_service, inspect_job_dto, inspect_provider_dto, list_services,
};
use agentbond_payments::headers::{
    PAYMENT_REQUIRED, PAYMENT_RESPONSE, PAYMENT_SIGNATURE, is_sensitive_header,
};
use agentbond_payments::{X402ResourceConfig, invoke_paid_demo};
use agentbond_sdk::{ChainReader, parse_pubkey};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
pub use state::AppState;

/// Test helper: build AppState with mock stores.
pub fn test_state(
    cfg: agentbond_app::AppConfig,
    catalog: agentbond_app::ServiceCatalog,
    reader: Arc<dyn ChainReader>,
    facilitator: Arc<dyn agentbond_payments::FacilitatorClient>,
) -> AppState {
    AppState {
        cfg: Arc::new(cfg),
        catalog: Arc::new(catalog),
        reader,
        facilitator,
        challenges: Arc::new(agentbond_payments::ChallengeStore::new()),
        settlements: Arc::new(agentbond_payments::SettlementStore::new()),
    }
}

#[derive(Clone)]
struct RequestId(String);

pub fn router(state: AppState, max_body: usize, timeout: Duration) -> Router {
    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .route("/v1/services", get(services))
        .route("/v1/services/{service_id}", get(service))
        .route("/v1/providers/{address}", get(provider))
        .route("/v1/jobs/{address}", get(job))
        .route("/v1/plans/jobs/create", post(plan_create))
        .route("/v1/plans/jobs/fund", post(plan_fund))
        .route("/v1/plans/jobs/accept", post(plan_accept))
        .route("/v1/plans/jobs/submit-receipt", post(plan_submit))
        .route("/v1/plans/jobs/accept-work", post(plan_accept_work))
        .route("/v1/plans/jobs/challenge", post(plan_challenge))
        .route("/v1/plans/jobs/resolve-timeout", post(plan_timeout))
        .route("/v1/x402/services/{service_id}/invoke", post(x402_invoke))
        .layer(middleware::from_fn(attach_request_id))
        .layer(RequestBodyLimitLayer::new(max_body))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            timeout,
        ))
        .layer(
            TraceLayer::new_for_http().make_span_with(|req: &axum::http::Request<_>| {
                let id = req
                    .extensions()
                    .get::<RequestId>()
                    .map(|r| r.0.clone())
                    .unwrap_or_else(|| "unknown".into());
                let mut headers = String::new();
                for (k, _) in req.headers() {
                    if is_sensitive_header(k.as_str()) {
                        headers.push_str(k.as_str());
                        headers.push_str("=<redacted>;");
                    }
                }
                tracing::info_span!("request", request_id = %id, sensitive = %headers)
            }),
        )
        .with_state(state)
}

async fn attach_request_id(mut req: Request<axum::body::Body>, next: Next) -> Response {
    let id = Uuid::new_v4().to_string();
    req.extensions_mut().insert(RequestId(id.clone()));
    let mut response = next.run(req).await;
    if let Ok(v) = HeaderValue::from_str(&id) {
        response.headers_mut().insert("x-request-id", v);
    }
    response
}

fn request_id_from(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

async fn live() -> StatusCode {
    StatusCode::OK
}

async fn ready(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rpc = state.reader.ready().await;
    let fac = state.facilitator.ready().await;
    let ok = rpc.is_ok() && fac.is_ok();
    let body = serde_json::json!({
        "rpc": rpc.is_ok(),
        "facilitator": fac.is_ok(),
    });
    if ok {
        Ok(Json(body))
    } else {
        Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "dependencies_not_ready",
            "dependencies not ready",
        )
        .with_details(body))
    }
}

async fn services(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    Ok(Json(
        serde_json::json!({ "services": list_services(&state.catalog) }),
    ))
}

async fn service(
    State(state): State<AppState>,
    Path(service_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let s = get_service(&state.catalog, &service_id).map_err(ApiError::from)?;
    let value = serde_json::to_value(s).map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(value))
}

async fn provider(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<Value>> {
    reject_secrets(&serde_json::json!({ "address": address }))?;
    let pk = parse_pubkey(&address).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let program = state.cfg.program_pubkey().map_err(ApiError::from)?;
    let account = inspect_provider_dto(state.reader.as_ref(), &program, &pk)
        .await
        .map_err(ApiError::from)?;
    let value = serde_json::to_value(account).map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(serde_json::json!({ "provider": value })))
}

async fn job(State(state): State<AppState>, Path(address): Path<String>) -> ApiResult<Json<Value>> {
    let pk = parse_pubkey(&address).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let program = state.cfg.program_pubkey().map_err(ApiError::from)?;
    let account = inspect_job_dto(state.reader.as_ref(), &program, &pk)
        .await
        .map_err(ApiError::from)?;
    let value = serde_json::to_value(account).map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(serde_json::json!({ "job": value })))
}

fn parse_json_body<T: serde::de::DeserializeOwned>(value: Value) -> ApiResult<T> {
    reject_secrets(&value)?;
    serde_json::from_value(value).map_err(|e| ApiError::bad_request(e.to_string()))
}

async fn plan_create(
    State(state): State<AppState>,
    Json(raw): Json<Value>,
) -> ApiResult<Json<Value>> {
    let req: CreateJobRequest = parse_json_body(raw)?;
    let plan = build_create_job_plan(&state.cfg, state.reader.as_ref(), &req)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(
        serde_json::to_value(plan).map_err(|e| ApiError::bad_request(e.to_string()))?,
    ))
}

async fn plan_fund(
    State(state): State<AppState>,
    Json(raw): Json<Value>,
) -> ApiResult<Json<Value>> {
    let req: FundJobRequest = parse_json_body(raw)?;
    let plan = build_fund_job_plan(&state.cfg, &req).map_err(ApiError::from)?;
    Ok(Json(
        serde_json::to_value(plan).map_err(|e| ApiError::bad_request(e.to_string()))?,
    ))
}

async fn plan_accept(
    State(state): State<AppState>,
    Json(raw): Json<Value>,
) -> ApiResult<Json<Value>> {
    let req: AcceptJobRequest = parse_json_body(raw)?;
    let plan = build_accept_job_plan(&state.cfg, &req).map_err(ApiError::from)?;
    Ok(Json(
        serde_json::to_value(plan).map_err(|e| ApiError::bad_request(e.to_string()))?,
    ))
}

async fn plan_submit(
    State(state): State<AppState>,
    Json(raw): Json<Value>,
) -> ApiResult<Json<Value>> {
    let req: SubmitReceiptRequest = parse_json_body(raw)?;
    let plan = build_submit_receipt_plan_uc(&state.cfg, state.reader.as_ref(), &req)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(
        serde_json::to_value(plan).map_err(|e| ApiError::bad_request(e.to_string()))?,
    ))
}

async fn plan_accept_work(
    State(state): State<AppState>,
    Json(raw): Json<Value>,
) -> ApiResult<Json<Value>> {
    let req: AcceptWorkRequest = parse_json_body(raw)?;
    let plan = build_accept_work_plan(&state.cfg, &req).map_err(ApiError::from)?;
    Ok(Json(
        serde_json::to_value(plan).map_err(|e| ApiError::bad_request(e.to_string()))?,
    ))
}

async fn plan_challenge(
    State(state): State<AppState>,
    Json(raw): Json<Value>,
) -> ApiResult<Json<Value>> {
    let req: ChallengeRequest = parse_json_body(raw)?;
    let plan = build_challenge_plan(&state.cfg, &req).map_err(ApiError::from)?;
    Ok(Json(
        serde_json::to_value(plan).map_err(|e| ApiError::bad_request(e.to_string()))?,
    ))
}

async fn plan_timeout(
    State(state): State<AppState>,
    Json(raw): Json<Value>,
) -> ApiResult<Json<Value>> {
    let req: TimeoutRequest = parse_json_body(raw)?;
    let plan = build_timeout_plan(&state.cfg, state.reader.as_ref(), &req)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(
        serde_json::to_value(plan).map_err(|e| ApiError::bad_request(e.to_string()))?,
    ))
}

#[derive(Deserialize)]
struct X402Body {
    input: Value,
}

async fn x402_invoke(
    State(state): State<AppState>,
    Path(service_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<X402Body>,
) -> ApiResult<(StatusCode, HeaderMap, Json<Value>)> {
    let service = get_service(&state.catalog, &service_id).map_err(ApiError::from)?;
    if service.x402_demo_route.is_none() {
        return Err(ApiError::bad_request("service has no x402 demo route"));
    }
    let resource = X402ResourceConfig {
        network: state.cfg.x402_network.clone(),
        asset: state.cfg.settlement_mint.clone(),
        pay_to: state.cfg.merchant_pay_to.clone(),
        fee_payer: state.cfg.x402_fee_payer.clone(),
        amount: state.cfg.x402_amount.clone(),
        max_timeout_seconds: 60,
        resource_url: format!("/v1/x402/services/{service_id}/invoke"),
        description: service.description.clone(),
        service_id: service_id.clone(),
    };
    let payment = headers.get(PAYMENT_SIGNATURE).and_then(|v| v.to_str().ok());
    let now = state
        .reader
        .get_unix_timestamp()
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, "clock_error", e.to_string()))?;

    match invoke_paid_demo(
        &resource,
        state.facilitator.as_ref(),
        state.challenges.as_ref(),
        state.settlements.as_ref(),
        payment,
        &body.input,
        now,
    )
    .await
    {
        Ok(Ok(result)) => {
            let mut out = HeaderMap::new();
            out.insert(
                PAYMENT_RESPONSE,
                result
                    .payment_response_header
                    .parse()
                    .map_err(|_| ApiError::bad_request("bad payment response header"))?,
            );
            Ok((StatusCode::OK, out, Json(result.body)))
        }
        Ok(Err(payment_required_header)) => {
            let mut out = HeaderMap::new();
            out.insert(
                PAYMENT_REQUIRED,
                payment_required_header
                    .parse()
                    .map_err(|_| ApiError::bad_request("bad payment required header"))?,
            );
            Err(ApiError::payment_required(out))
        }
        Err(e) => Err(ApiError::from(e)),
    }
}

fn reject_secrets(v: &Value) -> ApiResult<()> {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                let key = k.to_ascii_lowercase();
                if key.contains("private") || key.contains("secret") || key == "keypair" {
                    return Err(ApiError::bad_request("private key fields are not accepted"));
                }
                reject_secrets(val)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                reject_secrets(item)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[allow(dead_code)]
fn _touch_request_id(headers: &HeaderMap) -> String {
    request_id_from(headers)
}
