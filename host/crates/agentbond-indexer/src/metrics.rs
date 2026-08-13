use std::sync::Arc;

use prometheus::{Encoder, IntCounter, IntGauge, Registry, TextEncoder, opts};

#[derive(Clone)]
pub struct IndexerMetrics {
    pub registry: Arc<Registry>,
    pub received_updates: IntCounter,
    pub decoded_events: IntCounter,
    pub decode_failures: IntCounter,
    pub duplicate_updates: IntCounter,
    pub finalized_projections: IntCounter,
    pub current_finalized_slot: IntGauge,
    pub checkpoint_slot: IntGauge,
    pub detected_gaps: IntCounter,
    pub reconnect_count: IntCounter,
    pub database_errors: IntCounter,
}

impl IndexerMetrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();
        let received_updates =
            IntCounter::with_opts(opts!("agentbond_received_updates", "received updates"))?;
        let decoded_events =
            IntCounter::with_opts(opts!("agentbond_decoded_events", "decoded events"))?;
        let decode_failures =
            IntCounter::with_opts(opts!("agentbond_decode_failures", "decode failures"))?;
        let duplicate_updates =
            IntCounter::with_opts(opts!("agentbond_duplicate_updates", "duplicate updates"))?;
        let finalized_projections = IntCounter::with_opts(opts!(
            "agentbond_finalized_projections",
            "finalized projections"
        ))?;
        let current_finalized_slot = IntGauge::with_opts(opts!(
            "agentbond_current_finalized_slot",
            "current finalized slot"
        ))?;
        let checkpoint_slot =
            IntGauge::with_opts(opts!("agentbond_checkpoint_slot", "checkpoint slot"))?;
        let detected_gaps =
            IntCounter::with_opts(opts!("agentbond_detected_gaps", "detected gaps"))?;
        let reconnect_count =
            IntCounter::with_opts(opts!("agentbond_reconnect_count", "reconnect count"))?;
        let database_errors =
            IntCounter::with_opts(opts!("agentbond_database_errors", "database errors"))?;

        registry.register(Box::new(received_updates.clone()))?;
        registry.register(Box::new(decoded_events.clone()))?;
        registry.register(Box::new(decode_failures.clone()))?;
        registry.register(Box::new(duplicate_updates.clone()))?;
        registry.register(Box::new(finalized_projections.clone()))?;
        registry.register(Box::new(current_finalized_slot.clone()))?;
        registry.register(Box::new(checkpoint_slot.clone()))?;
        registry.register(Box::new(detected_gaps.clone()))?;
        registry.register(Box::new(reconnect_count.clone()))?;
        registry.register(Box::new(database_errors.clone()))?;

        Ok(Self {
            registry: Arc::new(registry),
            received_updates,
            decoded_events,
            decode_failures,
            duplicate_updates,
            finalized_projections,
            current_finalized_slot,
            checkpoint_slot,
            detected_gaps,
            reconnect_count,
            database_errors,
        })
    }

    pub fn render(&self) -> Result<String, prometheus::Error> {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buffer = Vec::new();
        encoder.encode(&metric_families, &mut buffer)?;
        String::from_utf8(buffer).map_err(|_| prometheus::Error::Msg("utf8".into()))
    }
}
