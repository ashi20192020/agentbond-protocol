use agentbond_types::{parse_instruction, Instruction};
use pinocchio::account::AccountView;
use pinocchio::address::Address;
use pinocchio::error::ProgramResult;

use crate::error::from_protocol;

mod accept_job;
mod accept_work;
mod challenge_work;
mod create_job;
mod deposit_bond;
mod execution_keys;
mod fund_job;
mod helpers;
mod initialize_config;
mod register_provider;
mod resolve;
mod set_paused;
mod submit_receipt;
mod withdraw_bond;

pub fn process(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let instruction = parse_instruction(instruction_data).map_err(from_protocol)?;
    match instruction {
        Instruction::InitializeConfig(payload) => {
            initialize_config::process(program_id, accounts, payload)
        }
        Instruction::SetPaused { paused } => set_paused::process(program_id, accounts, paused),
        Instruction::RegisterProvider => register_provider::process(program_id, accounts),
        Instruction::AddExecutionKey { key } => {
            execution_keys::add_execution_key(program_id, accounts, key)
        }
        Instruction::RevokeExecutionKey { key } => {
            execution_keys::revoke_execution_key(program_id, accounts, key)
        }
        Instruction::DepositBond { amount } => deposit_bond::process(program_id, accounts, amount),
        Instruction::WithdrawBond { amount } => {
            withdraw_bond::process(program_id, accounts, amount)
        }
        Instruction::CreateJob(payload) => create_job::process(program_id, accounts, payload),
        Instruction::FundJob => fund_job::process(program_id, accounts),
        Instruction::AcceptJob => accept_job::process(program_id, accounts),
        Instruction::SubmitReceipt(receipt) => {
            submit_receipt::process(program_id, accounts, receipt)
        }
        Instruction::AcceptWork => accept_work::process(program_id, accounts),
        Instruction::ChallengeWork { reason_hash } => {
            challenge_work::process(program_id, accounts, reason_hash)
        }
        Instruction::ResolveTimeoutSettle => resolve::resolve_timeout_settle(program_id, accounts),
        Instruction::ResolveTimeoutRefund => resolve::resolve_timeout_refund(program_id, accounts),
        Instruction::ExpireUnfunded => resolve::expire_unfunded(program_id, accounts),
        Instruction::ExpireUnaccepted => resolve::expire_unaccepted(program_id, accounts),
        Instruction::SlashBond => resolve::slash_bond(program_id, accounts),
        Instruction::CloseJob => resolve::close_job(program_id, accounts),
    }
}
