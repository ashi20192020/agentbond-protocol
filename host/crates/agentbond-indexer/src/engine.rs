use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use tracing::{info, warn};

use agentbond_db::{Commitment, Db, DecodedProjection, ProjectionRepo};

use crate::backfill::{GapBackfill, MAX_BACKFILL_SLOTS, NullBackfill};
use crate::error::IndexerError;
use crate::metrics::IndexerMetrics;
use crate::source::{AccountUpdate, ChainSource, ChainUpdate};

pub struct IndexerEngine {
    db: Arc<Db>,
    metrics: IndexerMetrics,
    backfill: Arc<dyn GapBackfill>,
}

impl IndexerEngine {
    pub fn new(db: Arc<Db>, metrics: IndexerMetrics) -> Self {
        Self {
            db,
            metrics,
            backfill: Arc::new(NullBackfill),
        }
    }

    pub fn with_backfill(mut self, backfill: Arc<dyn GapBackfill>) -> Self {
        self.backfill = backfill;
        self
    }

    pub async fn run_source(&self, source: &dyn ChainSource) -> Result<(), IndexerError> {
        let repo = ProjectionRepo::new(self.db.pool().clone());
        let mut stream = source.subscribe().await?;
        let mut pending: HashMap<u64, Vec<DecodedProjection>> = HashMap::new();
        let mut last_slot: Option<u64> = None;

        while let Some(item) = stream.next().await {
            let update = match item {
                Ok(u) => u,
                Err(e) => {
                    self.metrics.database_errors.inc();
                    return Err(e);
                }
            };
            if let Some((from, to)) = self
                .ingest_update(&repo, &mut pending, &mut last_slot, update)
                .await?
            {
                self.repair_gap(&repo, &mut pending, &mut last_slot, from, to)
                    .await?;
            }
        }
        info!("indexer source completed");
        Ok(())
    }

    /// Returns `Some((from,to))` when a gap should be repaired.
    async fn ingest_update(
        &self,
        repo: &ProjectionRepo,
        pending: &mut HashMap<u64, Vec<DecodedProjection>>,
        last_slot: &mut Option<u64>,
        update: ChainUpdate,
    ) -> Result<Option<(u64, u64)>, IndexerError> {
        self.metrics.received_updates.inc();
        let mut gap = None;
        match update {
            ChainUpdate::Slot(slot) => {
                if let Some(prev) = *last_slot
                    && slot.slot > prev + 1
                {
                    let from = prev + 1;
                    let to = slot.slot - 1;
                    repo.record_gap(from, to).await?;
                    self.metrics.detected_gaps.inc();
                    warn!(from, to, "ingestion gap detected");
                    gap = Some((from, to));
                }
                *last_slot = Some(slot.slot);
                if let Err(e) = repo.upsert_slot(&slot).await {
                    self.metrics.database_errors.inc();
                    return Err(e.into());
                }
                if slot.status == Commitment::Finalized {
                    let projections = pending.remove(&slot.slot).unwrap_or_default();
                    if let Err(e) = repo.finalize_slot(slot.slot, &projections).await {
                        self.metrics.database_errors.inc();
                        return Err(e.into());
                    }
                    self.metrics
                        .finalized_projections
                        .inc_by(projections.len() as u64);
                    self.metrics.current_finalized_slot.set(slot.slot as i64);
                    if let Ok((finalized, _)) = repo.checkpoint().await {
                        self.metrics.checkpoint_slot.set(finalized as i64);
                    }
                } else if slot.status == Commitment::Dead {
                    pending.remove(&slot.slot);
                }
            }
            ChainUpdate::Account(account) => {
                let AccountUpdate { raw, projection } = *account;
                match repo.insert_account_version(&raw).await {
                    Ok(true) => {}
                    Ok(false) => self.metrics.duplicate_updates.inc(),
                    Err(e) => {
                        self.metrics.database_errors.inc();
                        return Err(e.into());
                    }
                }
                if let Some(p) = projection {
                    if raw.commitment == Commitment::Finalized {
                        if let Err(e) = repo.finalize_slot(raw.slot, std::slice::from_ref(&p)).await
                        {
                            self.metrics.database_errors.inc();
                            return Err(e.into());
                        }
                        self.metrics.finalized_projections.inc();
                    } else {
                        pending.entry(raw.slot).or_default().push(p);
                    }
                }
            }
            ChainUpdate::Events(events) => {
                for ev in events {
                    match repo.insert_event(&ev).await {
                        Ok(true) => self.metrics.decoded_events.inc(),
                        Ok(false) => self.metrics.duplicate_updates.inc(),
                        Err(e) => {
                            self.metrics.database_errors.inc();
                            return Err(e.into());
                        }
                    }
                }
            }
        }
        Ok(gap)
    }

    async fn repair_gap(
        &self,
        repo: &ProjectionRepo,
        pending: &mut HashMap<u64, Vec<DecodedProjection>>,
        last_slot: &mut Option<u64>,
        from: u64,
        to: u64,
    ) -> Result<(), IndexerError> {
        let span = to.saturating_sub(from).saturating_add(1);
        if span > MAX_BACKFILL_SLOTS {
            let msg = format!("gap {from}..={to} exceeds backfill bound {MAX_BACKFILL_SLOTS}");
            warn!("{msg}");
            repo.mark_gap_failed(from, to, &msg).await?;
            return Ok(());
        }
        for slot in from..=to {
            match self.backfill.fetch_slot(slot).await {
                Ok(updates) => {
                    for update in updates {
                        let nested = self.ingest_update(repo, pending, last_slot, update).await?;
                        if nested.is_some() {
                            return Err(IndexerError::Backfill(
                                "nested gap during backfill is unsupported".into(),
                            ));
                        }
                    }
                }
                Err(e) => {
                    warn!(slot, error = %e, "gap backfill failed");
                    repo.mark_gap_failed(from, to, &e.to_string()).await?;
                    return Ok(());
                }
            }
        }
        repo.mark_gap_repaired(from, to).await?;
        info!(from, to, "gap repaired via backfill");
        Ok(())
    }
}
