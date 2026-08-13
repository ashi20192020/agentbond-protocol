use std::collections::HashMap;
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

use agentbond_db::{Commitment, SlotUpdate};

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
}

impl YellowstoneConfig {
    pub fn from_env() -> Result<Self, IndexerError> {
        let url = std::env::var("AGENTBOND_YELLOWSTONE_URL")
            .map_err(|_| IndexerError::Config("AGENTBOND_YELLOWSTONE_URL required".into()))?;
        validate_yellowstone_url(&url)?;
        let x_token = std::env::var("AGENTBOND_YELLOWSTONE_X_TOKEN").ok();
        let program_id = std::env::var("AGENTBOND_PROGRAM_ID")
            .unwrap_or_else(|_| agentbond_sdk::program_id().to_string())
            .parse()
            .map_err(|_| IndexerError::Config("bad AGENTBOND_PROGRAM_ID".into()))?;
        Ok(Self {
            url,
            x_token,
            program_id,
            connect_timeout: Duration::from_secs(10),
        })
    }
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

pub struct YellowstoneSource {
    cfg: YellowstoneConfig,
    metrics: IndexerMetrics,
}

impl YellowstoneSource {
    pub fn new(cfg: YellowstoneConfig, metrics: IndexerMetrics) -> Self {
        Self { cfg, metrics }
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
        tokio::spawn(async move {
            let mut attempt = 0u32;
            loop {
                match run_session(&cfg, &metrics, tx.clone()).await {
                    Ok(()) => break,
                    Err(e) => {
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
    metrics: &IndexerMetrics,
    tx: mpsc::Sender<Result<ChainUpdate, IndexerError>>,
) -> Result<(), IndexerError> {
    info!("connecting yellowstone");
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
    let request = SubscribeRequest {
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
        from_slot: None,
    };

    let (_sink, mut stream) = client
        .subscribe_with_request(Some(request))
        .await
        .map_err(|e| IndexerError::Source(e.to_string()))?;

    while let Some(msg) = stream.next().await {
        let update = msg.map_err(|e| IndexerError::Source(e.to_string()))?;
        metrics.received_updates.inc();
        let Some(oneof) = update.update_oneof else {
            continue;
        };
        match oneof {
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
                let _ = tx
                    .send(Ok(ChainUpdate::Slot(SlotUpdate {
                        slot: slot.slot,
                        parent_slot: slot.parent,
                        status,
                        block_time: None,
                    })))
                    .await;
            }
            UpdateOneof::Account(acc) => {
                let Some(info) = acc.account else { continue };
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
                    Ok((raw, projection)) => {
                        let _ = tx
                            .send(Ok(ChainUpdate::Account(Box::new(AccountUpdate {
                                raw,
                                projection,
                            }))))
                            .await;
                    }
                    Err(e) => {
                        metrics.decode_failures.inc();
                        warn!(error = %e, "account decode skipped");
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
                    Ok(events) => {
                        if !events.is_empty() {
                            metrics.decoded_events.inc_by(events.len() as u64);
                            let _ = tx.send(Ok(ChainUpdate::Events(events))).await;
                        }
                    }
                    Err(e) => {
                        metrics.decode_failures.inc();
                        warn!(error = %e, "event decode skipped");
                    }
                }
            }
            _ => {}
        }
    }
    Err(IndexerError::Source("yellowstone stream ended".into()))
}

// tokio-stream is used for ReceiverStream — add dependency or use manual stream.
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
