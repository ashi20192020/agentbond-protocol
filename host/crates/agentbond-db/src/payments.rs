use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use agentbond_payments::{
    BeginOutcome, ChallengeStore, LeaseToken, PaidDemoResult, PaymentChallenge, PaymentError,
    PaymentRequirements, ResourceInfo, SCHEME_EXACT, SettlementBinding, SettlementStore,
    SvmExactExtra, X402_VERSION, X402ResourceConfig, random_memo_hex,
};

use crate::error::DbError;

const LEASE_SECS: i64 = 30;
const FAIL_RETRY_SECS: i64 = 2;
const MAX_RESULT_JSON: usize = 16_384;

#[derive(Clone)]
pub struct PgChallengeStore {
    pool: PgPool,
}

impl PgChallengeStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn purge_expired(&self, now: i64, limit: i64) -> Result<u64, DbError> {
        let cutoff = chrono::DateTime::from_timestamp(now, 0)
            .ok_or_else(|| DbError::Validation("bad purge timestamp".into()))?;
        let res = sqlx::query(
            "DELETE FROM x402_challenges WHERE ctid IN (
                SELECT ctid FROM x402_challenges WHERE expires_at < $1 LIMIT $2
             )",
        )
        .bind(cutoff)
        .bind(limit)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }
}

#[async_trait]
impl ChallengeStore for PgChallengeStore {
    async fn issue(
        &self,
        cfg: &X402ResourceConfig,
        resource: &ResourceInfo,
        input_digest: &str,
        issued_at: i64,
    ) -> Result<(PaymentRequirements, PaymentChallenge), PaymentError> {
        let memo = random_memo_hex()?;
        let expires_at = issued_at
            .checked_add(cfg.max_timeout_seconds as i64)
            .ok_or_else(|| PaymentError::Internal("expires overflow".into()))?;
        let issued = chrono::DateTime::from_timestamp(issued_at, 0)
            .ok_or_else(|| PaymentError::Internal("bad issued_at".into()))?;
        let expires = chrono::DateTime::from_timestamp(expires_at, 0)
            .ok_or_else(|| PaymentError::Internal("bad expires_at".into()))?;
        sqlx::query(
            "INSERT INTO x402_challenges (
                memo, service_id, resource_url, description, merchant, asset, amount,
                network, fee_payer, input_digest, issued_at, expires_at, max_timeout_seconds
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
        )
        .bind(&memo)
        .bind(&cfg.service_id)
        .bind(&resource.url)
        .bind(&resource.description)
        .bind(&cfg.pay_to)
        .bind(&cfg.asset)
        .bind(&cfg.amount)
        .bind(&cfg.network)
        .bind(&cfg.fee_payer)
        .bind(input_digest)
        .bind(issued)
        .bind(expires)
        .bind(cfg.max_timeout_seconds as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| PaymentError::Internal(e.to_string()))?;
        let _ = self.purge_expired(issued_at, 64).await;
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
                memo: Some(memo),
                recent_blockhash: None,
                last_valid_block_height: None,
            },
        };
        let _ = X402_VERSION;
        Ok((requirements, challenge))
    }

    async fn get_valid(&self, memo: &str, now: i64) -> Result<PaymentChallenge, PaymentError> {
        let row = sqlx::query_as::<_, ChallengeRow>(
            "SELECT memo, service_id, resource_url, description, merchant, asset, amount,
                    network, fee_payer, input_digest, issued_at, expires_at, max_timeout_seconds
             FROM x402_challenges WHERE memo = $1",
        )
        .bind(memo)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| PaymentError::Internal(e.to_string()))?
        .ok_or(PaymentError::InvalidChallenge)?;
        let expires_at = row.expires_at.timestamp();
        if now > expires_at {
            let _ = sqlx::query("DELETE FROM x402_challenges WHERE memo = $1")
                .bind(memo)
                .execute(&self.pool)
                .await;
            return Err(PaymentError::ChallengeExpired);
        }
        Ok(PaymentChallenge {
            memo: row.memo,
            service_id: row.service_id,
            resource_url: row.resource_url,
            description: row.description,
            merchant: row.merchant,
            asset: row.asset,
            amount: row.amount,
            network: row.network,
            fee_payer: row.fee_payer,
            input_digest: row.input_digest,
            issued_at: row.issued_at.timestamp(),
            expires_at,
            max_timeout_seconds: row.max_timeout_seconds as u64,
        })
    }
}

#[derive(sqlx::FromRow)]
struct ChallengeRow {
    memo: String,
    service_id: String,
    resource_url: String,
    description: String,
    merchant: String,
    asset: String,
    amount: String,
    network: String,
    fee_payer: String,
    input_digest: String,
    issued_at: chrono::DateTime<Utc>,
    expires_at: chrono::DateTime<Utc>,
    max_timeout_seconds: i64,
}

#[derive(Clone)]
pub struct PgSettlementStore {
    pool: PgPool,
}

impl PgSettlementStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SettlementStore for PgSettlementStore {
    async fn begin(
        &self,
        tx_digest: &str,
        binding: SettlementBinding,
    ) -> Result<BeginOutcome, PaymentError> {
        validate_digest(tx_digest)?;
        validate_binding(&binding)?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| PaymentError::Internal(e.to_string()))?;
        let existing = sqlx::query_as::<_, SettlementRow>(
            "SELECT tx_digest, state, lease_token, lease_expires_at, service_id, resource_url,
                    input_digest, challenge_memo, result_body, payment_response_header, failed_at
             FROM x402_settlements WHERE tx_digest = $1 FOR UPDATE",
        )
        .bind(tx_digest)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| PaymentError::Internal(e.to_string()))?;

        if let Some(row) = existing {
            let row_binding = SettlementBinding {
                service_id: row.service_id.clone(),
                resource_url: row.resource_url.clone(),
                input_digest: row.input_digest.clone(),
                challenge_memo: row.challenge_memo.clone(),
            };
            if row_binding != binding {
                return Err(PaymentError::BindingMismatch);
            }
            let prior = row.state.clone();
            match prior.as_str() {
                "settled" => {
                    let body = row
                        .result_body
                        .ok_or(PaymentError::Internal("missing body".into()))?;
                    let header = row
                        .payment_response_header
                        .ok_or(PaymentError::Internal("missing header".into()))?;
                    tx.commit()
                        .await
                        .map_err(|e| PaymentError::Internal(e.to_string()))?;
                    return Ok(BeginOutcome::Cached(PaidDemoResult {
                        body,
                        payment_response_header: header,
                    }));
                }
                "settling" => {
                    let expired = row.lease_expires_at.map(|t| t < Utc::now()).unwrap_or(true);
                    if !expired {
                        return Err(PaymentError::SettlementInProgress);
                    }
                }
                "failed" => {
                    let ready = row
                        .failed_at
                        .map(|t| t + ChronoDuration::seconds(FAIL_RETRY_SECS) <= Utc::now())
                        .unwrap_or(true);
                    if !ready {
                        return Err(PaymentError::SettlementInProgress);
                    }
                }
                _ => return Err(PaymentError::Internal("bad settlement state".into())),
            }
            let lease = Uuid::new_v4();
            let lease_exp = Utc::now() + ChronoDuration::seconds(LEASE_SECS);
            sqlx::query(
                "UPDATE x402_settlements SET state = 'settling', lease_token = $2,
                        lease_expires_at = $3, failed_at = NULL, updated_at = NOW(),
                        service_id = $4, resource_url = $5, input_digest = $6, challenge_memo = $7
                 WHERE tx_digest = $1",
            )
            .bind(tx_digest)
            .bind(lease)
            .bind(lease_exp)
            .bind(&binding.service_id)
            .bind(&binding.resource_url)
            .bind(&binding.input_digest)
            .bind(&binding.challenge_memo)
            .execute(&mut *tx)
            .await
            .map_err(|e| PaymentError::Internal(e.to_string()))?;
            tx.commit()
                .await
                .map_err(|e| PaymentError::Internal(e.to_string()))?;
            let token = LeaseToken(lease);
            return Ok(if prior == "settling" {
                BeginOutcome::RecoveredStale(token)
            } else {
                BeginOutcome::Acquired(token)
            });
        }

        let lease = Uuid::new_v4();
        let lease_exp = Utc::now() + ChronoDuration::seconds(LEASE_SECS);
        let inserted = sqlx::query(
            "INSERT INTO x402_settlements (
                tx_digest, state, lease_token, lease_expires_at, service_id, resource_url,
                input_digest, challenge_memo
             ) VALUES ($1,'settling',$2,$3,$4,$5,$6,$7)
             ON CONFLICT (tx_digest) DO NOTHING",
        )
        .bind(tx_digest)
        .bind(lease)
        .bind(lease_exp)
        .bind(&binding.service_id)
        .bind(&binding.resource_url)
        .bind(&binding.input_digest)
        .bind(&binding.challenge_memo)
        .execute(&mut *tx)
        .await
        .map_err(|e| PaymentError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| PaymentError::Internal(e.to_string()))?;
        if inserted.rows_affected() == 1 {
            Ok(BeginOutcome::Acquired(LeaseToken(lease)))
        } else {
            // Race: retry read path.
            Box::pin(self.begin(tx_digest, binding)).await
        }
    }

    async fn complete(
        &self,
        tx_digest: &str,
        binding: &SettlementBinding,
        lease: &LeaseToken,
        result: PaidDemoResult,
    ) -> Result<(), PaymentError> {
        let body_str =
            serde_json::to_string(&result.body).map_err(|_| PaymentError::InvalidJson)?;
        if body_str.len() > MAX_RESULT_JSON || result.payment_response_header.len() > 8192 {
            return Err(PaymentError::Internal("result too large".into()));
        }
        let res = sqlx::query(
            "UPDATE x402_settlements
             SET state = 'settled', result_body = $4, payment_response_header = $5,
                 lease_token = NULL, lease_expires_at = NULL, updated_at = NOW()
             WHERE tx_digest = $1 AND state = 'settling' AND lease_token = $2
               AND service_id = $3 AND resource_url = $6 AND input_digest = $7
               AND challenge_memo = $8",
        )
        .bind(tx_digest)
        .bind(lease.0)
        .bind(&binding.service_id)
        .bind(&result.body)
        .bind(&result.payment_response_header)
        .bind(&binding.resource_url)
        .bind(&binding.input_digest)
        .bind(&binding.challenge_memo)
        .execute(&self.pool)
        .await
        .map_err(|e| PaymentError::Internal(e.to_string()))?;
        if res.rows_affected() != 1 {
            return Err(PaymentError::LeaseMismatch);
        }
        Ok(())
    }

    async fn fail(
        &self,
        tx_digest: &str,
        binding: &SettlementBinding,
        lease: &LeaseToken,
    ) -> Result<(), PaymentError> {
        let res = sqlx::query(
            "UPDATE x402_settlements
             SET state = 'failed', failed_at = NOW(), lease_token = NULL,
                 lease_expires_at = NULL, updated_at = NOW()
             WHERE tx_digest = $1 AND state = 'settling' AND lease_token = $2
               AND service_id = $3 AND resource_url = $4 AND input_digest = $5
               AND challenge_memo = $6",
        )
        .bind(tx_digest)
        .bind(lease.0)
        .bind(&binding.service_id)
        .bind(&binding.resource_url)
        .bind(&binding.input_digest)
        .bind(&binding.challenge_memo)
        .execute(&self.pool)
        .await
        .map_err(|e| PaymentError::Internal(e.to_string()))?;
        if res.rows_affected() != 1 {
            return Err(PaymentError::LeaseMismatch);
        }
        Ok(())
    }
}

fn validate_digest(tx_digest: &str) -> Result<(), PaymentError> {
    if tx_digest.len() != 64 || !tx_digest.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(PaymentError::Internal("invalid tx digest".into()));
    }
    Ok(())
}

fn validate_binding(binding: &SettlementBinding) -> Result<(), PaymentError> {
    if binding.input_digest.len() != 64
        || !binding.input_digest.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err(PaymentError::Internal("invalid input digest".into()));
    }
    if binding.challenge_memo.len() != 32
        || !binding
            .challenge_memo
            .chars()
            .all(|c| c.is_ascii_hexdigit())
    {
        return Err(PaymentError::Internal("invalid challenge memo".into()));
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct SettlementRow {
    #[sqlx(rename = "tx_digest")]
    _tx_digest: String,
    state: String,
    #[sqlx(rename = "lease_token")]
    _lease_token: Option<Uuid>,
    lease_expires_at: Option<chrono::DateTime<Utc>>,
    service_id: String,
    resource_url: String,
    input_digest: String,
    challenge_memo: String,
    result_body: Option<serde_json::Value>,
    payment_response_header: Option<String>,
    failed_at: Option<chrono::DateTime<Utc>>,
}
