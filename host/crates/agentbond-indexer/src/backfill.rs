use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use serde_json::Value;
use solana_pubkey::Pubkey;
use tracing::warn;

use agentbond_db::{Commitment, SlotUpdate};

use crate::decode::{AccountDecodeInput, decode_account_update, extract_protocol_events};
use crate::error::IndexerError;
use crate::source::{AccountUpdate, ChainUpdate};

/// Maximum slots repaired in one gap attempt.
pub const MAX_BACKFILL_SLOTS: u64 = 32;

#[async_trait]
pub trait GapBackfill: Send + Sync {
    async fn fetch_slot(&self, slot: u64) -> Result<Vec<ChainUpdate>, IndexerError>;
}

/// Offline/test backfill that records failure unless a fixture map is provided.
pub struct NullBackfill;

#[async_trait]
impl GapBackfill for NullBackfill {
    async fn fetch_slot(&self, slot: u64) -> Result<Vec<ChainUpdate>, IndexerError> {
        Err(IndexerError::Backfill(format!(
            "no backfill source configured for slot {slot}"
        )))
    }
}

/// In-memory backfill used by projection tests.
pub struct MapBackfill {
    pub slots: std::collections::HashMap<u64, Vec<ChainUpdate>>,
}

#[async_trait]
impl GapBackfill for MapBackfill {
    async fn fetch_slot(&self, slot: u64) -> Result<Vec<ChainUpdate>, IndexerError> {
        self.slots
            .get(&slot)
            .cloned()
            .ok_or_else(|| IndexerError::Backfill(format!("fixture backfill missing slot {slot}")))
    }
}

/// Bounded JSON-RPC `getBlock` backfill. Never contacts the network from unit tests.
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
        let v: Value = resp
            .json()
            .await
            .map_err(|e| IndexerError::Backfill(e.to_string()))?;
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
            parent_slot: result
                .get("previousBlockhash")
                .and_then(|_| result.get("parentSlot"))
                .and_then(|v| v.as_u64()),
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

            if let Some(accounts) = meta
                .and_then(|m| m.get("postAccounts"))
                .and_then(|v| v.as_array())
            {
                for (idx, acc) in accounts.iter().enumerate() {
                    let Some(pubkey_str) = acc.get("pubkey").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let Ok(address) = pubkey_str.parse::<Pubkey>() else {
                        continue;
                    };
                    let owner = acc
                        .get("owner")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<Pubkey>().ok());
                    let lamports = acc.get("lamports").and_then(|v| v.as_u64()).unwrap_or(0);
                    let data_b64 = acc
                        .get("data")
                        .and_then(|v| v.as_array())
                        .and_then(|a| a.first())
                        .and_then(|v| v.as_str());
                    let data = match data_b64 {
                        Some(b64) => {
                            match Engine::decode(&base64::engine::general_purpose::STANDARD, b64) {
                                Ok(bytes) => Some(bytes),
                                Err(_) => continue,
                            }
                        }
                        None => None,
                    };
                    let deleted =
                        lamports == 0 && data.as_ref().map(|d| d.is_empty()).unwrap_or(true);
                    match decode_account_update(AccountDecodeInput {
                        program: self.program_id,
                        address,
                        slot,
                        write_version: idx as u64,
                        owner,
                        lamports,
                        data,
                        deleted,
                        commitment: Commitment::Confirmed,
                    }) {
                        Ok((raw, projection)) => {
                            out.push(ChainUpdate::Account(Box::new(AccountUpdate {
                                raw,
                                projection,
                            })));
                        }
                        Err(e) => warn!(error = %e, "backfill account decode skipped"),
                    }
                }
            }
        }
        Ok(out)
    }
}
