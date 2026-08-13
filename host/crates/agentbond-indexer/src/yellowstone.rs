use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::{StreamExt, stream::BoxStream};
use rand::Rng;
use solana_pubkey::Pubkey;
use tokio::sync::mpsc;
use tracing::{info, warn};
use url::Url;
use yellowstone_grpc_client::{ClientTlsConfig, GeyserGrpcClient};
use yellowstone_grpc_proto::geyser::{
    CommitmentLevel, SubscribeRequest, SubscribeRequestFilterAccounts, SubscribeRequestFilterSlots,
    SubscribeRequestFilterTransactions, subscribe_update::UpdateOneof,
};

use agentbond_db::{Commitment, ProjectionRepo, SlotUpdate};

use crate::decode::{AccountDecodeInput, decode_account_update, extract_protocol_events};
use crate::error::IndexerError;
use crate::metrics::IndexerMetrics;
use crate::source::{AccountUpdate, ChainSource, ChainUpdate};

#[derive(Clone, Debug)]
pub struct YellowstoneConfig {
    pub url: String,
    pub x_token: Option<String>,
    pub program_id: Pubkey,
    pub connect_timeout: Duration,
    pub from_slot: Option<u64>,
}

impl YellowstoneConfig {
    pub fn from_env() -> Result<Self, IndexerError> {
        let url = std::env::var("AGENTBOND_YELLOWSTONE_URL")
            .map_err(|_| IndexerError::Config("AGENTBOND_YELLOWSTONE_URL required".into()))?;
        validate_yellowstone_url(&url)?;
        let x_token = std::env::var("AGENTBOND_YELLOWSTONE_X_TOKEN").ok();
        let program_id = std::env::var("AGENTBOND_PROGRAM_ID")
            .map_err(|_| IndexerError::Config("AGENTBOND_PROGRAM_ID required".into()))?
            .parse()
            .map_err(|_| IndexerError::Config("bad AGENTBOND_PROGRAM_ID".into()))?;
        Ok(Self {
            url,
            x_token,
            program_id,
            connect_timeout: Duration::from_secs(10),
            from_slot: None,
        })
    }

    pub fn with_from_slot(mut self, slot: Option<u64>) -> Self {
        self.from_slot = slot;
        self
    }
}

/// Supplies the latest finalized checkpoint before each Yellowstone session.
#[async_trait]
pub trait CheckpointProvider: Send + Sync {
    async fn finalized_slot(&self) -> Result<Option<u64>, IndexerError>;
}

pub struct DbCheckpointProvider {
    repo: ProjectionRepo,
}

impl DbCheckpointProvider {
    pub fn new(repo: ProjectionRepo) -> Self {
        Self { repo }
    }
}

#[async_trait]
impl CheckpointProvider for DbCheckpointProvider {
    async fn finalized_slot(&self) -> Result<Option<u64>, IndexerError> {
        let (finalized, _) = self.repo.checkpoint().await?;
        Ok(if finalized > 0 { Some(finalized) } else { None })
    }
}

/// Read the latest checkpoint and build a subscribe request (offline-testable).
pub async fn subscribe_request_for_checkpoint(
    cfg: &YellowstoneConfig,
    checkpoints: &dyn CheckpointProvider,
) -> Result<SubscribeRequest, IndexerError> {
    let from_slot = checkpoints.finalized_slot().await?;
    let mut cfg = cfg.clone();
    cfg.from_slot = from_slot;
    Ok(build_subscribe_request(&cfg))
}

pub fn validate_yellowstone_url(url: &str) -> Result<(), IndexerError> {
    let parsed = Url::parse(url).map_err(|e| IndexerError::Config(e.to_string()))?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(IndexerError::Config(
            "Yellowstone URL must not embed credentials".into(),
        ));
    }
    let host = parsed.host_str().unwrap_or("");
    let loopback = matches!(host, "127.0.0.1" | "localhost" | "::1");
    match parsed.scheme() {
        "http" if loopback => Ok(()),
        "https" => Ok(()),
        "http" => Err(IndexerError::Config(
            "non-loopback Yellowstone endpoints require https/TLS".into(),
        )),
        _ => Err(IndexerError::Config(
            "Yellowstone URL must be http(s)".into(),
        )),
    }
}

/// Build a subscribe request for tests / production (no network I/O).
pub fn build_subscribe_request(cfg: &YellowstoneConfig) -> SubscribeRequest {
    let mut accounts = HashMap::new();
    accounts.insert(
        "agentbond".into(),
        SubscribeRequestFilterAccounts {
            account: vec![],
            owner: vec![cfg.program_id.to_string()],
            filters: vec![],
            nonempty_txn_signature: None,
        },
    );
    let mut slots = HashMap::new();
    slots.insert(
        "slots".into(),
        SubscribeRequestFilterSlots {
            filter_by_commitment: Some(true),
            interslot_updates: Some(false),
        },
    );
    let mut transactions = HashMap::new();
    transactions.insert(
        "txs".into(),
        SubscribeRequestFilterTransactions {
            vote: Some(false),
            failed: Some(false),
            signature: None,
            account_include: vec![cfg.program_id.to_string()],
            account_exclude: vec![],
            account_required: vec![],
        },
    );
    SubscribeRequest {
        accounts,
        slots,
        transactions,
        transactions_status: HashMap::new(),
        blocks: HashMap::new(),
        blocks_meta: HashMap::new(),
        entry: HashMap::new(),
        commitment: Some(CommitmentLevel::Processed as i32),
        accounts_data_slice: vec![],
        ping: None,
        from_slot: cfg.from_slot,
    }
}

pub struct YellowstoneSource {
    cfg: YellowstoneConfig,
    metrics: IndexerMetrics,
    checkpoints: Arc<dyn CheckpointProvider>,
}

impl YellowstoneSource {
    pub fn new(
        cfg: YellowstoneConfig,
        metrics: IndexerMetrics,
        checkpoints: Arc<dyn CheckpointProvider>,
    ) -> Self {
        Self {
            cfg,
            metrics,
            checkpoints,
        }
    }
}

#[async_trait]
impl ChainSource for YellowstoneSource {
    async fn subscribe(
        &self,
    ) -> Result<BoxStream<'static, Result<ChainUpdate, IndexerError>>, IndexerError> {
        let (tx, rx) = mpsc::channel(256);
        let cfg = self.cfg.clone();
        let metrics = self.metrics.clone();
        let checkpoints = self.checkpoints.clone();
        tokio::spawn(async move {
            let mut attempt = 0u32;
            loop {
                if tx.is_closed() {
                    break;
                }
                match run_session(&cfg, checkpoints.as_ref(), &metrics, &tx).await {
                    Ok(()) => {
                        // Successful session ended; reset backoff before reconnect if still open.
                        attempt = 0;
                        if tx.is_closed() {
                            break;
                        }
                        metrics.reconnect_count.inc();
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                    Err(e) => {
                        if tx.is_closed() {
                            break;
                        }
                        metrics.reconnect_count.inc();
                        attempt = attempt.saturating_add(1);
                        let backoff = bounded_backoff(attempt);
                        warn!(error = %e, attempt, ?backoff, "yellowstone reconnect");
                        tokio::time::sleep(backoff).await;
                    }
                }
            }
        });
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}

fn bounded_backoff(attempt: u32) -> Duration {
    let base_ms = 500u64.saturating_mul(1u64 << attempt.min(6));
    let capped = base_ms.min(30_000);
    let jitter = rand::rng().random_range(0..capped / 4 + 1);
    Duration::from_millis(capped + jitter)
}

async fn run_session(
    cfg: &YellowstoneConfig,
    checkpoints: &dyn CheckpointProvider,
    metrics: &IndexerMetrics,
    tx: &mpsc::Sender<Result<ChainUpdate, IndexerError>>,
) -> Result<(), IndexerError> {
    let request = subscribe_request_for_checkpoint(cfg, checkpoints).await?;
    info!(from_slot = ?request.from_slot, "connecting yellowstone");
    let mut builder = GeyserGrpcClient::build_from_shared(cfg.url.clone())
        .map_err(|e| IndexerError::Source(e.to_string()))?
        .connect_timeout(cfg.connect_timeout)
        .timeout(Duration::from_secs(30));
    if let Some(token) = &cfg.x_token {
        builder = builder
            .x_token(Some(token.clone()))
            .map_err(|e| IndexerError::Source(e.to_string()))?;
    }
    if cfg.url.starts_with("https://") {
        builder = builder
            .tls_config(ClientTlsConfig::new().with_native_roots())
            .map_err(|e| IndexerError::Source(e.to_string()))?;
    }
    let mut client = builder
        .connect()
        .await
        .map_err(|e| IndexerError::Source(e.to_string()))?;

    let (_sink, mut stream) = client
        .subscribe_with_request(Some(request))
        .await
        .map_err(|e| IndexerError::Source(e.to_string()))?;

    while let Some(msg) = stream.next().await {
        if tx.is_closed() {
            return Ok(());
        }
        let update = msg.map_err(|e| IndexerError::Source(e.to_string()))?;
        // Counting happens in the engine to avoid double-count.
        let Some(oneof) = update.update_oneof else {
            continue;
        };
        let chain = match oneof {
            UpdateOneof::Slot(slot) => {
                let status = match slot.status() {
                    yellowstone_grpc_proto::geyser::SlotStatus::SlotFinalized => {
                        Commitment::Finalized
                    }
                    yellowstone_grpc_proto::geyser::SlotStatus::SlotConfirmed => {
                        Commitment::Confirmed
                    }
                    yellowstone_grpc_proto::geyser::SlotStatus::SlotDead => Commitment::Dead,
                    _ => Commitment::Processed,
                };
                Some(ChainUpdate::Slot(SlotUpdate {
                    slot: slot.slot,
                    parent_slot: slot.parent,
                    status,
                    block_time: None,
                }))
            }
            UpdateOneof::Account(acc) => {
                let Some(info) = acc.account else {
                    continue;
                };
                let address = Pubkey::try_from(info.pubkey.as_slice())
                    .map_err(|_| IndexerError::Decode("bad account pubkey".into()))?;
                let owner = if info.owner.is_empty() {
                    None
                } else {
                    Some(
                        Pubkey::try_from(info.owner.as_slice())
                            .map_err(|_| IndexerError::Decode("bad owner".into()))?,
                    )
                };
                let deleted = info.lamports == 0 && info.data.is_empty();
                match decode_account_update(AccountDecodeInput {
                    program: cfg.program_id,
                    address,
                    slot: acc.slot,
                    write_version: info.write_version,
                    owner,
                    lamports: info.lamports,
                    data: Some(info.data),
                    deleted,
                    commitment: Commitment::Processed,
                }) {
                    Ok((raw, projection)) => Some(ChainUpdate::Account(Box::new(AccountUpdate {
                        raw,
                        projection,
                    }))),
                    Err(e) => {
                        metrics.decode_failures.inc();
                        warn!(error = %e, "account decode skipped");
                        None
                    }
                }
            }
            UpdateOneof::Transaction(tx_update) => {
                let Some(info) = tx_update.transaction else {
                    continue;
                };
                let signature = bs58::encode(&info.signature).into_string();
                let logs = info
                    .meta
                    .as_ref()
                    .map(|m| m.log_messages.clone())
                    .unwrap_or_default();
                match extract_protocol_events(
                    &cfg.program_id,
                    &signature,
                    tx_update.slot,
                    &logs,
                    Commitment::Processed,
                ) {
                    Ok(events) if !events.is_empty() => Some(ChainUpdate::Events(events)),
                    Ok(_) => None,
                    Err(e) => {
                        metrics.decode_failures.inc();
                        warn!(error = %e, "event decode skipped");
                        None
                    }
                }
            }
            _ => None,
        };
        if let Some(update) = chain
            && tx.send(Ok(update)).await.is_err()
        {
            return Ok(());
        }
    }
    Err(IndexerError::Source("yellowstone stream ended".into()))
}

mod tokio_stream {
    pub mod wrappers {
        use std::pin::Pin;
        use std::task::{Context, Poll};

        use futures::Stream;
        use tokio::sync::mpsc::Receiver;

        pub struct ReceiverStream<T> {
            inner: Receiver<T>,
        }

        impl<T> ReceiverStream<T> {
            pub fn new(inner: Receiver<T>) -> Self {
                Self { inner }
            }
        }

        impl<T> Stream for ReceiverStream<T> {
            type Item = T;
            fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
                self.inner.poll_recv(cx)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_pubkey::Pubkey;
    use tokio::sync::Mutex;

    struct MemCheckpoint {
        slot: Mutex<Option<u64>>,
    }

    #[async_trait]
    impl CheckpointProvider for MemCheckpoint {
        async fn finalized_slot(&self) -> Result<Option<u64>, IndexerError> {
            Ok(*self.slot.lock().await)
        }
    }

    #[test]
    fn subscribe_request_includes_from_slot() {
        let cfg = YellowstoneConfig {
            url: "http://127.0.0.1:10000".into(),
            x_token: None,
            program_id: Pubkey::new_from_array([7u8; 32]),
            connect_timeout: Duration::from_secs(1),
            from_slot: Some(42),
        };
        let req = build_subscribe_request(&cfg);
        assert_eq!(req.from_slot, Some(42));
        assert!(req.accounts.contains_key("agentbond"));
    }

    #[tokio::test]
    async fn second_session_uses_advanced_checkpoint() {
        let cfg = YellowstoneConfig {
            url: "http://127.0.0.1:10000".into(),
            x_token: None,
            program_id: Pubkey::new_from_array([7u8; 32]),
            connect_timeout: Duration::from_secs(1),
            from_slot: None,
        };
        let cp = MemCheckpoint {
            slot: Mutex::new(Some(10)),
        };
        let first = subscribe_request_for_checkpoint(&cfg, &cp)
            .await
            .expect("first");
        assert_eq!(first.from_slot, Some(10));
        *cp.slot.lock().await = Some(50);
        let second = subscribe_request_for_checkpoint(&cfg, &cp)
            .await
            .expect("second");
        assert_eq!(second.from_slot, Some(50));
    }
}
