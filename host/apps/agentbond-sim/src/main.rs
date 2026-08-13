//! Local AgentBond demonstration: LiteSVM escrow paths + mock x402.
//! Offline only. Requires `cargo build-sbf` → `target/deploy/agentbond.so`.

mod env;
mod x402_demo;

use agentbond_types::{JobState, ProtocolError};
use anyhow::{Result, bail};
use env::{CHALLENGE_SECS, JOB_AMOUNT, MIN_BOND, START_TS, setup};
use solana_signer::Signer;

fn step(label: &str) {
    println!("  → {label}");
}

fn scenario_honest_settlement() -> Result<()> {
    println!("\n[1] Honest provider settlement");
    let mut env = setup()?;
    env.bootstrap_ready()?;
    let provider_ata = env.create_ata(&env.provider.pubkey())?;
    let before = env.token_balance(&provider_ata)?;

    step("Created");
    let job = env.create_job(1)?;
    step("Funded");
    env.fund_job(&job)?;
    step("Accepted");
    env.accept_job(&job)?;
    step("Submitted (signed receipt)");
    let receipt = env.make_receipt(&job, 1);
    env.submit_receipt(&job, &receipt)?;
    step("Settled (buyer AcceptWork)");
    env.accept_work(&job)?;

    env.assert_job_state(&job, JobState::Settled)?;
    let after = env.token_balance(&provider_ata)?;
    if after != before + JOB_AMOUNT {
        bail!(
            "provider principal: expected {}, got {after} (before={before})",
            before + JOB_AMOUNT
        );
    }
    let bond = env.read_bond()?;
    if bond.locked != 0 {
        bail!("bond still locked after settlement: {}", bond.locked);
    }
    println!("  state={:?} {}", JobState::Settled, env.balances_line()?);
    Ok(())
}

fn scenario_provider_timeout_refund() -> Result<()> {
    println!("\n[2] Provider timeout and buyer refund");
    let mut env = setup()?;
    env.bootstrap_ready()?;
    let buyer_ata = env.create_ata(&env.buyer.pubkey())?;

    step("Created");
    let job = env.create_job(2)?;
    step("Funded");
    env.fund_job(&job)?;
    let after_fund = env.token_balance(&buyer_ata)?;
    step("Accepted");
    env.accept_job(&job)?;
    step("Work deadline passed (no receipt)");
    env.set_clock(START_TS + 301);
    step("Refunded (ResolveTimeoutRefund)");
    env.resolve_timeout_refund(&job)?;

    env.assert_job_state(&job, JobState::Refunded)?;
    let after_refund = env.token_balance(&buyer_ata)?;
    if after_refund != after_fund + JOB_AMOUNT {
        bail!(
            "buyer refund: expected {}, got {after_refund} (after_fund={after_fund})",
            after_fund + JOB_AMOUNT
        );
    }
    let bond = env.read_bond()?;
    if bond.locked != 0 {
        bail!("bond still locked after refund: {}", bond.locked);
    }
    println!("  state={:?} {}", JobState::Refunded, env.balances_line()?);
    Ok(())
}

fn scenario_challenge_timeout_settle() -> Result<()> {
    println!("\n[3] Buyer challenge and timeout settlement");
    let mut env = setup()?;
    env.bootstrap_ready()?;
    let provider_ata = env.create_ata(&env.provider.pubkey())?;
    let before = env.token_balance(&provider_ata)?;

    step("Created → Funded → Accepted → Submitted");
    let job = env.create_job(3)?;
    env.fund_job(&job)?;
    env.accept_job(&job)?;
    let receipt = env.make_receipt(&job, 3);
    env.submit_receipt(&job, &receipt)?;
    step("Challenged");
    env.challenge_work(&job)?;
    env.assert_job_state(&job, JobState::Challenged)?;
    step("Challenge window elapsed → Settled (timeout)");
    // challenge_work uses clock at START_TS; deadline = now + CHALLENGE_SECS
    env.set_clock(START_TS + CHALLENGE_SECS);
    env.resolve_timeout_settle(&job, true)?;

    env.assert_job_state(&job, JobState::Settled)?;
    let after = env.token_balance(&provider_ata)?;
    if after != before + JOB_AMOUNT {
        bail!(
            "provider after challenge timeout settle: expected {}, got {after}",
            before + JOB_AMOUNT
        );
    }
    println!("  state={:?} {}", JobState::Settled, env.balances_line()?);
    Ok(())
}

fn scenario_admin_slash() -> Result<()> {
    println!("\n[4] Admin slash");
    let mut env = setup()?;
    env.bootstrap_ready()?;
    let buyer_ata = env.create_ata(&env.buyer.pubkey())?;

    step("Created → Funded → Accepted → Submitted → Challenged");
    let job = env.create_job(4)?;
    env.fund_job(&job)?;
    env.accept_job(&job)?;
    let receipt = env.make_receipt(&job, 4);
    env.submit_receipt(&job, &receipt)?;
    env.challenge_work(&job)?;
    let before = env.token_balance(&buyer_ata)?;
    step("Slashed (admin)");
    env.slash_bond(&job)?;

    env.assert_job_state(&job, JobState::Slashed)?;
    let after = env.token_balance(&buyer_ata)?;
    let expected = before + JOB_AMOUNT + MIN_BOND;
    if after < expected {
        bail!("buyer after slash: expected at least {expected}, got {after} (before={before})");
    }
    let job_acc = env.read_job(&job)?;
    if job_acc.locked_bond != 0 {
        bail!("job locked_bond not cleared: {}", job_acc.locked_bond);
    }
    println!("  state={:?} {}", JobState::Slashed, env.balances_line()?);
    Ok(())
}

fn scenario_receipt_replay() -> Result<()> {
    println!("\n[5] Receipt replay rejection");
    let mut env = setup()?;
    env.bootstrap_ready()?;

    step("Created → Funded → Accepted → Submitted");
    let job = env.create_job(5)?;
    env.fund_job(&job)?;
    env.accept_job(&job)?;
    let receipt = env.make_receipt(&job, 5);
    env.submit_receipt(&job, &receipt)?;
    env.assert_job_state(&job, JobState::Submitted)?;

    step("Replay same receipt → InvalidStateTransition");
    let ixs = env.submit_receipt_ixs(&job, &receipt, &env.exec)?;
    let payer = env.provider.insecure_clone();
    env.send_err_code(&payer, &ixs, &[], ProtocolError::InvalidStateTransition)?;

    println!(
        "  state={:?} (replay rejected) {}",
        JobState::Submitted,
        env.balances_line()?
    );
    Ok(())
}

async fn scenario_x402_mock() -> Result<()> {
    println!("\n[6] Local x402 402 → verify → settle → 200 (MockFacilitatorClient)");
    let out = x402_demo::run_x402_demo().await?;
    if out.status_without_payment != 402 || out.status_with_payment != 200 {
        bail!(
            "x402 statuses: without={} with={}",
            out.status_without_payment,
            out.status_with_payment
        );
    }
    if out.payment_response_header.is_empty() {
        bail!("missing PAYMENT-RESPONSE header after settle");
    }
    println!(
        "  statuses {}→{} body.service={}",
        out.status_without_payment,
        out.status_with_payment,
        out.body
            .get("service")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
    );
    Ok(())
}

pub fn run_all_sync_scenarios() -> Result<()> {
    env::require_program_so()?;
    scenario_honest_settlement()?;
    scenario_provider_timeout_refund()?;
    scenario_challenge_timeout_settle()?;
    scenario_admin_slash()?;
    scenario_receipt_replay()?;
    Ok(())
}

pub async fn run_all() -> Result<()> {
    println!("AgentBond local simulator (LiteSVM + mock x402)");
    println!("program.so: {}", env::program_so_path().display());
    run_all_sync_scenarios()?;
    scenario_x402_mock().await?;
    println!("\nAll six scenarios passed.");
    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(err) = run_all().await {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_onchain_scenarios_and_balances() {
        env::require_program_so().expect(
            "missing target/deploy/agentbond.so — run `cargo build-sbf` from the repo root",
        );

        // 1) Honest settlement balances
        {
            let mut env = setup().expect("setup");
            env.bootstrap_ready().expect("bootstrap");
            let provider_ata = env.create_ata(&env.provider.pubkey()).expect("ata");
            let before = env.token_balance(&provider_ata).expect("bal");
            let job = env.create_job(101).expect("job");
            env.fund_job(&job).expect("fund");
            env.accept_job(&job).expect("accept");
            let receipt = env.make_receipt(&job, 101);
            env.submit_receipt(&job, &receipt).expect("submit");
            env.accept_work(&job).expect("settle");
            env.assert_job_state(&job, JobState::Settled)
                .expect("state");
            assert_eq!(
                env.token_balance(&provider_ata).expect("bal"),
                before + JOB_AMOUNT
            );
            assert_eq!(env.read_bond().expect("bond").locked, 0);
        }

        // 2) Timeout refund balances
        {
            let mut env = setup().expect("setup");
            env.bootstrap_ready().expect("bootstrap");
            let buyer_ata = env.create_ata(&env.buyer.pubkey()).expect("ata");
            let job = env.create_job(102).expect("job");
            env.fund_job(&job).expect("fund");
            let after_fund = env.token_balance(&buyer_ata).expect("bal");
            env.accept_job(&job).expect("accept");
            env.set_clock(START_TS + 301);
            env.resolve_timeout_refund(&job).expect("refund");
            env.assert_job_state(&job, JobState::Refunded)
                .expect("state");
            assert_eq!(
                env.token_balance(&buyer_ata).expect("bal"),
                after_fund + JOB_AMOUNT
            );
            assert_eq!(env.read_bond().expect("bond").locked, 0);
        }

        // 3) Challenge timeout settle
        {
            let mut env = setup().expect("setup");
            env.bootstrap_ready().expect("bootstrap");
            let provider_ata = env.create_ata(&env.provider.pubkey()).expect("ata");
            let before = env.token_balance(&provider_ata).expect("bal");
            let job = env.create_job(103).expect("job");
            env.fund_job(&job).expect("fund");
            env.accept_job(&job).expect("accept");
            let receipt = env.make_receipt(&job, 103);
            env.submit_receipt(&job, &receipt).expect("submit");
            env.challenge_work(&job).expect("challenge");
            env.set_clock(START_TS + CHALLENGE_SECS);
            env.resolve_timeout_settle(&job, true).expect("settle");
            env.assert_job_state(&job, JobState::Settled)
                .expect("state");
            assert_eq!(
                env.token_balance(&provider_ata).expect("bal"),
                before + JOB_AMOUNT
            );
        }

        // 4) Admin slash balances
        {
            let mut env = setup().expect("setup");
            env.bootstrap_ready().expect("bootstrap");
            let buyer_ata = env.create_ata(&env.buyer.pubkey()).expect("ata");
            let job = env.create_job(104).expect("job");
            env.fund_job(&job).expect("fund");
            env.accept_job(&job).expect("accept");
            let receipt = env.make_receipt(&job, 104);
            env.submit_receipt(&job, &receipt).expect("submit");
            env.challenge_work(&job).expect("challenge");
            let before = env.token_balance(&buyer_ata).expect("bal");
            env.slash_bond(&job).expect("slash");
            env.assert_job_state(&job, JobState::Slashed)
                .expect("state");
            assert!(env.token_balance(&buyer_ata).expect("bal") >= before + JOB_AMOUNT + MIN_BOND);
            assert_eq!(env.read_job(&job).expect("job").locked_bond, 0);
        }

        // 5) Replay fails
        {
            let mut env = setup().expect("setup");
            env.bootstrap_ready().expect("bootstrap");
            let job = env.create_job(105).expect("job");
            env.fund_job(&job).expect("fund");
            env.accept_job(&job).expect("accept");
            let receipt = env.make_receipt(&job, 105);
            env.submit_receipt(&job, &receipt).expect("submit");
            let ixs = env
                .submit_receipt_ixs(&job, &receipt, &env.exec)
                .expect("ixs");
            let payer = env.provider.insecure_clone();
            env.send_err_code(&payer, &ixs, &[], ProtocolError::InvalidStateTransition)
                .expect("replay must fail");
            env.assert_job_state(&job, JobState::Submitted)
                .expect("still submitted");
        }
    }

    #[tokio::test]
    async fn x402_mock_facilitator_flow() {
        let out = x402_demo::run_x402_demo().await.expect("x402 demo");
        assert_eq!(out.status_without_payment, 402);
        assert_eq!(out.status_with_payment, 200);
        assert!(!out.payment_response_header.is_empty());
        assert_eq!(
            out.body.get("service").and_then(|v| v.as_str()),
            Some("agentbond-x402-demo")
        );
    }

    #[tokio::test]
    async fn all_six_scenarios_complete() {
        run_all().await.expect("all scenarios");
    }
}
