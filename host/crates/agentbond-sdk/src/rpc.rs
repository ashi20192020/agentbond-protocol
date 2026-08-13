use async_trait::async_trait;
use serde::Deserialize;
use solana_pubkey::Pubkey;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use crate::error::SdkError;
use crate::http_util::{build_http_client, read_body_bounded, reject_credentialed_url};

pub const MAX_RPC_BODY_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug)]
pub struct AccountData {
    pub owner: Pubkey,
    pub data: Vec<u8>,
    pub lamports: u64,
}

#[async_trait]
pub trait ChainReader: Send + Sync {
    async fn get_account(&self, address: &Pubkey) -> Result<Option<AccountData>, SdkError>;
    async fn get_unix_timestamp(&self) -> Result<i64, SdkError>;
    async fn ready(&self) -> Result<(), SdkError>;
}

pub struct HttpChainReader {
    client: reqwest::Client,
    rpc_url: String,
}

impl HttpChainReader {
    pub fn new(rpc_url: impl Into<String>, timeout: Duration) -> Result<Self, SdkError> {
        let rpc_url = rpc_url.into();
        reject_credentialed_url(&rpc_url)?;
        let client = build_http_client(timeout)?;
        Ok(Self { client, rpc_url })
    }

    pub(crate) async fn rpc_call<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<T, SdkError> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        let response = self
            .client
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    SdkError::Rpc("rpc timeout".into())
                } else {
                    SdkError::Rpc(e.to_string())
                }
            })?;
        if response.status().is_redirection() {
            return Err(SdkError::Rpc("rpc redirects are not allowed".into()));
        }
        if !response.status().is_success() {
            return Err(SdkError::Rpc(format!("http status {}", response.status())));
        }
        let bytes = read_body_bounded(response, MAX_RPC_BODY_BYTES).await?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| SdkError::Rpc(e.to_string()))?;
        if let Some(err) = value.get("error") {
            return Err(SdkError::Rpc(err.to_string()));
        }
        serde_json::from_value(value["result"].clone()).map_err(|e| SdkError::Rpc(e.to_string()))
    }

    pub(crate) async fn rpc_call_string(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<String, SdkError> {
        self.rpc_call(method, params).await
    }
}

#[async_trait]
impl ChainReader for HttpChainReader {
    async fn get_account(&self, address: &Pubkey) -> Result<Option<AccountData>, SdkError> {
        #[derive(Deserialize)]
        struct Value {
            value: Option<UiAccount>,
        }
        #[derive(Deserialize)]
        struct UiAccount {
            lamports: u64,
            owner: String,
            data: (String, String),
        }
        let result: Value = self
            .rpc_call(
                "getAccountInfo",
                serde_json::json!([
                    address.to_string(),
                    { "encoding": "base64" }
                ]),
            )
            .await?;
        let Some(acc) = result.value else {
            return Ok(None);
        };
        let owner = acc
            .owner
            .parse::<Pubkey>()
            .map_err(|_| SdkError::InvalidPubkey(acc.owner))?;
        let data = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &acc.data.0)
            .map_err(|e| SdkError::Rpc(format!("account data base64: {e}")))?;
        Ok(Some(AccountData {
            owner,
            data,
            lamports: acc.lamports,
        }))
    }

    async fn get_unix_timestamp(&self) -> Result<i64, SdkError> {
        let slot: u64 = self.rpc_call("getSlot", serde_json::json!([])).await?;
        let ts: Option<i64> = self
            .rpc_call("getBlockTime", serde_json::json!([slot]))
            .await?;
        ts.ok_or_else(|| SdkError::Rpc("block time unavailable".into()))
    }

    async fn ready(&self) -> Result<(), SdkError> {
        let _: u64 = self.rpc_call("getSlot", serde_json::json!([])).await?;
        Ok(())
    }
}

#[derive(Default)]
pub struct MockChainReader {
    accounts: Mutex<HashMap<Pubkey, AccountData>>,
    timestamp: Mutex<i64>,
    ready: Mutex<bool>,
}

impl MockChainReader {
    pub fn new() -> Self {
        Self {
            accounts: Mutex::new(HashMap::new()),
            timestamp: Mutex::new(1_700_000_000),
            ready: Mutex::new(true),
        }
    }

    pub async fn set_account(&self, address: Pubkey, account: AccountData) {
        self.accounts.lock().await.insert(address, account);
    }

    pub async fn set_timestamp(&self, ts: i64) {
        *self.timestamp.lock().await = ts;
    }

    pub async fn set_ready(&self, ready: bool) {
        *self.ready.lock().await = ready;
    }

    pub fn shared(self) -> Arc<Self> {
        Arc::new(self)
    }
}

#[async_trait]
impl ChainReader for MockChainReader {
    async fn get_account(&self, address: &Pubkey) -> Result<Option<AccountData>, SdkError> {
        Ok(self.accounts.lock().await.get(address).cloned())
    }

    async fn get_unix_timestamp(&self) -> Result<i64, SdkError> {
        Ok(*self.timestamp.lock().await)
    }

    async fn ready(&self) -> Result<(), SdkError> {
        if *self.ready.lock().await {
            Ok(())
        } else {
            Err(SdkError::Rpc("mock rpc not ready".into()))
        }
    }
}
