use agentbond_app::AppError;
use agentbond_payments::PaymentError;
use axum::Json;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
    details: Value,
    headers: Box<HeaderMap>,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>, details: Value) -> Self {
        Self {
            status,
            message: message.into(),
            details,
            headers: Box::new(HeaderMap::new()),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message, Value::Null)
    }

    pub fn payment_required(headers: HeaderMap) -> Self {
        Self {
            status: StatusCode::PAYMENT_REQUIRED,
            message: "payment required".into(),
            details: Value::Null,
            headers: Box::new(headers),
        }
    }
}

impl From<AppError> for ApiError {
    fn from(value: AppError) -> Self {
        match value {
            AppError::NotFound(m) => Self::new(StatusCode::NOT_FOUND, m, Value::Null),
            AppError::Validation(m) | AppError::Config(m) => Self::bad_request(m),
            AppError::Sdk(e) => Self::bad_request(e.to_string()),
        }
    }
}

impl From<PaymentError> for ApiError {
    fn from(value: PaymentError) -> Self {
        let status = match value {
            PaymentError::MissingPayment => StatusCode::PAYMENT_REQUIRED,
            PaymentError::VerifyTimeout | PaymentError::SettleTimeout => {
                StatusCode::GATEWAY_TIMEOUT
            }
            PaymentError::VerifyRejected | PaymentError::SettleRejected => {
                StatusCode::PAYMENT_REQUIRED
            }
            _ => StatusCode::BAD_REQUEST,
        };
        Self::new(status, value.to_string(), Value::Null)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(json!({
            "error": {
                "message": self.message,
                "details": self.details,
            }
        }));
        let mut response = (self.status, body).into_response();
        response.headers_mut().extend(*self.headers);
        response
    }
}
