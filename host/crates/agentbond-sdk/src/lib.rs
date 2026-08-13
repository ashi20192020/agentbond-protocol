//! AgentBond host SDK: PDAs, account decoding, instruction builders, and plans.

pub mod address;
pub mod decode;
pub mod error;
pub mod http_util;
pub mod instruction;
pub mod plan;
pub mod receipt;
pub mod rpc;
pub mod submit;

pub use address::{
    PROGRAM_ID_BYTES, Pda, bond_vault_ata, challenge_pda, config_pda, job_escrow_ata, job_pda,
    parse_pubkey, program_id, provider_bond_pda, provider_pda, user_settlement_ata,
};
pub use decode::{
    decode_challenge, decode_config, decode_job, decode_provider, decode_provider_bond,
};
pub use error::SdkError;
pub use instruction::*;
pub use plan::{AccountMetaPlan, InstructionPlan, PlanSummary, PlannedInstruction};
pub use receipt::{
    build_ed25519_verify_instruction, build_submit_receipt_plan, build_submit_receipt_plan_at,
    receipt_digest, validate_receipt,
};
pub use rpc::{AccountData, ChainReader, HttpChainReader, MockChainReader};
pub use submit::{
    ClusterKind, ConfirmPolicy, MAINNET_GENESIS_HASH, SubmitResult, cluster_from_genesis_hash,
    simulate_and_send_plan, validate_plan_instructions,
};
