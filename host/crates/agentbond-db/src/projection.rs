use chrono::{DateTime, TimeZone, Utc};
use sqlx::{PgPool, Postgres, Transaction};

use crate::error::DbError;
use crate::util::u64_to_numeric;

#[derive(Clone, Debug)]
pub struct SlotUpdate {
    pub slot: u64,
    pub parent_slot: Option<u64>,
    pub status: Commitment,
    pub block_time: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Commitment {
    Processed,
    Confirmed,
    Finalized,
    Dead,
}

impl Commitment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Processed => "processed",
            Self::Confirmed => "confirmed",
            Self::Finalized => "finalized",
            Self::Dead => "dead",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RawAccountVersion {
    pub address: [u8; 32],
    pub slot: u64,
    pub write_version: u64,
    pub owner: Option<[u8; 32]>,
    pub lamports: u64,
    pub executable: bool,
    pub data: Option<Vec<u8>>,
    pub deleted: bool,
    pub commitment: Commitment,
}

#[derive(Clone, Debug)]
pub struct RawProtocolEvent {
    pub signature: String,
    pub event_index: u32,
    pub slot: u64,
    pub program_id: [u8; 32],
    pub kind: u8,
    pub subject: [u8; 32],
    pub actor: [u8; 32],
    pub amount: u64,
    pub event_timestamp: i64,
    pub commitment: Commitment,
}

#[derive(Clone, Debug)]
pub struct DecodedProjection {
    pub kind: ProjectionKind,
    pub address: [u8; 32],
    pub slot: u64,
    pub write_version: u64,
    pub payload: ProjectionPayload,
}

#[derive(Clone, Debug)]
pub enum ProjectionKind {
    Config,
    Provider,
    ProviderBond,
    Job,
    Challenge,
    Tombstone,
}

#[derive(Clone, Debug)]
pub enum ProjectionPayload {
    Config {
        paused: bool,
        admin: [u8; 32],
        genesis_hash: [u8; 32],
        allowed_mint: [u8; 32],
        token_program: [u8; 32],
        mint_decimals: u8,
        min_provider_bond: u64,
        challenge_duration_seconds: u64,
    },
    Provider {
        authority: [u8; 32],
        status: String,
        execution_key_count: u8,
    },
    ProviderBond {
        authority: [u8; 32],
        mint: [u8; 32],
        deposited: u64,
        locked: u64,
    },
    Job {
        buyer: [u8; 32],
        provider: [u8; 32],
        mint: [u8; 32],
        token_program: [u8; 32],
        amount: u64,
        job_nonce: u64,
        state: String,
        fund_deadline: i64,
        accept_deadline: i64,
        work_deadline: i64,
        auto_settle_deadline: i64,
        request_hash: [u8; 32],
        receipt_digest: [u8; 32],
        locked_bond: u64,
        mint_decimals: u8,
    },
    Challenge {
        job: [u8; 32],
        buyer: [u8; 32],
        reason_hash: [u8; 32],
        bond_amount: u64,
        deadline: i64,
        status: String,
    },
    Tombstone,
}

pub struct ProjectionRepo {
    pool: PgPool,
}

impl ProjectionRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn upsert_slot(&self, update: &SlotUpdate) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO indexer_slots (slot, parent_slot, status, block_time, updated_at)
             VALUES ($1,$2,$3,$4,NOW())
             ON CONFLICT (slot) DO UPDATE SET
               parent_slot = EXCLUDED.parent_slot,
               status = EXCLUDED.status,
               block_time = COALESCE(EXCLUDED.block_time, indexer_slots.block_time),
               updated_at = NOW()",
        )
        .bind(update.slot as i64)
        .bind(update.parent_slot.map(|s| s as i64))
        .bind(update.status.as_str())
        .bind(
            update
                .block_time
                .and_then(|t| Utc.timestamp_opt(t, 0).single()),
        )
        .execute(&self.pool)
        .await?;
        if update.status == Commitment::Dead {
            self.cleanup_dead_slot(update.slot).await?;
        }
        Ok(())
    }

    async fn cleanup_dead_slot(&self, slot: u64) -> Result<(), DbError> {
        sqlx::query(
            "DELETE FROM raw_account_versions WHERE slot = $1 AND commitment <> 'finalized'",
        )
        .bind(slot as i64)
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "DELETE FROM raw_protocol_events WHERE slot = $1 AND commitment <> 'finalized'",
        )
        .bind(slot as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn insert_account_version(&self, acc: &RawAccountVersion) -> Result<bool, DbError> {
        let res = sqlx::query(
            "INSERT INTO raw_account_versions (
                address, slot, write_version, owner, lamports, executable, data, deleted, commitment
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
             ON CONFLICT DO NOTHING",
        )
        .bind(acc.address.as_slice())
        .bind(acc.slot as i64)
        .bind(acc.write_version as i64)
        .bind(acc.owner.as_ref().map(|o| o.as_slice()))
        .bind(u64_to_numeric(acc.lamports))
        .bind(acc.executable)
        .bind(acc.data.as_deref())
        .bind(acc.deleted)
        .bind(acc.commitment.as_str())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    pub async fn insert_event(&self, ev: &RawProtocolEvent) -> Result<bool, DbError> {
        let ts = Utc
            .timestamp_opt(ev.event_timestamp, 0)
            .single()
            .ok_or_else(|| DbError::Validation("bad event timestamp".into()))?;
        let res = sqlx::query(
            "INSERT INTO raw_protocol_events (
                signature, event_index, slot, program_id, kind, subject, actor, amount,
                event_timestamp, commitment
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
             ON CONFLICT DO NOTHING",
        )
        .bind(&ev.signature)
        .bind(ev.event_index as i32)
        .bind(ev.slot as i64)
        .bind(ev.program_id.as_slice())
        .bind(ev.kind as i16)
        .bind(ev.subject.as_slice())
        .bind(ev.actor.as_slice())
        .bind(u64_to_numeric(ev.amount))
        .bind(ts)
        .bind(ev.commitment.as_str())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    pub async fn finalize_slot(
        &self,
        slot: u64,
        projections: &[DecodedProjection],
    ) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        self.assert_ancestry(&mut tx, slot).await?;
        sqlx::query(
            "UPDATE indexer_slots SET status = 'finalized', updated_at = NOW() WHERE slot = $1",
        )
        .bind(slot as i64)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE raw_account_versions SET commitment = 'finalized' WHERE slot = $1")
            .bind(slot as i64)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE raw_protocol_events SET commitment = 'finalized' WHERE slot = $1")
            .bind(slot as i64)
            .execute(&mut *tx)
            .await?;
        for p in projections {
            apply_projection(&mut tx, p).await?;
        }
        sqlx::query(
            "UPDATE indexer_checkpoints
             SET finalized_slot = GREATEST(finalized_slot, $1),
                 processed_slot = GREATEST(processed_slot, $1),
                 updated_at = NOW()
             WHERE id = 1",
        )
        .bind(slot as i64)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn assert_ancestry(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        slot: u64,
    ) -> Result<(), DbError> {
        let row: Option<(Option<i64>,)> =
            sqlx::query_as("SELECT parent_slot FROM indexer_slots WHERE slot = $1")
                .bind(slot as i64)
                .fetch_optional(&mut **tx)
                .await?;
        let Some((parent,)) = row else {
            return Ok(());
        };
        let Some(parent) = parent else {
            return Ok(());
        };
        let parent_status: Option<(String,)> =
            sqlx::query_as("SELECT status FROM indexer_slots WHERE slot = $1")
                .bind(parent)
                .fetch_optional(&mut **tx)
                .await?;
        if let Some((status,)) = parent_status
            && status == "dead"
        {
            return Err(DbError::Conflict(
                "conflicting finalized ancestry: parent is dead".into(),
            ));
        }
        Ok(())
    }

    pub async fn checkpoint(&self) -> Result<(u64, u64), DbError> {
        let (f, p): (i64, i64) = sqlx::query_as(
            "SELECT finalized_slot, processed_slot FROM indexer_checkpoints WHERE id = 1",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok((f as u64, p as u64))
    }

    pub async fn record_gap(&self, from_slot: u64, to_slot: u64) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO ingestion_gaps (from_slot, to_slot, status)
             VALUES ($1,$2,'open')
             ON CONFLICT (from_slot, to_slot) DO NOTHING",
        )
        .bind(from_slot as i64)
        .bind(to_slot as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_gap_failed(
        &self,
        from_slot: u64,
        to_slot: u64,
        err: &str,
    ) -> Result<(), DbError> {
        let msg: String = err.chars().take(512).collect();
        sqlx::query(
            "UPDATE ingestion_gaps
             SET status = 'failed', attempts = attempts + 1, last_error = $3, updated_at = NOW()
             WHERE from_slot = $1 AND to_slot = $2",
        )
        .bind(from_slot as i64)
        .bind(to_slot as i64)
        .bind(msg)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_gap_repaired(&self, from_slot: u64, to_slot: u64) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE ingestion_gaps SET status = 'repaired', updated_at = NOW()
             WHERE from_slot = $1 AND to_slot = $2",
        )
        .bind(from_slot as i64)
        .bind(to_slot as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn open_gaps(&self) -> Result<Vec<(u64, u64)>, DbError> {
        let rows: Vec<(i64, i64)> = sqlx::query_as(
            "SELECT from_slot, to_slot FROM ingestion_gaps
             WHERE status IN ('open','failed') ORDER BY from_slot ASC LIMIT 32",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(a, b)| (a as u64, b as u64))
            .collect())
    }
}

async fn apply_projection(
    tx: &mut Transaction<'_, Postgres>,
    p: &DecodedProjection,
) -> Result<(), DbError> {
    // Reject stale writes: only apply if newer than existing as_of_slot.
    match &p.payload {
        ProjectionPayload::Tombstone => {
            sqlx::query("DELETE FROM proj_jobs WHERE address = $1 AND as_of_slot <= $2")
                .bind(p.address.as_slice())
                .bind(p.slot as i64)
                .execute(&mut **tx)
                .await?;
            sqlx::query("DELETE FROM proj_providers WHERE address = $1 AND as_of_slot <= $2")
                .bind(p.address.as_slice())
                .bind(p.slot as i64)
                .execute(&mut **tx)
                .await?;
            sqlx::query("DELETE FROM proj_provider_bonds WHERE address = $1 AND as_of_slot <= $2")
                .bind(p.address.as_slice())
                .bind(p.slot as i64)
                .execute(&mut **tx)
                .await?;
            sqlx::query("DELETE FROM proj_challenges WHERE address = $1 AND as_of_slot <= $2")
                .bind(p.address.as_slice())
                .bind(p.slot as i64)
                .execute(&mut **tx)
                .await?;
            sqlx::query("DELETE FROM proj_config WHERE address = $1 AND as_of_slot <= $2")
                .bind(p.address.as_slice())
                .bind(p.slot as i64)
                .execute(&mut **tx)
                .await?;
        }
        ProjectionPayload::Config {
            paused,
            admin,
            genesis_hash,
            allowed_mint,
            token_program,
            mint_decimals,
            min_provider_bond,
            challenge_duration_seconds,
        } => {
            sqlx::query(
                "INSERT INTO proj_config (
                    address, as_of_slot, paused, admin, genesis_hash, allowed_mint, token_program,
                    mint_decimals, min_provider_bond, challenge_duration_seconds, updated_at
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,NOW())
                 ON CONFLICT (address) DO UPDATE SET
                    as_of_slot = EXCLUDED.as_of_slot,
                    paused = EXCLUDED.paused,
                    admin = EXCLUDED.admin,
                    genesis_hash = EXCLUDED.genesis_hash,
                    allowed_mint = EXCLUDED.allowed_mint,
                    token_program = EXCLUDED.token_program,
                    mint_decimals = EXCLUDED.mint_decimals,
                    min_provider_bond = EXCLUDED.min_provider_bond,
                    challenge_duration_seconds = EXCLUDED.challenge_duration_seconds,
                    updated_at = NOW()
                 WHERE proj_config.as_of_slot <= EXCLUDED.as_of_slot",
            )
            .bind(p.address.as_slice())
            .bind(p.slot as i64)
            .bind(paused)
            .bind(admin.as_slice())
            .bind(genesis_hash.as_slice())
            .bind(allowed_mint.as_slice())
            .bind(token_program.as_slice())
            .bind(*mint_decimals as i16)
            .bind(u64_to_numeric(*min_provider_bond))
            .bind(*challenge_duration_seconds as i64)
            .execute(&mut **tx)
            .await?;
        }
        ProjectionPayload::Provider {
            authority,
            status,
            execution_key_count,
        } => {
            sqlx::query(
                "INSERT INTO proj_providers (address, as_of_slot, authority, status, execution_key_count, updated_at)
                 VALUES ($1,$2,$3,$4,$5,NOW())
                 ON CONFLICT (address) DO UPDATE SET
                    as_of_slot = EXCLUDED.as_of_slot,
                    authority = EXCLUDED.authority,
                    status = EXCLUDED.status,
                    execution_key_count = EXCLUDED.execution_key_count,
                    updated_at = NOW()
                 WHERE proj_providers.as_of_slot <= EXCLUDED.as_of_slot",
            )
            .bind(p.address.as_slice())
            .bind(p.slot as i64)
            .bind(authority.as_slice())
            .bind(status)
            .bind(*execution_key_count as i16)
            .execute(&mut **tx)
            .await?;
        }
        ProjectionPayload::ProviderBond {
            authority,
            mint,
            deposited,
            locked,
        } => {
            sqlx::query(
                "INSERT INTO proj_provider_bonds (address, as_of_slot, authority, mint, deposited, locked, updated_at)
                 VALUES ($1,$2,$3,$4,$5,$6,NOW())
                 ON CONFLICT (address) DO UPDATE SET
                    as_of_slot = EXCLUDED.as_of_slot,
                    authority = EXCLUDED.authority,
                    mint = EXCLUDED.mint,
                    deposited = EXCLUDED.deposited,
                    locked = EXCLUDED.locked,
                    updated_at = NOW()
                 WHERE proj_provider_bonds.as_of_slot <= EXCLUDED.as_of_slot",
            )
            .bind(p.address.as_slice())
            .bind(p.slot as i64)
            .bind(authority.as_slice())
            .bind(mint.as_slice())
            .bind(u64_to_numeric(*deposited))
            .bind(u64_to_numeric(*locked))
            .execute(&mut **tx)
            .await?;
        }
        ProjectionPayload::Job {
            buyer,
            provider,
            mint,
            token_program,
            amount,
            job_nonce,
            state,
            fund_deadline,
            accept_deadline,
            work_deadline,
            auto_settle_deadline,
            request_hash,
            receipt_digest,
            locked_bond,
            mint_decimals,
        } => {
            sqlx::query(
                "INSERT INTO proj_jobs (
                    address, as_of_slot, buyer, provider, mint, token_program, amount, job_nonce, state,
                    fund_deadline, accept_deadline, work_deadline, auto_settle_deadline,
                    request_hash, receipt_digest, locked_bond, mint_decimals, updated_at
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,NOW())
                 ON CONFLICT (address) DO UPDATE SET
                    as_of_slot = EXCLUDED.as_of_slot,
                    buyer = EXCLUDED.buyer,
                    provider = EXCLUDED.provider,
                    mint = EXCLUDED.mint,
                    token_program = EXCLUDED.token_program,
                    amount = EXCLUDED.amount,
                    job_nonce = EXCLUDED.job_nonce,
                    state = EXCLUDED.state,
                    fund_deadline = EXCLUDED.fund_deadline,
                    accept_deadline = EXCLUDED.accept_deadline,
                    work_deadline = EXCLUDED.work_deadline,
                    auto_settle_deadline = EXCLUDED.auto_settle_deadline,
                    request_hash = EXCLUDED.request_hash,
                    receipt_digest = EXCLUDED.receipt_digest,
                    locked_bond = EXCLUDED.locked_bond,
                    mint_decimals = EXCLUDED.mint_decimals,
                    updated_at = NOW()
                 WHERE proj_jobs.as_of_slot <= EXCLUDED.as_of_slot",
            )
            .bind(p.address.as_slice())
            .bind(p.slot as i64)
            .bind(buyer.as_slice())
            .bind(provider.as_slice())
            .bind(mint.as_slice())
            .bind(token_program.as_slice())
            .bind(u64_to_numeric(*amount))
            .bind(u64_to_numeric(*job_nonce))
            .bind(state)
            .bind(ts(*fund_deadline)?)
            .bind(ts(*accept_deadline)?)
            .bind(ts(*work_deadline)?)
            .bind(ts(*auto_settle_deadline)?)
            .bind(request_hash.as_slice())
            .bind(receipt_digest.as_slice())
            .bind(u64_to_numeric(*locked_bond))
            .bind(*mint_decimals as i16)
            .execute(&mut **tx)
            .await?;
        }
        ProjectionPayload::Challenge {
            job,
            buyer,
            reason_hash,
            bond_amount,
            deadline,
            status,
        } => {
            sqlx::query(
                "INSERT INTO proj_challenges (
                    address, as_of_slot, job, buyer, reason_hash, bond_amount, deadline, status, updated_at
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,NOW())
                 ON CONFLICT (address) DO UPDATE SET
                    as_of_slot = EXCLUDED.as_of_slot,
                    job = EXCLUDED.job,
                    buyer = EXCLUDED.buyer,
                    reason_hash = EXCLUDED.reason_hash,
                    bond_amount = EXCLUDED.bond_amount,
                    deadline = EXCLUDED.deadline,
                    status = EXCLUDED.status,
                    updated_at = NOW()
                 WHERE proj_challenges.as_of_slot <= EXCLUDED.as_of_slot",
            )
            .bind(p.address.as_slice())
            .bind(p.slot as i64)
            .bind(job.as_slice())
            .bind(buyer.as_slice())
            .bind(reason_hash.as_slice())
            .bind(u64_to_numeric(*bond_amount))
            .bind(ts(*deadline)?)
            .bind(status)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

fn ts(unix: i64) -> Result<DateTime<Utc>, DbError> {
    Utc.timestamp_opt(unix, 0)
        .single()
        .ok_or_else(|| DbError::Validation("bad timestamp".into()))
}
