use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::error::PaymentError;
use crate::models::{PaymentRequirements, ResourceInfo, SCHEME_EXACT, SvmExactExtra, X402_VERSION};
use crate::stores::{ChallengeStore, PaymentChallenge, random_memo_hex};

const MAX_CHALLENGES: usize = 512;

pub struct MemoryChallengeStore {
    inner: Mutex<HashMap<String, PaymentChallenge>>,
}

impl Default for MemoryChallengeStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryChallengeStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl ChallengeStore for MemoryChallengeStore {
    async fn issue(
        &self,
        cfg: &crate::resource::X402ResourceConfig,
        resource: &ResourceInfo,
        input_digest: &str,
        issued_at: i64,
    ) -> Result<(PaymentRequirements, PaymentChallenge), PaymentError> {
        let memo = random_memo_hex()?;
        let expires_at = issued_at
            .checked_add(cfg.max_timeout_seconds as i64)
            .ok_or_else(|| PaymentError::Internal("expires_at overflow".into()))?;
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
            expires_at,
            max_timeout_seconds: cfg.max_timeout_seconds,
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
        evict_expired(&mut guard, issued_at);
        if guard.len() >= MAX_CHALLENGES {
            evict_oldest(&mut guard);
        }
        guard.insert(memo, challenge.clone());
        let _ = X402_VERSION;
        let _ = Duration::from_secs(1);
        Ok((requirements, challenge))
    }

    async fn get_valid(&self, memo: &str, now: i64) -> Result<PaymentChallenge, PaymentError> {
        let mut guard = self.inner.lock().await;
        evict_expired(&mut guard, now);
        let challenge = guard
            .get(memo)
            .cloned()
            .ok_or(PaymentError::InvalidChallenge)?;
        if now > challenge.expires_at {
            guard.remove(memo);
            return Err(PaymentError::ChallengeExpired);
        }
        Ok(challenge)
    }
}

fn evict_expired(map: &mut HashMap<String, PaymentChallenge>, now: i64) {
    map.retain(|_, c| now <= c.expires_at);
}

fn evict_oldest(map: &mut HashMap<String, PaymentChallenge>) {
    if let Some(key) = map
        .iter()
        .min_by_key(|(_, c)| c.issued_at)
        .map(|(k, _)| k.clone())
    {
        map.remove(&key);
    }
}
