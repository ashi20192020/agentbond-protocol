use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use crate::error::PaymentError;
use crate::models::{SettleRequest, SettleResponse, VerifyRequest, VerifyResponse};

#[async_trait]
pub trait FacilitatorClient: Send + Sync {
    async fn verify(&self, request: &VerifyRequest) -> Result<VerifyResponse, PaymentError>;
    async fn settle(&self, request: &SettleRequest) -> Result<SettleResponse, PaymentError>;
    async fn ready(&self) -> Result<(), PaymentError>;
}

pub struct HttpFacilitatorClient {
    client: reqwest::Client,
    base_url: String,
}

impl HttpFacilitatorClient {
    pub fn new(base_url: impl Into<String>, timeout: Duration) -> Result<Self, PaymentError> {
        let base_url = base_url.into();
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            return Err(PaymentError::Config(
                "facilitator URL must be http(s)".into(),
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| PaymentError::Facilitator(e.to_string()))?;
        Ok(Self { client, base_url })
    }

    async fn post_json<T: for<'de> serde::Deserialize<'de>, B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, PaymentError> {
        let url = format!("{}{}", self.base_url.trim_end_matches('/'), path);
        let response = self.client.post(url).json(body).send().await.map_err(|e| {
            if e.is_timeout() {
                PaymentError::VerifyTimeout
            } else {
                PaymentError::Facilitator(e.to_string())
            }
        })?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| PaymentError::Facilitator(e.to_string()))?;
        if bytes.len() > 64 * 1024 {
            return Err(PaymentError::Facilitator("response too large".into()));
        }
        if !status.is_success() {
            return Err(PaymentError::Facilitator(format!("http {status}")));
        }
        serde_json::from_slice(&bytes).map_err(|e| PaymentError::Facilitator(e.to_string()))
    }
}

#[async_trait]
impl FacilitatorClient for HttpFacilitatorClient {
    async fn verify(&self, request: &VerifyRequest) -> Result<VerifyResponse, PaymentError> {
        self.post_json("/verify", request)
            .await
            .map_err(|e| match e {
                PaymentError::VerifyTimeout => PaymentError::VerifyTimeout,
                other => other,
            })
    }

    async fn settle(&self, request: &SettleRequest) -> Result<SettleResponse, PaymentError> {
        self.post_json("/settle", request)
            .await
            .map_err(|e| match e {
                PaymentError::VerifyTimeout => PaymentError::SettleTimeout,
                other => other,
            })
    }

    async fn ready(&self) -> Result<(), PaymentError> {
        let url = format!("{}/health", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| PaymentError::Facilitator(e.to_string()))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(PaymentError::Facilitator("facilitator not ready".into()))
        }
    }
}

#[derive(Clone, Default)]
pub struct MockFacilitatorClient {
    verify_ok: Arc<Mutex<bool>>,
    settle_ok: Arc<Mutex<bool>>,
    verify_delay: Arc<Mutex<Option<Duration>>>,
    settle_delay: Arc<Mutex<Option<Duration>>>,
    ready: Arc<Mutex<bool>>,
    verify_calls: Arc<Mutex<u64>>,
    settle_calls: Arc<Mutex<u64>>,
}

impl MockFacilitatorClient {
    pub fn new() -> Self {
        Self {
            verify_ok: Arc::new(Mutex::new(true)),
            settle_ok: Arc::new(Mutex::new(true)),
            verify_delay: Arc::new(Mutex::new(None)),
            settle_delay: Arc::new(Mutex::new(None)),
            ready: Arc::new(Mutex::new(true)),
            verify_calls: Arc::new(Mutex::new(0)),
            settle_calls: Arc::new(Mutex::new(0)),
        }
    }

    pub async fn set_verify_ok(&self, ok: bool) {
        *self.verify_ok.lock().await = ok;
    }

    pub async fn set_settle_ok(&self, ok: bool) {
        *self.settle_ok.lock().await = ok;
    }

    pub async fn set_verify_delay(&self, delay: Option<Duration>) {
        *self.verify_delay.lock().await = delay;
    }

    pub async fn set_settle_delay(&self, delay: Option<Duration>) {
        *self.settle_delay.lock().await = delay;
    }

    pub async fn set_ready(&self, ready: bool) {
        *self.ready.lock().await = ready;
    }

    pub async fn verify_calls(&self) -> u64 {
        *self.verify_calls.lock().await
    }

    pub async fn settle_calls(&self) -> u64 {
        *self.settle_calls.lock().await
    }
}

#[async_trait]
impl FacilitatorClient for MockFacilitatorClient {
    async fn verify(&self, _request: &VerifyRequest) -> Result<VerifyResponse, PaymentError> {
        *self.verify_calls.lock().await += 1;
        if let Some(delay) = *self.verify_delay.lock().await {
            tokio::time::sleep(delay).await;
            return Err(PaymentError::VerifyTimeout);
        }
        if *self.verify_ok.lock().await {
            Ok(VerifyResponse {
                is_valid: true,
                invalid_reason: None,
                payer: Some("mock-payer".into()),
            })
        } else {
            Ok(VerifyResponse {
                is_valid: false,
                invalid_reason: Some("rejected".into()),
                payer: None,
            })
        }
    }

    async fn settle(&self, _request: &SettleRequest) -> Result<SettleResponse, PaymentError> {
        *self.settle_calls.lock().await += 1;
        if let Some(delay) = *self.settle_delay.lock().await {
            tokio::time::sleep(delay).await;
            return Err(PaymentError::SettleTimeout);
        }
        if *self.settle_ok.lock().await {
            Ok(SettleResponse {
                success: true,
                error_reason: None,
                transaction: Some("mock-tx".into()),
                network: Some("solana:localnet".into()),
                payer: Some("mock-payer".into()),
            })
        } else {
            Ok(SettleResponse {
                success: false,
                error_reason: Some("settle failed".into()),
                transaction: None,
                network: None,
                payer: None,
            })
        }
    }

    async fn ready(&self) -> Result<(), PaymentError> {
        if *self.ready.lock().await {
            Ok(())
        } else {
            Err(PaymentError::Facilitator("not ready".into()))
        }
    }
}
