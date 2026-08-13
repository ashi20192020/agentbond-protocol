use std::collections::HashMap;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::error::PaymentError;
use crate::resource::PaidDemoResult;

const MAX_ENTRIES: usize = 512;
const ENTRY_TTL: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettlementBinding {
    pub service_id: String,
    pub resource_url: String,
    pub input_digest: String,
    pub challenge_memo: String,
}

#[derive(Clone, Debug)]
enum State {
    Settling {
        binding: SettlementBinding,
        started: Instant,
    },
    Settled {
        binding: SettlementBinding,
        result: PaidDemoResult,
        completed: Instant,
    },
    /// Failed reservation released after bounded window; key may be retried.
    Failed {
        binding: SettlementBinding,
        failed_at: Instant,
    },
}

/// Atomic Unseen -> Settling -> Settled keyed by transaction payload digest.
pub struct SettlementStore {
    inner: Mutex<HashMap<String, State>>,
}

impl Default for SettlementStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SettlementStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub fn tx_digest(transaction_b64: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(transaction_b64.as_bytes());
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// Reserve for settlement. Returns cached result on exact retry.
    pub async fn begin(
        &self,
        tx_digest: &str,
        binding: SettlementBinding,
    ) -> Result<Option<PaidDemoResult>, PaymentError> {
        let mut guard = self.inner.lock().await;
        evict(&mut guard);
        match guard.get(tx_digest) {
            Some(State::Settled {
                binding: existing,
                result,
                ..
            }) => {
                if existing != &binding {
                    return Err(PaymentError::BindingMismatch);
                }
                Ok(Some(result.clone()))
            }
            Some(State::Settling {
                binding: existing, ..
            }) => {
                if existing != &binding {
                    return Err(PaymentError::BindingMismatch);
                }
                Err(PaymentError::SettlementInProgress)
            }
            Some(State::Failed {
                binding: existing,
                failed_at,
            }) => {
                if existing != &binding {
                    return Err(PaymentError::BindingMismatch);
                }
                // Bounded retry: allow re-reserve after 2s.
                if failed_at.elapsed() < Duration::from_secs(2) {
                    return Err(PaymentError::SettlementInProgress);
                }
                guard.insert(
                    tx_digest.into(),
                    State::Settling {
                        binding,
                        started: Instant::now(),
                    },
                );
                Ok(None)
            }
            None => {
                if guard.len() >= MAX_ENTRIES {
                    evict_one_targeted(&mut guard);
                }
                guard.insert(
                    tx_digest.into(),
                    State::Settling {
                        binding,
                        started: Instant::now(),
                    },
                );
                Ok(None)
            }
        }
    }

    pub async fn complete(
        &self,
        tx_digest: &str,
        binding: &SettlementBinding,
        result: PaidDemoResult,
    ) -> Result<(), PaymentError> {
        let mut guard = self.inner.lock().await;
        match guard.get(tx_digest) {
            Some(State::Settling {
                binding: existing, ..
            }) if existing == binding => {
                guard.insert(
                    tx_digest.into(),
                    State::Settled {
                        binding: binding.clone(),
                        result,
                        completed: Instant::now(),
                    },
                );
                Ok(())
            }
            _ => Err(PaymentError::InvalidChallenge),
        }
    }

    /// Mark failed so another attempt may retry after a short bound.
    pub async fn fail(&self, tx_digest: &str, binding: &SettlementBinding) {
        let mut guard = self.inner.lock().await;
        if matches!(
            guard.get(tx_digest),
            Some(State::Settling {
                binding: existing, ..
            }) if existing == binding
        ) {
            guard.insert(
                tx_digest.into(),
                State::Failed {
                    binding: binding.clone(),
                    failed_at: Instant::now(),
                },
            );
        }
    }
}

fn evict(map: &mut HashMap<String, State>) {
    map.retain(|_, state| match state {
        State::Settling { started, .. } => started.elapsed() <= ENTRY_TTL,
        State::Settled { completed, .. } => completed.elapsed() <= ENTRY_TTL,
        State::Failed { failed_at, .. } => failed_at.elapsed() <= ENTRY_TTL,
    });
}

fn evict_one_targeted(map: &mut HashMap<String, State>) {
    // Prefer failed, then oldest settled, then oldest settling.
    let key = map
        .iter()
        .filter(|(_, s)| matches!(s, State::Failed { .. }))
        .min_by_key(|(_, s)| match s {
            State::Failed { failed_at, .. } => *failed_at,
            _ => Instant::now(),
        })
        .map(|(k, _)| k.clone())
        .or_else(|| {
            map.iter()
                .filter(|(_, s)| matches!(s, State::Settled { .. }))
                .min_by_key(|(_, s)| match s {
                    State::Settled { completed, .. } => *completed,
                    _ => Instant::now(),
                })
                .map(|(k, _)| k.clone())
        })
        .or_else(|| {
            map.iter()
                .min_by_key(|(_, s)| match s {
                    State::Settling { started, .. } => *started,
                    State::Settled { completed, .. } => *completed,
                    State::Failed { failed_at, .. } => *failed_at,
                })
                .map(|(k, _)| k.clone())
        });
    if let Some(k) = key {
        map.remove(&k);
    }
}
