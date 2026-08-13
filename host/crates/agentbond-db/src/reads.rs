use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Serialize;
use sqlx::PgPool;

use crate::error::DbError;
use crate::util::{numeric_to_u64, pk_bytes, pk_str};

#[derive(Clone, Debug, Serialize)]
pub struct IndexStatusDto {
    pub as_of_slot: String,
    pub processed_slot: String,
    pub open_gaps: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct IndexedJobDto {
    pub address: String,
    pub as_of_slot: String,
    pub buyer: String,
    pub provider: String,
    pub amount: String,
    pub job_nonce: String,
    pub state: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct JobHistoryItemDto {
    pub signature: String,
    pub event_index: u32,
    pub slot: String,
    pub kind: u8,
    pub actor: String,
    pub amount: String,
    pub event_timestamp: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
pub struct IndexedProviderDto {
    pub address: String,
    pub as_of_slot: String,
    pub authority: String,
    pub status: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProviderActivityItemDto {
    pub signature: String,
    pub event_index: u32,
    pub slot: String,
    pub kind: u8,
    pub subject: String,
    pub amount: String,
    pub event_timestamp: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Page<T> {
    pub as_of_slot: String,
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

pub struct ReadRepo {
    pool: PgPool,
}

impl ReadRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn status(&self) -> Result<IndexStatusDto, DbError> {
        let (finalized, processed): (i64, i64) = sqlx::query_as(
            "SELECT finalized_slot, processed_slot FROM indexer_checkpoints WHERE id = 1",
        )
        .fetch_one(&self.pool)
        .await?;
        let (gaps,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*)::bigint FROM ingestion_gaps WHERE status IN ('open','failed','repairing','partial')",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(IndexStatusDto {
            as_of_slot: finalized.to_string(),
            processed_slot: processed.to_string(),
            open_gaps: crate::util::i64_to_u64(gaps)?,
        })
    }

    pub async fn list_jobs(
        &self,
        limit: i64,
        cursor: Option<&str>,
        state: Option<&str>,
        buyer: Option<&str>,
        provider: Option<&str>,
    ) -> Result<Page<IndexedJobDto>, DbError> {
        let limit = validate_limit(limit)?;
        if let Some(state) = state {
            validate_job_state(state)?;
        }
        let (as_of,): (i64,) =
            sqlx::query_as("SELECT finalized_slot FROM indexer_checkpoints WHERE id = 1")
                .fetch_one(&self.pool)
                .await?;
        let cursor_addr = match cursor {
            Some(c) => Some(pk_bytes(c)?),
            None => None,
        };
        let buyer_b = match buyer {
            Some(b) => Some(pk_bytes(b)?),
            None => None,
        };
        let provider_b = match provider {
            Some(p) => Some(pk_bytes(p)?),
            None => None,
        };
        let rows = sqlx::query_as::<_, JobRow>(
            "SELECT address, as_of_slot, buyer, provider, amount, job_nonce, state
             FROM proj_jobs
             WHERE ($1::bytea IS NULL OR address > $1)
               AND ($2::text IS NULL OR state = $2)
               AND ($3::bytea IS NULL OR buyer = $3)
               AND ($4::bytea IS NULL OR provider = $4)
             ORDER BY address ASC
             LIMIT $5",
        )
        .bind(cursor_addr.as_ref().map(|b| b.as_slice()))
        .bind(state)
        .bind(buyer_b.as_ref().map(|b| b.as_slice()))
        .bind(provider_b.as_ref().map(|b| b.as_slice()))
        .bind(limit + 1)
        .fetch_all(&self.pool)
        .await?;
        let has_more = rows.len() > limit as usize;
        let mut items = Vec::new();
        for row in rows.into_iter().take(limit as usize) {
            if row.as_of_slot > as_of {
                return Err(DbError::Conflict("projection newer than checkpoint".into()));
            }
            items.push(IndexedJobDto {
                address: pk_str(&row.address)?,
                as_of_slot: row.as_of_slot.to_string(),
                buyer: pk_str(&row.buyer)?,
                provider: pk_str(&row.provider)?,
                amount: numeric_to_u64(row.amount)?.to_string(),
                job_nonce: numeric_to_u64(row.job_nonce)?.to_string(),
                state: row.state,
            });
        }
        let next_cursor = if has_more {
            items.last().map(|j| j.address.clone())
        } else {
            None
        };
        Ok(Page {
            as_of_slot: as_of.to_string(),
            items,
            next_cursor,
        })
    }

    pub async fn job_history(
        &self,
        address: &str,
        limit: i64,
        cursor: Option<&str>,
    ) -> Result<Page<JobHistoryItemDto>, DbError> {
        let limit = validate_limit(limit)?;
        let addr = pk_bytes(address)?;
        let (as_of,): (i64,) =
            sqlx::query_as("SELECT finalized_slot FROM indexer_checkpoints WHERE id = 1")
                .fetch_one(&self.pool)
                .await?;
        let (cur_sig, cur_idx) = parse_event_cursor(cursor)?;
        let rows = sqlx::query_as::<_, EventRow>(
            "SELECT signature, event_index, slot, kind, actor, amount, event_timestamp
             FROM raw_protocol_events
             WHERE subject = $1 AND commitment = 'finalized'
               AND ($2::text IS NULL OR (signature, event_index) > ($2, $3))
             ORDER BY signature ASC, event_index ASC
             LIMIT $4",
        )
        .bind(addr.as_slice())
        .bind(cur_sig.as_deref())
        .bind(cur_idx)
        .bind(limit + 1)
        .fetch_all(&self.pool)
        .await?;
        let has_more = rows.len() > limit as usize;
        let mut items = Vec::new();
        for row in rows.into_iter().take(limit as usize) {
            items.push(JobHistoryItemDto {
                signature: row.signature,
                event_index: row.event_index as u32,
                slot: row.slot.to_string(),
                kind: row.kind as u8,
                actor: pk_str(&row.actor)?,
                amount: numeric_to_u64(row.amount)?.to_string(),
                event_timestamp: row.event_timestamp,
            });
        }
        let next_cursor = if has_more {
            items
                .last()
                .map(|i| format!("{}:{}", i.signature, i.event_index))
        } else {
            None
        };
        Ok(Page {
            as_of_slot: as_of.to_string(),
            items,
            next_cursor,
        })
    }

    pub async fn list_providers(
        &self,
        limit: i64,
        cursor: Option<&str>,
    ) -> Result<Page<IndexedProviderDto>, DbError> {
        let limit = validate_limit(limit)?;
        let (as_of,): (i64,) =
            sqlx::query_as("SELECT finalized_slot FROM indexer_checkpoints WHERE id = 1")
                .fetch_one(&self.pool)
                .await?;
        let cursor_addr = match cursor {
            Some(c) => Some(pk_bytes(c)?),
            None => None,
        };
        let rows = sqlx::query_as::<_, ProviderRow>(
            "SELECT address, as_of_slot, authority, status FROM proj_providers
             WHERE ($1::bytea IS NULL OR address > $1)
             ORDER BY address ASC LIMIT $2",
        )
        .bind(cursor_addr.as_ref().map(|b| b.as_slice()))
        .bind(limit + 1)
        .fetch_all(&self.pool)
        .await?;
        let has_more = rows.len() > limit as usize;
        let mut items = Vec::new();
        for row in rows.into_iter().take(limit as usize) {
            items.push(IndexedProviderDto {
                address: pk_str(&row.address)?,
                as_of_slot: row.as_of_slot.to_string(),
                authority: pk_str(&row.authority)?,
                status: row.status,
            });
        }
        let next_cursor = if has_more {
            items.last().map(|p| p.address.clone())
        } else {
            None
        };
        Ok(Page {
            as_of_slot: as_of.to_string(),
            items,
            next_cursor,
        })
    }

    pub async fn provider_activity(
        &self,
        address: &str,
        limit: i64,
        cursor: Option<&str>,
    ) -> Result<Page<ProviderActivityItemDto>, DbError> {
        let limit = validate_limit(limit)?;
        let addr = pk_bytes(address)?;
        let (as_of,): (i64,) =
            sqlx::query_as("SELECT finalized_slot FROM indexer_checkpoints WHERE id = 1")
                .fetch_one(&self.pool)
                .await?;
        let (cur_sig, cur_idx) = parse_event_cursor(cursor)?;
        let rows = sqlx::query_as::<_, EventRow2>(
            "SELECT signature, event_index, slot, kind, subject, amount, event_timestamp
             FROM raw_protocol_events
             WHERE actor = $1 AND commitment = 'finalized'
               AND ($2::text IS NULL OR (signature, event_index) > ($2, $3))
             ORDER BY signature ASC, event_index ASC
             LIMIT $4",
        )
        .bind(addr.as_slice())
        .bind(cur_sig.as_deref())
        .bind(cur_idx)
        .bind(limit + 1)
        .fetch_all(&self.pool)
        .await?;
        let has_more = rows.len() > limit as usize;
        let mut items = Vec::new();
        for row in rows.into_iter().take(limit as usize) {
            items.push(ProviderActivityItemDto {
                signature: row.signature,
                event_index: row.event_index as u32,
                slot: row.slot.to_string(),
                kind: row.kind as u8,
                subject: pk_str(&row.subject)?,
                amount: numeric_to_u64(row.amount)?.to_string(),
                event_timestamp: row.event_timestamp,
            });
        }
        let next_cursor = if has_more {
            items
                .last()
                .map(|i| format!("{}:{}", i.signature, i.event_index))
        } else {
            None
        };
        Ok(Page {
            as_of_slot: as_of.to_string(),
            items,
            next_cursor,
        })
    }
}

fn validate_limit(limit: i64) -> Result<i64, DbError> {
    if !(1..=100).contains(&limit) {
        return Err(DbError::Validation(
            "limit must be between 1 and 100".into(),
        ));
    }
    Ok(limit)
}

fn validate_job_state(state: &str) -> Result<(), DbError> {
    match state {
        "Created" | "Funded" | "Accepted" | "Submitted" | "Challenged" | "Settled" | "Refunded"
        | "Expired" | "Slashed" | "Closed" => Ok(()),
        _ => Err(DbError::Validation(format!("invalid job state: {state}"))),
    }
}

fn parse_event_cursor(cursor: Option<&str>) -> Result<(Option<String>, i32), DbError> {
    let Some(c) = cursor else {
        return Ok((None, 0));
    };
    let (sig, idx) = c
        .rsplit_once(':')
        .ok_or_else(|| DbError::Validation("invalid cursor".into()))?;
    let idx: i32 = idx
        .parse()
        .map_err(|_| DbError::Validation("invalid cursor index".into()))?;
    if sig.is_empty() || idx < 0 {
        return Err(DbError::Validation("invalid cursor".into()));
    }
    Ok((Some(sig.to_string()), idx))
}

#[derive(sqlx::FromRow)]
struct JobRow {
    address: Vec<u8>,
    as_of_slot: i64,
    buyer: Vec<u8>,
    provider: Vec<u8>,
    amount: Decimal,
    job_nonce: Decimal,
    state: String,
}

#[derive(sqlx::FromRow)]
struct ProviderRow {
    address: Vec<u8>,
    as_of_slot: i64,
    authority: Vec<u8>,
    status: String,
}

#[derive(sqlx::FromRow)]
struct EventRow {
    signature: String,
    event_index: i32,
    slot: i64,
    kind: i16,
    actor: Vec<u8>,
    amount: Decimal,
    event_timestamp: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct EventRow2 {
    signature: String,
    event_index: i32,
    slot: i64,
    kind: i16,
    subject: Vec<u8>,
    amount: Decimal,
    event_timestamp: DateTime<Utc>,
}
