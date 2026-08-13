use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify};

use crate::error::PaymentError;
use crate::http_util::{
    MAX_FACILITATOR_BODY, build_http_client, read_body_bounded, reject_credentialed_url,
};
use crate::models::{
    SCHEME_EXACT, SettleRequest, SettleResponse, SupportedResponse, VerifyRequest, VerifyResponse,
};

#[async_trait]
pub trait FacilitatorClient: Send + Sync {
    async fn verify(&self, request: &VerifyRequest) -> Result<VerifyResponse, PaymentError>;
    async fn settle(&self, request: &SettleRequest) -> Result<SettleResponse, PaymentError>;
    async fn ready(&self) -> Result<(), PaymentError>;
}

pub struct HttpFacilitatorClient {
    client: reqwest::Client,
    base_url: String,
    expected_network: String,
    expected_fee_payer: Option<String>,
}

impl HttpFacilitatorClient {
    pub fn new(
        base_url: impl Into<String>,
        timeout: Duration,
        expected_network: impl Into<String>,
        expected_fee_payer: Option<String>,
    ) -> Result<Self, PaymentError> {
        let base_url = base_url.into();
        reject_credentialed_url(&base_url)?;
        let client = build_http_client(timeout)?;
        Ok(Self {
            client,
            base_url,
            expected_network: expected_network.into(),
            expected_fee_payer,
        })
    }

    async fn post_json<T: for<'de> serde::Deserialize<'de>, B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
        timeout_kind: TimeoutKind,
    ) -> Result<T, PaymentError> {
        let url = format!("{}{}", self.base_url.trim_end_matches('/'), path);
        let response = self.client.post(url).json(body).send().await.map_err(|e| {
            if e.is_timeout() {
                match timeout_kind {
                    TimeoutKind::Verify => PaymentError::VerifyTimeout,
                    TimeoutKind::Settle => PaymentError::SettleTimeout,
                }
            } else {
                PaymentError::Facilitator(e.to_string())
            }
        })?;
        if response.status().is_redirection() {
            return Err(PaymentError::Facilitator(
                "facilitator redirects are not allowed".into(),
            ));
        }
        let status = response.status();
        let bytes = read_body_bounded(response, MAX_FACILITATOR_BODY).await?;
        if !status.is_success() {
            return Err(PaymentError::Facilitator(format!("http {status}")));
        }
        serde_json::from_slice(&bytes).map_err(|e| PaymentError::Facilitator(e.to_string()))
    }

    async fn get_json<T: for<'de> serde::Deserialize<'de>>(
        &self,
        path: &str,
    ) -> Result<T, PaymentError> {
        let url = format!("{}{}", self.base_url.trim_end_matches('/'), path);
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| PaymentError::Facilitator(e.to_string()))?;
        if response.status().is_redirection() {
            return Err(PaymentError::Facilitator(
                "facilitator redirects are not allowed".into(),
            ));
        }
        let status = response.status();
        let bytes = read_body_bounded(response, MAX_FACILITATOR_BODY).await?;
        if !status.is_success() {
            return Err(PaymentError::Facilitator(format!("http {status}")));
        }
        serde_json::from_slice(&bytes).map_err(|e| PaymentError::Facilitator(e.to_string()))
    }
}

enum TimeoutKind {
    Verify,
    Settle,
}

#[async_trait]
impl FacilitatorClient for HttpFacilitatorClient {
    async fn verify(&self, request: &VerifyRequest) -> Result<VerifyResponse, PaymentError> {
        self.post_json("/verify", request, TimeoutKind::Verify)
            .await
    }

    async fn settle(&self, request: &SettleRequest) -> Result<SettleResponse, PaymentError> {
        self.post_json("/settle", request, TimeoutKind::Settle)
            .await
    }

    async fn ready(&self) -> Result<(), PaymentError> {
        // Facilitator supported-capabilities endpoint (not a custom /health).
        let supported: SupportedResponse = self.get_json("/supported").await?;
        let ok = supported.kinds.iter().any(|k| {
            k.scheme == SCHEME_EXACT
                && k.network == self.expected_network
                && match (&self.expected_fee_payer, &k.fee_payer) {
                    (Some(expected), Some(actual)) => expected == actual,
                    (Some(_), None) => false,
                    (None, _) => true,
                }
        });
        if ok {
            Ok(())
        } else {
            Err(PaymentError::Facilitator(
                "supported kinds do not include exact/network pair".into(),
            ))
        }
    }
}

#[derive(Clone)]
struct SettleHold {
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

#[derive(Clone, Default)]
pub struct MockFacilitatorClient {
    verify_ok: Arc<Mutex<bool>>,
    settle_ok: Arc<Mutex<bool>>,
    verify_delay: Arc<Mutex<Option<Duration>>>,
    settle_delay: Arc<Mutex<Option<Duration>>>,
    settle_hold: Arc<Mutex<Option<SettleHold>>>,
    ready: Arc<Mutex<bool>>,
    verify_calls: Arc<Mutex<u64>>,
    settle_calls: Arc<Mutex<u64>>,
    network: Arc<Mutex<String>>,
    fee_payer: Arc<Mutex<Option<String>>>,
}

impl MockFacilitatorClient {
    pub fn new() -> Self {
        Self {
            verify_ok: Arc::new(Mutex::new(true)),
            settle_ok: Arc::new(Mutex::new(true)),
            verify_delay: Arc::new(Mutex::new(None)),
            settle_delay: Arc::new(Mutex::new(None)),
            settle_hold: Arc::new(Mutex::new(None)),
            ready: Arc::new(Mutex::new(true)),
            verify_calls: Arc::new(Mutex::new(0)),
            settle_calls: Arc::new(Mutex::new(0)),
            network: Arc::new(Mutex::new("solana:localnet".into())),
            fee_payer: Arc::new(Mutex::new(None)),
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
    /// Hold the next settle call until `release_settle_hold` is called.
    pub async fn arm_settle_hold(&self) -> Arc<Notify> {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        *self.settle_hold.lock().await = Some(SettleHold {
            entered: entered.clone(),
            release: release.clone(),
        });
        entered
    }
    pub async fn release_settle_hold(&self) {
        if let Some(hold) = self.settle_hold.lock().await.as_ref() {
            hold.release.notify_waiters();
        }
        *self.settle_hold.lock().await = None;
    }
    pub async fn set_ready(&self, ready: bool) {
        *self.ready.lock().await = ready;
    }
    pub async fn set_network(&self, network: impl Into<String>) {
        *self.network.lock().await = network.into();
    }
    pub async fn set_fee_payer(&self, fee_payer: Option<String>) {
        *self.fee_payer.lock().await = fee_payer;
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
        let hold = self.settle_hold.lock().await.clone();
        if let Some(hold) = hold {
            hold.entered.notify_waiters();
            hold.release.notified().await;
        }
        if let Some(delay) = *self.settle_delay.lock().await {
            tokio::time::sleep(delay).await;
            return Err(PaymentError::SettleTimeout);
        }
        if *self.settle_ok.lock().await {
            Ok(SettleResponse {
                success: true,
                error_reason: None,
                transaction: Some("mock-tx".into()),
                network: Some(self.network.lock().await.clone()),
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
        if !*self.ready.lock().await {
            return Err(PaymentError::Facilitator("not ready".into()));
        }
        let _ = (
            self.network.lock().await.clone(),
            self.fee_payer.lock().await.clone(),
        );
        Ok(())
    }
}
