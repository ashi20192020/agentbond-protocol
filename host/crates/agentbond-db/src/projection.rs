use std::collections::HashSet;

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};

use crate::error::DbError;
use crate::util::{i64_to_u64, u64_to_i64, u64_to_numeric};

/// Bound ancestry walks so finalization never issues unbounded queries.
const MAX_ANCESTRY_DEPTH: u32 = 256;

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

    pub fn parse(s: &str) -> Result<Self, DbError> {
        match s {
            "processed" => Ok(Self::Processed),
            "confirmed" => Ok(Self::Confirmed),
            "finalized" => Ok(Self::Finalized),
            "dead" => Ok(Self::Dead),
            other => Err(DbError::Validation(format!("unknown commitment: {other}"))),
        }
    }

    /// Returns whether `next` may replace `self`.
    /// Downgrades from finalized/dead are rejected by callers as no-ops or conflicts.
    fn can_upgrade(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        match self {
            Self::Finalized | Self::Dead => false,
            Self::Confirmed => matches!(next, Self::Finalized | Self::Dead),
            Self::Processed => matches!(next, Self::Confirmed | Self::Finalized | Self::Dead),
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecodedProjection {
    pub kind: ProjectionKind,
    pub address: [u8; 32],
    pub slot: u64,
    pub write_version: u64,
    pub payload: ProjectionPayload,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProjectionKind {
    Config,
    Provider,
    ProviderBond,
    Job,
    Challenge,
    Tombstone,
}

impl ProjectionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Config => "Config",
            Self::Provider => "Provider",
            Self::ProviderBond => "ProviderBond",
            Self::Job => "Job",
            Self::Challenge => "Challenge",
            Self::Tombstone => "Tombstone",
        }
    }

    fn parse(s: &str) -> Result<Self, DbError> {
        match s {
            "Config" => Ok(Self::Config),
            "Provider" => Ok(Self::Provider),
            "ProviderBond" => Ok(Self::ProviderBond),
            "Job" => Ok(Self::Job),
            "Challenge" => Ok(Self::Challenge),
            "Tombstone" => Ok(Self::Tombstone),
            other => Err(DbError::Validation(format!(
                "unknown projection kind: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
        if let Some(parent) = update.parent_slot
            && parent == update.slot
        {
            return Err(DbError::Conflict(
                "conflicting parent_slot: slot cannot parent itself".into(),
            ));
        }
        let slot = u64_to_i64(update.slot)?;
        let parent = match update.parent_slot {
            Some(p) => Some(u64_to_i64(p)?),
            None => None,
        };
        let block_time = update
            .block_time
            .and_then(|t| Utc.timestamp_opt(t, 0).single());

        let mut tx = self.pool.begin().await?;
        let existing: Option<(String, Option<i64>)> = sqlx::query_as(
            "SELECT status, parent_slot FROM indexer_slots WHERE slot = $1 FOR UPDATE",
        )
        .bind(slot)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some((status, stored_parent)) = existing {
            let current = Commitment::parse(&status)?;
            reconcile_parent(stored_parent, parent, current)?;
            if current == Commitment::Finalized && update.status == Commitment::Dead {
                return Err(DbError::Conflict(
                    "illegal slot status transition: finalized -> dead".into(),
                ));
            }
            if current == Commitment::Dead && update.status == Commitment::Finalized {
                return Err(DbError::Conflict(
                    "illegal slot status transition: dead -> finalized".into(),
                ));
            }
            // Ignore commitment downgrades and repeats that cannot upgrade (replay-safe).
            if !current.can_upgrade(update.status) {
                tx.commit().await?;
                return Ok(());
            }
            sqlx::query(
                "UPDATE indexer_slots
                 SET parent_slot = COALESCE(parent_slot, $2),
                     status = $3,
                     block_time = COALESCE($4, block_time),
                     updated_at = NOW()
                 WHERE slot = $1",
            )
            .bind(slot)
            .bind(parent)
            .bind(update.status.as_str())
            .bind(block_time)
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query(
                "INSERT INTO indexer_slots (slot, parent_slot, status, block_time, updated_at)
                 VALUES ($1,$2,$3,$4,NOW())",
            )
            .bind(slot)
            .bind(parent)
            .bind(update.status.as_str())
            .bind(block_time)
            .execute(&mut *tx)
            .await?;
        }

        if update.status == Commitment::Dead {
            cleanup_dead_slot_tx(&mut tx, update.slot).await?;
        }

        if update.status != Commitment::Dead {
            sqlx::query(
                "UPDATE indexer_checkpoints
                 SET processed_slot = GREATEST(processed_slot, $1),
                     updated_at = NOW()
                 WHERE id = 1",
            )
            .bind(slot)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn insert_account_with_projection(
        &self,
        acc: &RawAccountVersion,
        projection: Option<&DecodedProjection>,
    ) -> Result<bool, DbError> {
        let slot = u64_to_i64(acc.slot)?;
        let write_version = u64_to_i64(acc.write_version)?;
        let mut tx = self.pool.begin().await?;

        let slot_status: Option<(String,)> =
            sqlx::query_as("SELECT status FROM indexer_slots WHERE slot = $1")
                .bind(slot)
                .fetch_optional(&mut *tx)
                .await?;
        let slot_finalized = matches!(
            slot_status.as_ref().map(|(s,)| s.as_str()),
            Some("finalized")
        );
        let apply_now = slot_finalized || acc.commitment == Commitment::Finalized;
        let commitment = if apply_now {
            Commitment::Finalized
        } else {
            acc.commitment
        };
        if commitment == Commitment::Dead {
            return Err(DbError::Validation(
                "account commitment cannot be dead".into(),
            ));
        }

        let res = sqlx::query(
            "INSERT INTO raw_account_versions (
                address, slot, write_version, owner, lamports, executable, data, deleted, commitment
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
             ON CONFLICT DO NOTHING",
        )
        .bind(acc.address.as_slice())
        .bind(slot)
        .bind(write_version)
        .bind(acc.owner.as_ref().map(|o| o.as_slice()))
        .bind(u64_to_numeric(acc.lamports))
        .bind(acc.executable)
        .bind(acc.data.as_deref())
        .bind(acc.deleted)
        .bind(commitment.as_str())
        .execute(&mut *tx)
        .await?;

        if res.rows_affected() == 0 {
            tx.commit().await?;
            return Ok(false);
        }

        if let Some(p) = projection {
            let payload = serde_json::to_value(&p.payload)
                .map_err(|e| DbError::Validation(format!("projection payload encode: {e}")))?;
            sqlx::query(
                "INSERT INTO staged_account_projections (
                    address, slot, write_version, kind, payload
                 ) VALUES ($1,$2,$3,$4,$5)
                 ON CONFLICT (address, slot, write_version) DO UPDATE SET
                    kind = EXCLUDED.kind,
                    payload = EXCLUDED.payload",
            )
            .bind(p.address.as_slice())
            .bind(u64_to_i64(p.slot)?)
            .bind(u64_to_i64(p.write_version)?)
            .bind(p.kind.as_str())
            .bind(payload)
            .execute(&mut *tx)
            .await?;

            if apply_now {
                apply_projection(&mut tx, p).await?;
                sqlx::query(
                    "DELETE FROM staged_account_projections
                     WHERE address = $1 AND slot = $2 AND write_version = $3",
                )
                .bind(p.address.as_slice())
                .bind(u64_to_i64(p.slot)?)
                .bind(u64_to_i64(p.write_version)?)
                .execute(&mut *tx)
                .await?;
            }
        }

        tx.commit().await?;
        Ok(true)
    }

    pub async fn insert_event(&self, ev: &RawProtocolEvent) -> Result<bool, DbError> {
        let ts = Utc
            .timestamp_opt(ev.event_timestamp, 0)
            .single()
            .ok_or_else(|| DbError::Validation("bad event timestamp".into()))?;
        let slot = u64_to_i64(ev.slot)?;
        let slot_status: Option<(String,)> =
            sqlx::query_as("SELECT status FROM indexer_slots WHERE slot = $1")
                .bind(slot)
                .fetch_optional(&self.pool)
                .await?;
        let commitment = if matches!(
            slot_status.as_ref().map(|(s,)| s.as_str()),
            Some("finalized")
        ) {
            Commitment::Finalized
        } else {
            ev.commitment
        };
        if commitment == Commitment::Dead {
            return Err(DbError::Validation(
                "event commitment cannot be dead".into(),
            ));
        }
        let res = sqlx::query(
            "INSERT INTO raw_protocol_events (
                signature, event_index, slot, program_id, kind, subject, actor, amount,
                event_timestamp, commitment
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
             ON CONFLICT DO NOTHING",
        )
        .bind(&ev.signature)
        .bind(ev.event_index as i32)
        .bind(slot)
        .bind(ev.program_id.as_slice())
        .bind(ev.kind as i16)
        .bind(ev.subject.as_slice())
        .bind(ev.actor.as_slice())
        .bind(u64_to_numeric(ev.amount))
        .bind(ts)
        .bind(commitment.as_str())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    pub async fn finalize_slot(&self, slot: u64) -> Result<u64, DbError> {
        let slot_i = u64_to_i64(slot)?;
        let mut tx = self.pool.begin().await?;

        let row: Option<(String,)> =
            sqlx::query_as("SELECT status FROM indexer_slots WHERE slot = $1 FOR UPDATE")
                .bind(slot_i)
                .fetch_optional(&mut *tx)
                .await?;
        let Some((status,)) = row else {
            return Err(DbError::NotFound(format!("unknown slot {slot}")));
        };
        let status = Commitment::parse(&status)?;
        if status == Commitment::Dead {
            return Err(DbError::Conflict(format!(
                "cannot finalize dead slot {slot}"
            )));
        }

        // Ancestry is required the first time a slot becomes finalized.
        if status != Commitment::Finalized {
            assert_ancestry(&mut tx, slot).await?;
        }

        let staged_rows: Vec<(Vec<u8>, i64, i64, String, serde_json::Value)> = sqlx::query_as(
            "SELECT address, slot, write_version, kind, payload
             FROM staged_account_projections
             WHERE slot = $1
             ORDER BY address, write_version",
        )
        .bind(slot_i)
        .fetch_all(&mut *tx)
        .await?;

        let mut projections = Vec::with_capacity(staged_rows.len());
        for (address, s, wv, kind, payload) in staged_rows {
            let address = pk32(&address)?;
            let kind = ProjectionKind::parse(&kind)?;
            let payload: ProjectionPayload = serde_json::from_value(payload)
                .map_err(|e| DbError::Validation(format!("staged payload decode: {e}")))?;
            projections.push(DecodedProjection {
                kind,
                address,
                slot: i64_to_u64(s)?,
                write_version: i64_to_u64(wv)?,
                payload,
            });
        }

        if status != Commitment::Finalized {
            sqlx::query(
                "UPDATE indexer_slots SET status = 'finalized', updated_at = NOW() WHERE slot = $1",
            )
            .bind(slot_i)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("UPDATE raw_account_versions SET commitment = 'finalized' WHERE slot = $1")
            .bind(slot_i)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE raw_protocol_events SET commitment = 'finalized' WHERE slot = $1")
            .bind(slot_i)
            .execute(&mut *tx)
            .await?;

        let applied = projections.len() as u64;
        for p in &projections {
            apply_projection(&mut tx, p).await?;
        }

        sqlx::query("DELETE FROM staged_account_projections WHERE slot = $1")
            .bind(slot_i)
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "UPDATE indexer_checkpoints
             SET finalized_slot = GREATEST(finalized_slot, $1),
                 processed_slot = GREATEST(processed_slot, $1),
                 updated_at = NOW()
             WHERE id = 1",
        )
        .bind(slot_i)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(applied)
    }

    pub async fn checkpoint(&self) -> Result<(u64, u64), DbError> {
        let (f, p): (i64, i64) = sqlx::query_as(
            "SELECT finalized_slot, processed_slot FROM indexer_checkpoints WHERE id = 1",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok((i64_to_u64(f)?, i64_to_u64(p)?))
    }

    pub async fn record_gap(&self, from_slot: u64, to_slot: u64) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO ingestion_gaps (from_slot, to_slot, status)
             VALUES ($1,$2,'open')
             ON CONFLICT (from_slot, to_slot) DO NOTHING",
        )
        .bind(u64_to_i64(from_slot)?)
        .bind(u64_to_i64(to_slot)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_gap_partial(
        &self,
        from_slot: u64,
        to_slot: u64,
        err: &str,
    ) -> Result<(), DbError> {
        let msg: String = err.chars().take(512).collect();
        sqlx::query(
            "UPDATE ingestion_gaps
             SET status = 'partial', attempts = attempts + 1, last_error = $3, updated_at = NOW()
             WHERE from_slot = $1 AND to_slot = $2",
        )
        .bind(u64_to_i64(from_slot)?)
        .bind(u64_to_i64(to_slot)?)
        .bind(msg)
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
        .bind(u64_to_i64(from_slot)?)
        .bind(u64_to_i64(to_slot)?)
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
        .bind(u64_to_i64(from_slot)?)
        .bind(u64_to_i64(to_slot)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn open_gaps(&self) -> Result<Vec<(u64, u64, String)>, DbError> {
        let rows: Vec<(i64, i64, String)> = sqlx::query_as(
            "SELECT from_slot, to_slot, status FROM ingestion_gaps
             WHERE status IN ('partial','open','failed','repairing')
             ORDER BY from_slot ASC LIMIT 32",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for (a, b, status) in rows {
            out.push((i64_to_u64(a)?, i64_to_u64(b)?, status));
        }
        Ok(out)
    }
}

async fn cleanup_dead_slot_tx(
    tx: &mut Transaction<'_, Postgres>,
    slot: u64,
) -> Result<(), DbError> {
    let slot = u64_to_i64(slot)?;
    sqlx::query("DELETE FROM raw_account_versions WHERE slot = $1 AND commitment <> 'finalized'")
        .bind(slot)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM raw_protocol_events WHERE slot = $1 AND commitment <> 'finalized'")
        .bind(slot)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM staged_account_projections WHERE slot = $1")
        .bind(slot)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn reconcile_parent(
    stored: Option<i64>,
    incoming: Option<i64>,
    current: Commitment,
) -> Result<(), DbError> {
    match (stored, incoming) {
        (Some(a), Some(b)) if a != b => Err(DbError::Conflict(
            "conflicting parent_slot for existing slot".into(),
        )),
        (Some(_), None) => Err(DbError::Conflict(
            "conflicting parent_slot for existing slot".into(),
        )),
        (None, Some(_)) if current == Commitment::Finalized => Err(DbError::Conflict(
            "cannot set parent_slot after finalization".into(),
        )),
        _ => Ok(()),
    }
}

async fn assert_ancestry(tx: &mut Transaction<'_, Postgres>, slot: u64) -> Result<(), DbError> {
    let mut current = slot;
    let mut visited = HashSet::new();
    for _ in 0..MAX_ANCESTRY_DEPTH {
        if !visited.insert(current) {
            return Err(DbError::Conflict(
                "conflicting finalized ancestry: parent cycle detected".into(),
            ));
        }
        let row: Option<(Option<i64>,)> =
            sqlx::query_as("SELECT parent_slot FROM indexer_slots WHERE slot = $1")
                .bind(u64_to_i64(current)?)
                .fetch_optional(&mut **tx)
                .await?;
        let Some((parent,)) = row else {
            if current == slot {
                return Err(DbError::NotFound(format!("unknown slot {slot}")));
            }
            return Err(DbError::Conflict(format!(
                "conflicting finalized ancestry: missing parent slot {current}"
            )));
        };
        let Some(parent) = parent else {
            return Ok(());
        };
        let parent_u = i64_to_u64(parent)?;
        if parent_u == current {
            return Err(DbError::Conflict(
                "conflicting finalized ancestry: self-parent".into(),
            ));
        }
        let parent_status: Option<(String,)> =
            sqlx::query_as("SELECT status FROM indexer_slots WHERE slot = $1")
                .bind(parent)
                .fetch_optional(&mut **tx)
                .await?;
        let Some((status,)) = parent_status else {
            return Err(DbError::Conflict(format!(
                "conflicting finalized ancestry: missing parent slot {parent_u}"
            )));
        };
        if status == "dead" {
            return Err(DbError::Conflict(
                "conflicting finalized ancestry: ancestor is dead".into(),
            ));
        }
        current = parent_u;
    }
    Err(DbError::Conflict(
        "conflicting finalized ancestry: parent chain too deep".into(),
    ))
}

async fn apply_projection(
    tx: &mut Transaction<'_, Postgres>,
    p: &DecodedProjection,
) -> Result<(), DbError> {
    let slot = u64_to_i64(p.slot)?;
    match &p.payload {
        ProjectionPayload::Tombstone => {
            sqlx::query("DELETE FROM proj_jobs WHERE address = $1 AND as_of_slot <= $2")
                .bind(p.address.as_slice())
                .bind(slot)
                .execute(&mut **tx)
                .await?;
            sqlx::query("DELETE FROM proj_providers WHERE address = $1 AND as_of_slot <= $2")
                .bind(p.address.as_slice())
                .bind(slot)
                .execute(&mut **tx)
                .await?;
            sqlx::query("DELETE FROM proj_provider_bonds WHERE address = $1 AND as_of_slot <= $2")
                .bind(p.address.as_slice())
                .bind(slot)
                .execute(&mut **tx)
                .await?;
            sqlx::query("DELETE FROM proj_challenges WHERE address = $1 AND as_of_slot <= $2")
                .bind(p.address.as_slice())
                .bind(slot)
                .execute(&mut **tx)
                .await?;
            sqlx::query("DELETE FROM proj_config WHERE address = $1 AND as_of_slot <= $2")
                .bind(p.address.as_slice())
                .bind(slot)
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
            .bind(slot)
            .bind(paused)
            .bind(admin.as_slice())
            .bind(genesis_hash.as_slice())
            .bind(allowed_mint.as_slice())
            .bind(token_program.as_slice())
            .bind(*mint_decimals as i16)
            .bind(u64_to_numeric(*min_provider_bond))
            .bind(u64_to_i64(*challenge_duration_seconds)?)
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
            .bind(slot)
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
            .bind(slot)
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
            .bind(slot)
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
            .bind(slot)
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

fn pk32(bytes: &[u8]) -> Result<[u8; 32], DbError> {
    <[u8; 32]>::try_from(bytes).map_err(|_| DbError::Validation("pubkey must be 32 bytes".into()))
}

fn ts(unix: i64) -> Result<DateTime<Utc>, DbError> {
    Utc.timestamp_opt(unix, 0)
        .single()
        .ok_or_else(|| DbError::Validation("bad timestamp".into()))
}
