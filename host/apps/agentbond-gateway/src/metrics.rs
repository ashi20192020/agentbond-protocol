use std::sync::Arc;

use agentbond_payments::{
    BeginOutcome, LeaseToken, PaidDemoResult, PaymentError, SettlementBinding, SettlementStore,
};
use async_trait::async_trait;
use prometheus::{Encoder, IntCounter, Registry, TextEncoder, opts};

#[derive(Clone)]
pub struct PaymentMetrics {
    registry: Arc<Registry>,
    pub lease_acquisition: IntCounter,
    pub cache_hit: IntCounter,
    pub in_progress: IntCounter,
    pub stale_recovery: IntCounter,
    pub completion: IntCounter,
    pub failure: IntCounter,
}

impl PaymentMetrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();
        let lease_acquisition = IntCounter::with_opts(opts!(
            "agentbond_settlement_lease_acquisition",
            "settlement leases acquired"
        ))?;
        let cache_hit = IntCounter::with_opts(opts!(
            "agentbond_settlement_cache_hit",
            "settlement cache hits"
        ))?;
        let in_progress = IntCounter::with_opts(opts!(
            "agentbond_settlement_in_progress",
            "settlement in progress results"
        ))?;
        let stale_recovery = IntCounter::with_opts(opts!(
            "agentbond_settlement_recovery",
            "stale lease recoveries"
        ))?;
        let completion = IntCounter::with_opts(opts!(
            "agentbond_settlement_completion",
            "settlement completions"
        ))?;
        let failure =
            IntCounter::with_opts(opts!("agentbond_settlement_failure", "settlement failures"))?;
        registry.register(Box::new(lease_acquisition.clone()))?;
        registry.register(Box::new(cache_hit.clone()))?;
        registry.register(Box::new(in_progress.clone()))?;
        registry.register(Box::new(stale_recovery.clone()))?;
        registry.register(Box::new(completion.clone()))?;
        registry.register(Box::new(failure.clone()))?;
        Ok(Self {
            registry: Arc::new(registry),
            lease_acquisition,
            cache_hit,
            in_progress,
            stale_recovery,
            completion,
            failure,
        })
    }

    pub fn render(&self) -> Result<String, prometheus::Error> {
        let encoder = TextEncoder::new();
        let mut buffer = Vec::new();
        encoder.encode(&self.registry.gather(), &mut buffer)?;
        String::from_utf8(buffer).map_err(|_| prometheus::Error::Msg("utf8".into()))
    }
}

pub struct MeteredSettlementStore {
    inner: Arc<dyn SettlementStore>,
    metrics: Arc<PaymentMetrics>,
}

impl MeteredSettlementStore {
    pub fn new(inner: Arc<dyn SettlementStore>, metrics: Arc<PaymentMetrics>) -> Self {
        Self { inner, metrics }
    }
}

#[async_trait]
impl SettlementStore for MeteredSettlementStore {
    async fn begin(
        &self,
        tx_digest: &str,
        binding: SettlementBinding,
    ) -> Result<BeginOutcome, PaymentError> {
        match self.inner.begin(tx_digest, binding).await {
            Ok(BeginOutcome::Cached(result)) => {
                self.metrics.cache_hit.inc();
                Ok(BeginOutcome::Cached(result))
            }
            Ok(BeginOutcome::Acquired(lease)) => {
                self.metrics.lease_acquisition.inc();
                Ok(BeginOutcome::Acquired(lease))
            }
            Ok(BeginOutcome::RecoveredStale(lease)) => {
                self.metrics.stale_recovery.inc();
                self.metrics.lease_acquisition.inc();
                Ok(BeginOutcome::RecoveredStale(lease))
            }
            Err(PaymentError::SettlementInProgress) => {
                self.metrics.in_progress.inc();
                Err(PaymentError::SettlementInProgress)
            }
            Err(e) => Err(e),
        }
    }

    async fn complete(
        &self,
        tx_digest: &str,
        binding: &SettlementBinding,
        lease: &LeaseToken,
        result: PaidDemoResult,
    ) -> Result<(), PaymentError> {
        self.inner
            .complete(tx_digest, binding, lease, result)
            .await?;
        self.metrics.completion.inc();
        Ok(())
    }

    async fn fail(
        &self,
        tx_digest: &str,
        binding: &SettlementBinding,
        lease: &LeaseToken,
    ) -> Result<(), PaymentError> {
        self.inner.fail(tx_digest, binding, lease).await?;
        self.metrics.failure.inc();
        Ok(())
    }
}
