use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use serde_json::Value;
use solana_pubkey::Pubkey;
use tracing::warn;

use agentbond_db::{Commitment, SlotUpdate};

use crate::decode::extract_protocol_events;
use crate::error::IndexerError;
use crate::source::ChainUpdate;

/// Maximum slots repaired in one gap attempt.
pub const MAX_BACKFILL_SLOTS: u64 = 32;
pub const MAX_RPC_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_GPA_ACCOUNTS: usize = 256;

#[async_trait]
pub trait GapBackfill: Send + Sync {
    async fn fetch_slot(&self, slot: u64) -> Result<Vec<ChainUpdate>, IndexerError>;
    /// Returns true only when account projections were reconciled for the range.
    async fn reconcile_accounts(&self, from: u64, to: u64) -> Result<bool, IndexerError>;
}

pub struct NullBackfill;

#[async_trait]
impl GapBackfill for NullBackfill {
    async fn fetch_slot(&self, slot: u64) -> Result<Vec<ChainUpdate>, IndexerError> {
        Err(IndexerError::Backfill(format!(
            "no backfill source configured for slot {slot}"
        )))
    }

    async fn reconcile_accounts(&self, _from: u64, _to: u64) -> Result<bool, IndexerError> {
        Ok(false)
    }
}

pub struct MapBackfill {
    pub slots: std::collections::HashMap<u64, Vec<ChainUpdate>>,
    pub accounts_reconciled: bool,
}

#[async_trait]
impl GapBackfill for MapBackfill {
    async fn fetch_slot(&self, slot: u64) -> Result<Vec<ChainUpdate>, IndexerError> {
        self.slots
            .get(&slot)
            .cloned()
            .ok_or_else(|| IndexerError::Backfill(format!("fixture backfill missing slot {slot}")))
    }

    async fn reconcile_accounts(&self, _from: u64, _to: u64) -> Result<bool, IndexerError> {
        Ok(self.accounts_reconciled)
    }
}

/// Bounded JSON-RPC backfill: `getBlock` for events only.
/// Account repair requires a separate bounded `getProgramAccounts` reconcile.
pub struct RpcGapBackfill {
    http: reqwest::Client,
    rpc_url: String,
    program_id: Pubkey,
}

impl RpcGapBackfill {
    pub fn new(rpc_url: &str, program_id: Pubkey, timeout: Duration) -> Result<Self, IndexerError> {
        let url = url::Url::parse(rpc_url)
            .map_err(|e| IndexerError::Config(format!("invalid AGENTBOND_RPC_URL: {e}")))?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err(IndexerError::Config(
                "AGENTBOND_RPC_URL must not embed credentials".into(),
            ));
        }
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(timeout)
            .build()
            .map_err(|e| IndexerError::Config(e.to_string()))?;
        Ok(Self {
            http,
            rpc_url: rpc_url.to_string(),
            program_id,
        })
    }

    async fn rpc(&self, method: &str, params: Value) -> Result<Value, IndexerError> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        let resp = self
            .http
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| IndexerError::Backfill(e.to_string()))?;
        let status = resp.status();
        let bytes = read_body_bounded(resp, MAX_RPC_BYTES).await?;
        let v: Value =
            serde_json::from_slice(&bytes).map_err(|e| IndexerError::Backfill(e.to_string()))?;
        if !status.is_success() {
            return Err(IndexerError::Backfill(format!("rpc http {status}")));
        }
        if let Some(err) = v.get("error") {
            return Err(IndexerError::Backfill(err.to_string()));
        }
        v.get("result")
            .cloned()
            .ok_or_else(|| IndexerError::Backfill("rpc missing result".into()))
    }
}

/// Stream an HTTP body and stop as soon as it exceeds `max_bytes`.
pub async fn read_body_bounded(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<Bytes, IndexerError> {
    let mut out = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| IndexerError::Backfill(e.to_string()))?;
        if out.len().saturating_add(chunk.len()) > max_bytes {
            return Err(IndexerError::Backfill("rpc response too large".into()));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(out))
}

#[async_trait]
impl GapBackfill for RpcGapBackfill {
    async fn fetch_slot(&self, slot: u64) -> Result<Vec<ChainUpdate>, IndexerError> {
        let result = self
            .rpc(
                "getBlock",
                serde_json::json!([
                    slot,
                    {
                        "encoding": "json",
                        "transactionDetails": "full",
                        "rewards": false,
                        "maxSupportedTransactionVersion": 0
                    }
                ]),
            )
            .await?;
        if result.is_null() {
            return Err(IndexerError::Backfill(format!("getBlock null for {slot}")));
        }
        let mut out = vec![ChainUpdate::Slot(SlotUpdate {
            slot,
            parent_slot: result.get("parentSlot").and_then(|v| v.as_u64()),
            status: Commitment::Confirmed,
            block_time: result.get("blockTime").and_then(|v| v.as_i64()),
        })];

        let Some(txs) = result.get("transactions").and_then(|v| v.as_array()) else {
            return Ok(out);
        };
        for tx in txs {
            let meta = tx.get("meta");
            let err = meta.and_then(|m| m.get("err"));
            if err.map(|e| !e.is_null()).unwrap_or(false) {
                continue;
            }
            let logs = meta
                .and_then(|m| m.get("logMessages"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let signature = tx
                .get("transaction")
                .and_then(|t| t.get("signatures"))
                .and_then(|s| s.as_array())
                .and_then(|a| a.first())
                .and_then(|s| s.as_str())
                .unwrap_or("backfill-missing-signature")
                .to_string();
            match extract_protocol_events(
                &self.program_id,
                &signature,
                slot,
                &logs,
                Commitment::Confirmed,
            ) {
                Ok(events) if !events.is_empty() => out.push(ChainUpdate::Events(events)),
                Ok(_) => {}
                Err(e) => warn!(error = %e, slot, "backfill event extract skipped"),
            }
        }
        Ok(out)
    }

    async fn reconcile_accounts(&self, _from: u64, _to: u64) -> Result<bool, IndexerError> {
        // Bounded GPA probe: success means we obtained a finite account set for the program.
        // Slot-accurate historical account state is not available from getBlock; GPA is tip-state.
        // We therefore never claim full historical account repair for arbitrary past gaps.
        let result = self
            .rpc(
                "getProgramAccounts",
                serde_json::json!([
                    self.program_id.to_string(),
                    {
                        "encoding": "base64",
                        "dataSlice": { "offset": 0, "length": 0 }
                    }
                ]),
            )
            .await;
        match result {
            Ok(Value::Array(arr)) if arr.len() <= MAX_GPA_ACCOUNTS => {
                // Tip-state GPA cannot safely reconstruct historical slot projections.
                let _ = arr;
                Ok(false)
            }
            Ok(Value::Array(arr)) => Err(IndexerError::Backfill(format!(
                "getProgramAccounts returned {} accounts (max {MAX_GPA_ACCOUNTS})",
                arr.len()
            ))),
            Ok(_) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn oversized_streaming_rpc_body_is_rejected() {
        let server = MockServer::start().await;
        let huge = vec![b'x'; MAX_RPC_BYTES.saturating_add(1)];
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(huge))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let resp = client
            .post(server.uri())
            .body("{}")
            .send()
            .await
            .expect("send");
        let err = read_body_bounded(resp, MAX_RPC_BYTES)
            .await
            .expect_err("oversized");
        assert!(
            err.to_string().contains("too large"),
            "unexpected error: {err}"
        );
    }
}
