use std::collections::HashMap;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::error::PaymentError;
use crate::resource::PaidDemoResult;
use crate::stores::{BeginOutcome, LeaseToken, SettlementStore};

const MAX_ENTRIES: usize = 512;
const ENTRY_TTL: Duration = Duration::from_secs(120);
const FAIL_RETRY: Duration = Duration::from_secs(2);
const LEASE_TTL: Duration = Duration::from_secs(30);

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
        lease: LeaseToken,
        started: Instant,
    },
    Settled {
        binding: SettlementBinding,
        result: PaidDemoResult,
        completed: Instant,
    },
    Failed {
        binding: SettlementBinding,
        failed_at: Instant,
    },
}

pub struct MemorySettlementStore {
    inner: Mutex<HashMap<String, State>>,
}

impl Default for MemorySettlementStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemorySettlementStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl SettlementStore for MemorySettlementStore {
    async fn begin(
        &self,
        tx_digest: &str,
        binding: SettlementBinding,
    ) -> Result<BeginOutcome, PaymentError> {
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
                Ok(BeginOutcome::Cached(result.clone()))
            }
            Some(State::Settling {
                binding: existing,
                started,
                ..
            }) => {
                if existing != &binding {
                    return Err(PaymentError::BindingMismatch);
                }
                if started.elapsed() > LEASE_TTL {
                    let lease = LeaseToken::new();
                    guard.insert(
                        tx_digest.into(),
                        State::Settling {
                            binding,
                            lease: lease.clone(),
                            started: Instant::now(),
                        },
                    );
                    return Ok(BeginOutcome::RecoveredStale(lease));
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
                if failed_at.elapsed() < FAIL_RETRY {
                    return Err(PaymentError::SettlementInProgress);
                }
                let lease = LeaseToken::new();
                guard.insert(
                    tx_digest.into(),
                    State::Settling {
                        binding,
                        lease: lease.clone(),
                        started: Instant::now(),
                    },
                );
                Ok(BeginOutcome::Acquired(lease))
            }
            None => {
                if guard.len() >= MAX_ENTRIES {
                    evict_one_targeted(&mut guard);
                }
                let lease = LeaseToken::new();
                guard.insert(
                    tx_digest.into(),
                    State::Settling {
                        binding,
                        lease: lease.clone(),
                        started: Instant::now(),
                    },
                );
                Ok(BeginOutcome::Acquired(lease))
            }
        }
    }

    async fn complete(
        &self,
        tx_digest: &str,
        binding: &SettlementBinding,
        lease: &LeaseToken,
        result: PaidDemoResult,
    ) -> Result<(), PaymentError> {
        let mut guard = self.inner.lock().await;
        match guard.get(tx_digest) {
            Some(State::Settling {
                binding: existing,
                lease: held,
                ..
            }) if existing == binding && held == lease => {
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
            Some(State::Settling { .. }) => Err(PaymentError::LeaseMismatch),
            _ => Err(PaymentError::InvalidChallenge),
        }
    }

    async fn fail(
        &self,
        tx_digest: &str,
        binding: &SettlementBinding,
        lease: &LeaseToken,
    ) -> Result<(), PaymentError> {
        let mut guard = self.inner.lock().await;
        match guard.get(tx_digest) {
            Some(State::Settling {
                binding: existing,
                lease: held,
                ..
            }) if existing == binding && held == lease => {
                guard.insert(
                    tx_digest.into(),
                    State::Failed {
                        binding: binding.clone(),
                        failed_at: Instant::now(),
                    },
                );
                Ok(())
            }
            Some(State::Settling { .. }) => Err(PaymentError::LeaseMismatch),
            _ => Ok(()),
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
    let key = map
        .iter()
        .filter_map(|(k, state)| match state {
            State::Settled { completed, .. } => Some((k.clone(), completed.elapsed())),
            State::Failed { failed_at, .. } => Some((k.clone(), failed_at.elapsed())),
            State::Settling { .. } => None,
        })
        .max_by_key(|(_, age)| *age)
        .map(|(k, _)| k);
    if let Some(key) = key {
        map.remove(&key);
    }
}
