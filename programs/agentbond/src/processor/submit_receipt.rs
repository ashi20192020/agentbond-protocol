use agentbond_types::{
    AgentBondWorkReceiptV1, JobState, ProtocolError, ProtocolEventKind, PROVIDER_STATUS_ACTIVE,
};
use pinocchio::account::AccountView;
use pinocchio::address::Address;
use pinocchio::error::ProgramResult;

use crate::accounts::require_writable;
use crate::ed25519::verify_preceding_ed25519;
use crate::error::fail;
use crate::events;
use crate::processor::helpers::{
    load_validated_config, load_validated_job, load_validated_provider, next_account, now_ts,
    save_job, transition_job,
};

pub fn process(
    program_id: &Address,
    accounts: &[AccountView],
    receipt: AgentBondWorkReceiptV1,
) -> ProgramResult {
    let mut accounts = accounts.iter();
    let config_account = next_account(&mut accounts)?;
    let provider_account = next_account(&mut accounts)?;
    let job_account = next_account(&mut accounts)?;
    let instructions_sysvar = next_account(&mut accounts)?;

    require_writable(job_account)?;

    let config = load_validated_config(program_id, config_account)?;
    let provider = load_validated_provider(program_id, provider_account)?;
    if provider.status != PROVIDER_STATUS_ACTIVE {
        return Err(fail(ProtocolError::ProviderInactive));
    }

    let mut job = load_validated_job(program_id, job_account)?;
    if job.state != JobState::Accepted {
        return Err(fail(ProtocolError::InvalidStateTransition));
    }
    if job.provider != provider.authority {
        return Err(fail(ProtocolError::Unauthorized));
    }

    let now = now_ts()?;
    if now > job.work_deadline {
        return Err(fail(ProtocolError::DeadlineExpired));
    }
    if receipt.expires_at < now {
        return Err(fail(ProtocolError::ReceiptExpired));
    }
    if receipt.created_at > now {
        return Err(fail(ProtocolError::FutureTimestamp));
    }

    if receipt.program_id != program_id.to_bytes() {
        return Err(fail(ProtocolError::InvalidReceiptField));
    }
    if receipt.genesis_hash != config.genesis_hash {
        return Err(fail(ProtocolError::InvalidReceiptField));
    }
    if receipt.job != job_account.address().to_bytes() {
        return Err(fail(ProtocolError::InvalidReceiptField));
    }
    if receipt.buyer != job.buyer {
        return Err(fail(ProtocolError::InvalidReceiptField));
    }
    if receipt.provider != job.provider {
        return Err(fail(ProtocolError::InvalidReceiptField));
    }
    if receipt.request_hash != job.request_hash {
        return Err(fail(ProtocolError::InvalidReceiptField));
    }
    if receipt.job_nonce != job.job_nonce {
        return Err(fail(ProtocolError::InvalidReceiptField));
    }

    let encoded = receipt.encode().map_err(fail)?;
    let signer_key = verify_preceding_ed25519(instructions_sysvar, &encoded)?;
    if !provider.contains_execution_key(&signer_key) {
        return Err(fail(ProtocolError::InvalidSignature));
    }

    let digest = receipt.digest().map_err(fail)?;
    job.receipt_digest = digest;
    transition_job(&mut job, JobState::Submitted)?;
    save_job(job_account, &job)?;

    events::emit(
        ProtocolEventKind::ReceiptSubmitted,
        job_account.address(),
        &Address::new_from_array(signer_key),
        0,
        now,
    )
}
