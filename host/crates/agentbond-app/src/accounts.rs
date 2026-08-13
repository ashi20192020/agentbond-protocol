use agentbond_types::{
    ChallengeAccount, ConfigAccount, JobAccount, JobState, PROVIDER_STATUS_ACTIVE,
    PROVIDER_STATUS_INACTIVE, ProviderAccount, ProviderBondAccount,
};
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;

fn pk_b58(bytes: &[u8; 32]) -> String {
    Pubkey::new_from_array(*bytes).to_string()
}

fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn job_state_name(state: JobState) -> &'static str {
    match state {
        JobState::Created => "Created",
        JobState::Funded => "Funded",
        JobState::Accepted => "Accepted",
        JobState::Submitted => "Submitted",
        JobState::Challenged => "Challenged",
        JobState::Settled => "Settled",
        JobState::Refunded => "Refunded",
        JobState::Expired => "Expired",
        JobState::Slashed => "Slashed",
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigDto {
    pub bump: u8,
    pub paused: bool,
    pub admin: String,
    pub genesis_hash_hex: String,
    pub allowed_mint: String,
    pub token_program: String,
    pub mint_decimals: u8,
    pub min_provider_bond: u64,
    pub challenge_duration_seconds: i64,
}

impl ConfigDto {
    pub fn from_account(a: &ConfigAccount) -> Self {
        Self {
            bump: a.bump,
            paused: a.paused,
            admin: pk_b58(&a.admin),
            genesis_hash_hex: hex32(&a.genesis_hash),
            allowed_mint: pk_b58(&a.allowed_mint),
            token_program: pk_b58(&a.token_program),
            mint_decimals: a.mint_decimals,
            min_provider_bond: a.min_provider_bond,
            challenge_duration_seconds: a.challenge_duration_seconds,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderDto {
    pub bump: u8,
    pub status: String,
    pub authority: String,
    pub execution_key_count: u8,
    pub execution_keys: Vec<String>,
}

impl ProviderDto {
    pub fn from_account(a: &ProviderAccount) -> Self {
        let status = match a.status {
            PROVIDER_STATUS_ACTIVE => "Active",
            PROVIDER_STATUS_INACTIVE => "Inactive",
            _ => "Unknown",
        };
        let keys = a
            .execution_keys
            .iter()
            .take(usize::from(a.execution_key_count))
            .map(hex32)
            .collect();
        Self {
            bump: a.bump,
            status: status.into(),
            authority: pk_b58(&a.authority),
            execution_key_count: a.execution_key_count,
            execution_keys: keys,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderBondDto {
    pub bump: u8,
    pub provider: String,
    pub mint: String,
    pub token_program: String,
    pub deposited: u64,
    pub locked: u64,
}

impl ProviderBondDto {
    pub fn from_account(a: &ProviderBondAccount) -> Self {
        Self {
            bump: a.bump,
            provider: pk_b58(&a.provider),
            mint: pk_b58(&a.mint),
            token_program: pk_b58(&a.token_program),
            deposited: a.deposited,
            locked: a.locked,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobDto {
    pub bump: u8,
    pub state: String,
    pub buyer: String,
    pub provider: String,
    pub mint: String,
    pub token_program: String,
    pub amount: u64,
    pub job_nonce: u64,
    pub fund_deadline: i64,
    pub accept_deadline: i64,
    pub work_deadline: i64,
    pub auto_settle_deadline: i64,
    pub receipt_digest_hex: String,
    pub request_hash_hex: String,
    pub locked_bond: u64,
    pub mint_decimals: u8,
}

impl JobDto {
    pub fn from_account(a: &JobAccount) -> Self {
        Self {
            bump: a.bump,
            state: job_state_name(a.state).into(),
            buyer: pk_b58(&a.buyer),
            provider: pk_b58(&a.provider),
            mint: pk_b58(&a.mint),
            token_program: pk_b58(&a.token_program),
            amount: a.amount,
            job_nonce: a.job_nonce,
            fund_deadline: a.fund_deadline,
            accept_deadline: a.accept_deadline,
            work_deadline: a.work_deadline,
            auto_settle_deadline: a.auto_settle_deadline,
            receipt_digest_hex: hex32(&a.receipt_digest),
            request_hash_hex: hex32(&a.request_hash),
            locked_bond: a.locked_bond,
            mint_decimals: a.mint_decimals,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChallengeDto {
    pub bump: u8,
    pub status: u8,
    pub job: String,
    pub buyer: String,
    pub reason_hash_hex: String,
    pub deadline: i64,
    pub bond_amount: u64,
}

impl ChallengeDto {
    pub fn from_account(a: &ChallengeAccount) -> Self {
        Self {
            bump: a.bump,
            status: a.status,
            job: pk_b58(&a.job),
            buyer: pk_b58(&a.buyer),
            reason_hash_hex: hex32(&a.reason_hash),
            deadline: a.deadline,
            bond_amount: a.bond_amount,
        }
    }
}
