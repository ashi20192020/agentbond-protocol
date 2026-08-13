use std::sync::Arc;

use futures::StreamExt;
use tracing::{info, warn};

use agentbond_db::{Commitment, Db, ProjectionRepo};

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
        let (finalized, processed) = repo.checkpoint().await?;
        let mut last_slot = if processed > 0 {
            Some(processed)
        } else if finalized > 0 {
            Some(finalized)
        } else {
            None
        };
        self.metrics
            .checkpoint_slot
            .set(i64::try_from(finalized).unwrap_or(i64::MAX));
        let mut stream = source.subscribe().await?;

        while let Some(item) = stream.next().await {
            let update = match item {
                Ok(u) => u,
                Err(e) => {
                    self.metrics.database_errors.inc();
                    return Err(e);
                }
            };
            if let Some((from, to)) = self.ingest_update(&repo, &mut last_slot, update).await? {
                self.repair_gap(&repo, &mut last_slot, from, to).await?;
            }
        }
        info!("indexer source completed");
        Ok(())
    }

    async fn ingest_update(
        &self,
        repo: &ProjectionRepo,
        last_slot: &mut Option<u64>,
        update: ChainUpdate,
    ) -> Result<Option<(u64, u64)>, IndexerError> {
        self.metrics.received_updates.inc();
        let mut gap = None;
        match update {
            ChainUpdate::Slot(slot) => {
                // Only advance gap tracking forward; ignore older notifications.
                if let Some(prev) = *last_slot {
                    if let Some((from, to)) = detect_forward_gap(prev, slot.slot) {
                        repo.record_gap(from, to).await?;
                        self.metrics.detected_gaps.inc();
                        warn!(from, to, "ingestion gap detected");
                        gap = Some((from, to));
                        *last_slot = Some(slot.slot);
                    } else if slot.slot > prev {
                        *last_slot = Some(slot.slot);
                    }
                } else {
                    *last_slot = Some(slot.slot);
                }

                if let Err(e) = repo.upsert_slot(&slot).await {
                    self.metrics.database_errors.inc();
                    return Err(e.into());
                }
                if slot.status == Commitment::Finalized {
                    match repo.finalize_slot(slot.slot).await {
                        Ok(n) => {
                            self.metrics.finalized_projections.inc_by(n);
                            if let Ok(v) = i64::try_from(slot.slot) {
                                self.metrics.current_finalized_slot.set(v);
                            }
                            if let Ok((finalized, _)) = repo.checkpoint().await
                                && let Ok(v) = i64::try_from(finalized)
                            {
                                self.metrics.checkpoint_slot.set(v);
                            }
                        }
                        Err(e) => {
                            self.metrics.database_errors.inc();
                            return Err(e.into());
                        }
                    }
                }
            }
            ChainUpdate::Account(account) => {
                let AccountUpdate { raw, projection } = *account;
                match repo
                    .insert_account_with_projection(&raw, projection.as_ref())
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => self.metrics.duplicate_updates.inc(),
                    Err(e) => {
                        self.metrics.database_errors.inc();
                        return Err(e.into());
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
                        let nested = self.ingest_update(repo, last_slot, update).await?;
                        if nested.is_some() {
                            return Err(IndexerError::Backfill(
                                "nested gap during backfill is unsupported".into(),
                            ));
                        }
                    }
                }
                Err(e) => {
                    warn!(slot, error = %e, "gap event backfill failed");
                    repo.mark_gap_failed(from, to, &e.to_string()).await?;
                    return Ok(());
                }
            }
        }
        // getBlock repairs events only; account coverage remains unknown until GPA reconcile.
        match self.backfill.reconcile_accounts(from, to).await {
            Ok(true) => {
                repo.mark_gap_repaired(from, to).await?;
                info!(from, to, "gap fully repaired (events + accounts)");
            }
            Ok(false) => {
                repo.mark_gap_partial(
                    from,
                    to,
                    "events repaired via getBlock; account projections pending reconciliation",
                )
                .await?;
                info!(from, to, "gap partially repaired (events only)");
            }
            Err(e) => {
                repo.mark_gap_partial(from, to, &e.to_string()).await?;
            }
        }
        Ok(())
    }
}

/// Forward gap between previously seen slot and a newer notification.
/// Uses checked arithmetic so `prev == u64::MAX` cannot overflow.
pub fn detect_forward_gap(prev: u64, slot: u64) -> Option<(u64, u64)> {
    let next = prev.checked_add(1)?;
    if slot > next {
        let to = slot.checked_sub(1)?;
        Some((next, to))
    } else {
        None
    }
}

#[cfg(test)]
mod gap_tests {
    use super::detect_forward_gap;

    #[test]
    fn gap_at_u64_max_does_not_overflow() {
        assert_eq!(detect_forward_gap(u64::MAX, u64::MAX), None);
        assert_eq!(detect_forward_gap(u64::MAX, 0), None);
        assert_eq!(detect_forward_gap(10, 12), Some((11, 11)));
        assert_eq!(detect_forward_gap(10, 11), None);
        assert_eq!(detect_forward_gap(10, 10), None);
    }
}
