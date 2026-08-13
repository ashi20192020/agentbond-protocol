#![cfg_attr(not(feature = "std"), no_std)]

mod accounts;
mod error;
mod events;
mod instruction;
mod pda;
mod receipt;
mod state;

pub use accounts::{
    ChallengeAccount, ConfigAccount, JobAccount, ProviderAccount, ProviderBondAccount,
    ACCOUNT_LAYOUT_VERSION, CHALLENGE_ACCOUNT_DISCRIMINATOR, CHALLENGE_ACCOUNT_LEN,
    CONFIG_ACCOUNT_DISCRIMINATOR, CONFIG_ACCOUNT_LEN, JOB_ACCOUNT_DISCRIMINATOR, JOB_ACCOUNT_LEN,
    MAX_EXECUTION_KEYS, PROVIDER_ACCOUNT_DISCRIMINATOR, PROVIDER_ACCOUNT_LEN,
    PROVIDER_BOND_ACCOUNT_DISCRIMINATOR, PROVIDER_BOND_ACCOUNT_LEN, PROVIDER_STATUS_ACTIVE,
    PROVIDER_STATUS_INACTIVE,
};
pub use error::ProtocolError;
pub use events::ProtocolEvent;
pub use instruction::{
    parse_instruction, Instruction, InstructionKind, INSTRUCTION_DISCRIMINATOR_LEN,
};
pub use pda::{
    bond_seed_parts, challenge_seed_parts, config_seed_parts, job_nonce_le_bytes, job_seed_parts,
    provider_seed_parts, BOND_SEED, CHALLENGE_SEED, CONFIG_SEED, JOB_SEED, PROVIDER_SEED,
};
pub use receipt::{
    AgentBondWorkReceiptV1, RECEIPT_DOMAIN, RECEIPT_ENCODED_LEN, RECEIPT_VERSION_V1,
};
pub use state::{is_terminal, validate_transition, JobState};
