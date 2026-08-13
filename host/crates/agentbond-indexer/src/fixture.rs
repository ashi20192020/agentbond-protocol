use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::{self, BoxStream};
use serde::Deserialize;
use solana_pubkey::Pubkey;

use agentbond_db::SlotUpdate;
use base64::Engine;

use crate::decode::{AccountDecodeInput, decode_account_update, extract_protocol_events};
use crate::error::IndexerError;
use crate::source::{AccountUpdate, ChainSource, ChainUpdate, parse_commitment};

#[derive(Clone, Debug, Deserialize)]
pub struct FixtureFile {
    pub program_id: String,
    pub updates: Vec<FixtureUpdate>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FixtureUpdate {
    Slot {
        slot: u64,
        parent_slot: Option<u64>,
        status: String,
        block_time: Option<i64>,
    },
    Account {
        address: String,
        slot: u64,
        write_version: u64,
        owner: Option<String>,
        lamports: u64,
        data_base64: Option<String>,
        deleted: bool,
        commitment: String,
    },
    Transaction {
        signature: String,
        slot: u64,
        logs: Vec<String>,
        commitment: String,
    },
}

pub struct FixtureSource {
    program: Pubkey,
    updates: Vec<FixtureUpdate>,
}

impl FixtureSource {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, IndexerError> {
        let text = std::fs::read_to_string(path.as_ref())
            .map_err(|e| IndexerError::Fixture(e.to_string()))?;
        Self::from_json(&text)
    }

    pub fn from_json(text: &str) -> Result<Self, IndexerError> {
        let file: FixtureFile =
            serde_json::from_str(text).map_err(|e| IndexerError::Fixture(e.to_string()))?;
        let program: Pubkey = file
            .program_id
            .parse()
            .map_err(|_| IndexerError::Fixture("bad program_id".into()))?;
        Ok(Self {
            program,
            updates: file.updates,
        })
    }

    pub fn into_updates(self) -> Result<Vec<ChainUpdate>, IndexerError> {
        let mut out = Vec::new();
        for item in self.updates {
            match item {
                FixtureUpdate::Slot {
                    slot,
                    parent_slot,
                    status,
                    block_time,
                } => out.push(ChainUpdate::Slot(SlotUpdate {
                    slot,
                    parent_slot,
                    status: parse_commitment(&status),
                    block_time,
                })),
                FixtureUpdate::Account {
                    address,
                    slot,
                    write_version,
                    owner,
                    lamports,
                    data_base64,
                    deleted,
                    commitment,
                } => {
                    let address: Pubkey = address
                        .parse()
                        .map_err(|_| IndexerError::Fixture("bad address".into()))?;
                    let owner = owner
                        .map(|o| o.parse::<Pubkey>())
                        .transpose()
                        .map_err(|_| IndexerError::Fixture("bad owner".into()))?;
                    let data = match data_base64 {
                        Some(b64) => Some(
                            Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
                                .map_err(|e| IndexerError::Fixture(e.to_string()))?,
                        ),
                        None => None,
                    };
                    let (raw, projection) = decode_account_update(AccountDecodeInput {
                        program: self.program,
                        address,
                        slot,
                        write_version,
                        owner,
                        lamports,
                        data,
                        deleted,
                        commitment: parse_commitment(&commitment),
                    })?;
                    out.push(ChainUpdate::Account(Box::new(AccountUpdate {
                        raw,
                        projection,
                    })));
                }
                FixtureUpdate::Transaction {
                    signature,
                    slot,
                    logs,
                    commitment,
                } => {
                    let events = extract_protocol_events(
                        &self.program,
                        &signature,
                        slot,
                        &logs,
                        parse_commitment(&commitment),
                    )?;
                    out.push(ChainUpdate::Events(events));
                }
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl ChainSource for FixtureSource {
    async fn subscribe(
        &self,
    ) -> Result<BoxStream<'static, Result<ChainUpdate, IndexerError>>, IndexerError> {
        let updates = self.clone().into_updates()?;
        Ok(Box::pin(stream::iter(updates.into_iter().map(Ok))))
    }
}

impl Clone for FixtureSource {
    fn clone(&self) -> Self {
        Self {
            program: self.program,
            updates: self.updates.clone(),
        }
    }
}

pub async fn replay_fixture(
    db: Arc<agentbond_db::Db>,
    path: impl AsRef<Path>,
    metrics: &crate::metrics::IndexerMetrics,
) -> Result<(), IndexerError> {
    let source = FixtureSource::from_path(path)?;
    let engine = crate::engine::IndexerEngine::new(db, metrics.clone());
    engine.run_source(&source).await
}
