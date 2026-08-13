use agentbond_app::AppError;
use agentbond_payments::PaymentError;
use agentbond_sdk::SdkError;
use axum::Json;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use tokio::task_local;

task_local! {
    pub static REQUEST_ID: String;
}

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: String,
    message: String,
    details: Value,
    headers: Box<HeaderMap>,
    request_id: Option<String>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
            details: Value::Null,
            headers: Box::new(HeaderMap::new()),
            request_id: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }

    pub fn with_request_id(mut self, id: impl Into<String>) -> Self {
        self.request_id = Some(id.into());
        self
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    pub fn payment_required(headers: HeaderMap) -> Self {
        Self {
            status: StatusCode::PAYMENT_REQUIRED,
            code: "payment_required".into(),
            message: "payment required".into(),
            details: Value::Null,
            headers: Box::new(headers),
            request_id: None,
        }
    }
}

impl From<AppError> for ApiError {
    fn from(value: AppError) -> Self {
        match value {
            AppError::NotFound(m) => Self::new(StatusCode::NOT_FOUND, "not_found", m),
            AppError::Validation(m) | AppError::Config(m) => Self::bad_request(m),
            AppError::Sdk(SdkError::Rpc(m)) => {
                Self::new(StatusCode::BAD_GATEWAY, "rpc_error", safe_message(&m))
            }
            AppError::Sdk(e) => Self::bad_request(safe_message(&e.to_string())),
        }
    }
}

fn safe_message(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("private") || lower.contains("secret") || lower.contains("stack") {
        "request failed".into()
    } else {
        raw.to_string()
    }
}

impl From<PaymentError> for ApiError {
    fn from(value: PaymentError) -> Self {
        let (status, code) = match value {
            PaymentError::MissingPayment => (StatusCode::PAYMENT_REQUIRED, "payment_required"),
            PaymentError::VerifyTimeout => (StatusCode::GATEWAY_TIMEOUT, "verify_timeout"),
            PaymentError::SettleTimeout => (StatusCode::GATEWAY_TIMEOUT, "settle_timeout"),
            PaymentError::VerifyRejected | PaymentError::SettleRejected => {
                (StatusCode::PAYMENT_REQUIRED, "payment_rejected")
            }
            PaymentError::SettlementInProgress => (StatusCode::CONFLICT, "settlement_in_progress"),
            PaymentError::LeaseMismatch => (StatusCode::CONFLICT, "lease_mismatch"),
            PaymentError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "payment_internal"),
            _ => (StatusCode::BAD_REQUEST, "payment_error"),
        };
        Self::new(status, code, value.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let request_id = self
            .request_id
            .clone()
            .or_else(|| REQUEST_ID.try_with(|id| id.clone()).ok())
            .unwrap_or_else(|| "unknown".into());
        let body = Json(json!({
            "error": {
                "code": self.code,
                "message": self.message,
                "details": self.details,
                "request_id": request_id,
            }
        }));
        let mut response = (self.status, body).into_response();
        response.headers_mut().extend(*self.headers);
        if let Ok(v) = HeaderValue::from_str(&request_id) {
            response.headers_mut().insert("x-request-id", v);
        }
        response
    }
}
