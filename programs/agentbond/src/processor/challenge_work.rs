use agentbond_types::{
    ChallengeAccount, JobState, ProtocolError, ProtocolEventKind, CHALLENGE_ACCOUNT_LEN,
    CHALLENGE_SEED,
};
use pinocchio::account::AccountView;
use pinocchio::address::Address;
use pinocchio::cpi::Seed;
use pinocchio::error::ProgramResult;

use crate::accounts::{
    create_pda_account, require_signer, require_system_program, require_uninitialized,
    require_writable,
};
use crate::error::fail;
use crate::events;
use crate::processor::helpers::{
    load_validated_config_readonly, load_validated_job, next_account, now_ts, save_challenge,
    save_job, transition_job,
};

pub fn process(
    program_id: &Address,
    accounts: &[AccountView],
    reason_hash: [u8; 32],
) -> ProgramResult {
    let mut accounts = accounts.iter();
    let buyer = next_account(&mut accounts)?;
    let config_account = next_account(&mut accounts)?;
    let job_account = next_account(&mut accounts)?;
    let challenge_account = next_account(&mut accounts)?;
    let system_program = next_account(&mut accounts)?;

    require_signer(buyer)?;
    require_writable(job_account)?;
    require_system_program(system_program)?;

    let config = load_validated_config_readonly(program_id, config_account)?;
    let mut job = load_validated_job(program_id, job_account)?;
    if job.buyer.as_ref() != buyer.address().as_ref() {
        return Err(fail(ProtocolError::Unauthorized));
    }
    if job.state != JobState::Submitted {
        return Err(fail(ProtocolError::InvalidStateTransition));
    }

    let now = now_ts()?;
    if now >= job.auto_settle_deadline {
        return Err(fail(ProtocolError::DeadlineExpired));
    }

    let job_key = job_account.address().to_bytes();
    let (expected, bump) =
        Address::try_find_program_address(&[CHALLENGE_SEED, job_key.as_slice()], program_id)
            .ok_or_else(|| fail(ProtocolError::InvalidPda))?;
    if challenge_account.address() != &expected {
        return Err(fail(ProtocolError::InvalidPda));
    }
    require_uninitialized(challenge_account)?;

    let deadline = now
        .checked_add(config.challenge_duration_seconds)
        .ok_or_else(|| fail(ProtocolError::MathOverflow))?;

    let bump_seed = [bump];
    let seeds = [
        Seed::from(CHALLENGE_SEED),
        Seed::from(job_key.as_slice()),
        Seed::from(bump_seed.as_slice()),
    ];
    create_pda_account(
        buyer,
        challenge_account,
        program_id,
        CHALLENGE_ACCOUNT_LEN,
        &seeds,
    )?;

    let challenge = ChallengeAccount {
        bump,
        status: ChallengeAccount::STATUS_OPEN,
        job: job_key,
        buyer: job.buyer,
        reason_hash,
        bond_amount: 0,
        deadline,
    };
    save_challenge(challenge_account, &challenge)?;
    transition_job(&mut job, JobState::Challenged)?;
    save_job(job_account, &job)?;

    events::emit(
        ProtocolEventKind::JobChallenged,
        job_account.address(),
        buyer.address(),
        0,
        now,
    )
}
