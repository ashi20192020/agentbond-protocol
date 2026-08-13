use agentbond_db::{
    Commitment, DecodedProjection, ProjectionKind, ProjectionPayload, RawAccountVersion,
    RawProtocolEvent,
};
use agentbond_types::{
    CHALLENGE_ACCOUNT_LEN, CONFIG_ACCOUNT_LEN, ChallengeAccount, ConfigAccount, EVENT_ENCODED_LEN,
    JOB_ACCOUNT_LEN, JobAccount, PROVIDER_ACCOUNT_LEN, PROVIDER_BOND_ACCOUNT_LEN,
    PROVIDER_STATUS_ACTIVE, ProtocolEvent, ProviderAccount, ProviderBondAccount,
};
use base64::Engine;
use solana_pubkey::Pubkey;

use crate::error::IndexerError;

const MAX_LOG_B64: usize = 256;
const MAX_STACK_DEPTH: usize = 64;

/// Extract AgentBond protocol events using Solana invoke-stack attribution.
pub fn extract_protocol_events(
    program: &Pubkey,
    signature: &str,
    slot: u64,
    logs: &[String],
    commitment: Commitment,
) -> Result<Vec<RawProtocolEvent>, IndexerError> {
    let program_str = program.to_string();
    let mut stack: Vec<String> = Vec::new();
    let mut out = Vec::new();
    let mut event_index = 0u32;

    for log in logs {
        if let Some(rest) = log.strip_prefix("Program ")
            && let Some((id, after)) = rest.split_once(' ')
        {
            if after.starts_with("invoke") {
                if let Some(depth) = parse_invoke_depth(after) {
                    let expected = stack.len().saturating_add(1);
                    if depth != expected {
                        stack.clear();
                        continue;
                    }
                }
                if stack.len() >= MAX_STACK_DEPTH {
                    stack.clear();
                    continue;
                }
                stack.push(id.to_string());
                continue;
            }
            if after.starts_with("success") || after.starts_with("failed") {
                if let Some(top) = stack.last()
                    && top == id
                {
                    stack.pop();
                } else {
                    // Mismatch must not expose an older AgentBond frame.
                    stack.clear();
                }
                continue;
            }
        }

        let Some(data) = log.strip_prefix("Program data: ") else {
            continue;
        };
        let Some(active) = stack.last() else {
            continue;
        };
        if active != &program_str {
            continue;
        }
        let trimmed = data.trim();
        if trimmed.len() > MAX_LOG_B64 * 2 {
            continue;
        }
        let Ok(bytes) = Engine::decode(&base64::engine::general_purpose::STANDARD, trimmed) else {
            continue;
        };
        if bytes.len() != EVENT_ENCODED_LEN {
            continue;
        }
        let Ok(event) = ProtocolEvent::decode(&bytes) else {
            continue;
        };
        out.push(RawProtocolEvent {
            signature: signature.to_string(),
            event_index,
            slot,
            program_id: program.to_bytes(),
            kind: event.kind.as_u8(),
            subject: event.subject,
            actor: event.actor,
            amount: event.amount,
            event_timestamp: event.timestamp,
            commitment,
        });
        event_index = event_index.saturating_add(1);
    }
    Ok(out)
}

fn parse_invoke_depth(after: &str) -> Option<usize> {
    let start = after.find('[')?;
    let end = after.find(']')?;
    if end <= start + 1 {
        return None;
    }
    after[start + 1..end].parse().ok()
}

pub struct AccountDecodeInput {
    pub program: Pubkey,
    pub address: Pubkey,
    pub slot: u64,
    pub write_version: u64,
    pub owner: Option<Pubkey>,
    pub lamports: u64,
    pub data: Option<Vec<u8>>,
    pub deleted: bool,
    pub commitment: Commitment,
}

pub fn decode_account_update(
    input: AccountDecodeInput,
) -> Result<(RawAccountVersion, Option<DecodedProjection>), IndexerError> {
    let AccountDecodeInput {
        program,
        address,
        slot,
        write_version,
        owner,
        lamports,
        data,
        deleted,
        commitment,
    } = input;
    let raw = RawAccountVersion {
        address: address.to_bytes(),
        slot,
        write_version,
        owner: owner.map(|o| o.to_bytes()),
        lamports,
        executable: false,
        data: data.clone(),
        deleted,
        commitment,
    };
    if deleted || data.as_ref().map(|d| d.is_empty()).unwrap_or(true) {
        return Ok((
            raw,
            Some(DecodedProjection {
                kind: ProjectionKind::Tombstone,
                address: address.to_bytes(),
                slot,
                write_version,
                payload: ProjectionPayload::Tombstone,
            }),
        ));
    }
    let Some(owner) = owner else {
        return Ok((raw, None));
    };
    if owner != program {
        return Ok((raw, None));
    }
    let data = data.unwrap_or_default();
    let projection = try_decode_owned(&data, address.to_bytes(), slot, write_version);
    Ok((raw, projection))
}

fn try_decode_owned(
    data: &[u8],
    address: [u8; 32],
    slot: u64,
    write_version: u64,
) -> Option<DecodedProjection> {
    if data.len() == CONFIG_ACCOUNT_LEN
        && let Ok(cfg) = ConfigAccount::decode(data)
    {
        return Some(DecodedProjection {
            kind: ProjectionKind::Config,
            address,
            slot,
            write_version,
            payload: ProjectionPayload::Config {
                paused: cfg.paused,
                admin: cfg.admin,
                genesis_hash: cfg.genesis_hash,
                allowed_mint: cfg.allowed_mint,
                token_program: cfg.token_program,
                mint_decimals: cfg.mint_decimals,
                min_provider_bond: cfg.min_provider_bond,
                challenge_duration_seconds: cfg.challenge_duration_seconds as u64,
            },
        });
    }
    if data.len() == PROVIDER_ACCOUNT_LEN
        && let Ok(provider) = ProviderAccount::decode(data)
    {
        let status = if provider.status == PROVIDER_STATUS_ACTIVE {
            "Active".into()
        } else {
            "Inactive".into()
        };
        return Some(DecodedProjection {
            kind: ProjectionKind::Provider,
            address,
            slot,
            write_version,
            payload: ProjectionPayload::Provider {
                authority: provider.authority,
                status,
                execution_key_count: provider.execution_key_count,
            },
        });
    }
    if data.len() == PROVIDER_BOND_ACCOUNT_LEN
        && let Ok(bond) = ProviderBondAccount::decode(data)
    {
        return Some(DecodedProjection {
            kind: ProjectionKind::ProviderBond,
            address,
            slot,
            write_version,
            payload: ProjectionPayload::ProviderBond {
                authority: bond.provider,
                mint: bond.mint,
                deposited: bond.deposited,
                locked: bond.locked,
            },
        });
    }
    if data.len() == JOB_ACCOUNT_LEN
        && let Ok(job) = JobAccount::decode(data)
    {
        return Some(DecodedProjection {
            kind: ProjectionKind::Job,
            address,
            slot,
            write_version,
            payload: ProjectionPayload::Job {
                buyer: job.buyer,
                provider: job.provider,
                mint: job.mint,
                token_program: job.token_program,
                amount: job.amount,
                job_nonce: job.job_nonce,
                state: format!("{:?}", job.state),
                fund_deadline: job.fund_deadline,
                accept_deadline: job.accept_deadline,
                work_deadline: job.work_deadline,
                auto_settle_deadline: job.auto_settle_deadline,
                request_hash: job.request_hash,
                receipt_digest: job.receipt_digest,
                locked_bond: job.locked_bond,
                mint_decimals: job.mint_decimals,
            },
        });
    }
    if data.len() == CHALLENGE_ACCOUNT_LEN
        && let Ok(challenge) = ChallengeAccount::decode(data)
    {
        let status = if challenge.status == ChallengeAccount::STATUS_OPEN {
            "Open".into()
        } else {
            "Resolved".into()
        };
        return Some(DecodedProjection {
            kind: ProjectionKind::Challenge,
            address,
            slot,
            write_version,
            payload: ProjectionPayload::Challenge {
                job: challenge.job,
                buyer: challenge.buyer,
                reason_hash: challenge.reason_hash,
                bond_amount: challenge.bond_amount,
                deadline: challenge.deadline,
                status,
            },
        });
    }
    None
}
