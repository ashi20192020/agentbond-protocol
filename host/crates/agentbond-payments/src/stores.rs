use async_trait::async_trait;
use uuid::Uuid;

use crate::error::PaymentError;
use crate::models::{PaymentRequirements, ResourceInfo};
use crate::resource::{PaidDemoResult, X402ResourceConfig};
use crate::settlement::SettlementBinding;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaymentChallenge {
    pub memo: String,
    pub service_id: String,
    pub resource_url: String,
    pub description: String,
    pub merchant: String,
    pub asset: String,
    pub amount: String,
    pub network: String,
    pub fee_payer: String,
    pub input_digest: String,
    pub issued_at: i64,
    pub expires_at: i64,
    pub max_timeout_seconds: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaseToken(pub Uuid);

impl LeaseToken {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for LeaseToken {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub enum BeginOutcome {
    Cached(PaidDemoResult),
    Acquired(LeaseToken),
    /// Lease re-acquired after an expired in-progress lease.
    RecoveredStale(LeaseToken),
}

impl BeginOutcome {
    pub fn lease(self) -> Option<LeaseToken> {
        match self {
            Self::Acquired(lease) | Self::RecoveredStale(lease) => Some(lease),
            Self::Cached(_) => None,
        }
    }
}

#[async_trait]
pub trait ChallengeStore: Send + Sync {
    async fn issue(
        &self,
        cfg: &X402ResourceConfig,
        resource: &ResourceInfo,
        input_digest: &str,
        issued_at: i64,
    ) -> Result<(PaymentRequirements, PaymentChallenge), PaymentError>;

    async fn get_valid(&self, memo: &str, now: i64) -> Result<PaymentChallenge, PaymentError>;
}

#[async_trait]
pub trait SettlementStore: Send + Sync {
    async fn begin(
        &self,
        tx_digest: &str,
        binding: SettlementBinding,
    ) -> Result<BeginOutcome, PaymentError>;

    async fn complete(
        &self,
        tx_digest: &str,
        binding: &SettlementBinding,
        lease: &LeaseToken,
        result: PaidDemoResult,
    ) -> Result<(), PaymentError>;

    async fn fail(
        &self,
        tx_digest: &str,
        binding: &SettlementBinding,
        lease: &LeaseToken,
    ) -> Result<(), PaymentError>;
}

pub fn tx_digest(transaction_b64: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(transaction_b64.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn random_memo_hex() -> Result<String, PaymentError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| PaymentError::Internal("rng failed".into()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
