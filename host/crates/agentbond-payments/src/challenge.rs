use std::collections::HashMap;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::error::PaymentError;
use crate::models::{PaymentRequirements, ResourceInfo, SCHEME_EXACT, SvmExactExtra, X402_VERSION};

const MAX_CHALLENGES: usize = 512;
const CHALLENGE_TTL: Duration = Duration::from_secs(120);

#[derive(Clone, Debug)]
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
    pub max_timeout_seconds: u64,
    created: Instant,
}

pub struct ChallengeStore {
    inner: Mutex<HashMap<String, PaymentChallenge>>,
}

impl Default for ChallengeStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ChallengeStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub async fn issue(
        &self,
        cfg: &crate::resource::X402ResourceConfig,
        resource: &ResourceInfo,
        input_digest: &str,
        issued_at: i64,
    ) -> Result<(PaymentRequirements, PaymentChallenge), PaymentError> {
        let memo = unique_memo(&cfg.service_id, input_digest, issued_at);
        let challenge = PaymentChallenge {
            memo: memo.clone(),
            service_id: cfg.service_id.clone(),
            resource_url: resource.url.clone(),
            description: resource.description.clone(),
            merchant: cfg.pay_to.clone(),
            asset: cfg.asset.clone(),
            amount: cfg.amount.clone(),
            network: cfg.network.clone(),
            fee_payer: cfg.fee_payer.clone(),
            input_digest: input_digest.into(),
            issued_at,
            max_timeout_seconds: cfg.max_timeout_seconds,
            created: Instant::now(),
        };
        let requirements = PaymentRequirements {
            scheme: SCHEME_EXACT.into(),
            network: cfg.network.clone(),
            amount: cfg.amount.clone(),
            asset: cfg.asset.clone(),
            pay_to: cfg.pay_to.clone(),
            max_timeout_seconds: cfg.max_timeout_seconds,
            extra: SvmExactExtra {
                fee_payer: cfg.fee_payer.clone(),
                memo: Some(memo.clone()),
                recent_blockhash: None,
                last_valid_block_height: None,
            },
        };
        let mut guard = self.inner.lock().await;
        evict_expired(&mut guard);
        if guard.len() >= MAX_CHALLENGES {
            evict_oldest(&mut guard);
        }
        guard.insert(memo, challenge.clone());
        let _ = X402_VERSION;
        Ok((requirements, challenge))
    }

    pub async fn get_valid(&self, memo: &str, now: i64) -> Result<PaymentChallenge, PaymentError> {
        let mut guard = self.inner.lock().await;
        evict_expired(&mut guard);
        let challenge = guard
            .get(memo)
            .cloned()
            .ok_or(PaymentError::InvalidChallenge)?;
        let expires = challenge
            .issued_at
            .checked_add(challenge.max_timeout_seconds as i64)
            .ok_or(PaymentError::ChallengeExpired)?;
        if now > expires || challenge.created.elapsed() > CHALLENGE_TTL {
            guard.remove(memo);
            return Err(PaymentError::ChallengeExpired);
        }
        Ok(challenge)
    }
}

fn unique_memo(service_id: &str, input_digest: &str, issued_at: i64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(service_id.as_bytes());
    hasher.update(input_digest.as_bytes());
    hasher.update(issued_at.to_le_bytes());
    hasher.update(uuid_like());
    let digest = hasher.finalize();
    // 16+ bytes hex
    hex::encode(&digest[..16])
}

fn uuid_like() -> [u8; 16] {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&nanos.to_le_bytes());
    out[8..].copy_from_slice(&(nanos.wrapping_mul(0x9e37_79b9_7f4a_7c15)).to_le_bytes());
    out
}

fn evict_expired(map: &mut HashMap<String, PaymentChallenge>) {
    map.retain(|_, c| c.created.elapsed() <= CHALLENGE_TTL);
}

fn evict_oldest(map: &mut HashMap<String, PaymentChallenge>) {
    if let Some(key) = map
        .iter()
        .min_by_key(|(_, c)| c.created)
        .map(|(k, _)| k.clone())
    {
        map.remove(&key);
    }
}

mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
