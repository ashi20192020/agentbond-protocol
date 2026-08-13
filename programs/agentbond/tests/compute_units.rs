mod common;

use common::{setup, JOB_AMOUNT, START_TS};
use solana_signer::Signer;
use spl_token::ID as TOKEN_PROGRAM_ID;

fn print_cu(label: &str, cu: u64) {
    println!("MEASURED_CU {label}={cu}");
    assert!(cu > 0, "{label} CU must be recorded");
}

#[test]
fn measure_important_instructions() {
    // FundJob
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(1);
    let buyer_ata = env.create_ata(&env.buyer.pubkey());
    env.mint_to(&buyer_ata, JOB_AMOUNT * 2);
    let escrow = env.ensure_escrow(&job);
    let fund_ix = env.ix_fund_job(&job, buyer_ata, escrow, TOKEN_PROGRAM_ID);
    let buyer = env.buyer.insecure_clone();
    let fund_cu = env.send_cu(&buyer, &[fund_ix], &[]);
    print_cu("FundJob", fund_cu);

    // AcceptJob
    let accept_ix = env.ix_accept_job(&job);
    let provider = env.provider.insecure_clone();
    let accept_cu = env.send_cu(&provider, &[accept_ix], &[]);
    print_cu("AcceptJob", accept_cu);

    // SubmitReceipt
    let receipt = env.make_receipt(&job, 1);
    let submit_cu = env.submit_receipt(&job, &receipt);
    print_cu("SubmitReceipt", submit_cu);

    // ChallengeWork then settle path measured on a fresh job below.
    let challenge_cu = env.challenge_work(&job);
    print_cu("ChallengeWork", challenge_cu);

    // SlashBond
    let slash_cu = env.slash_bond(&job);
    print_cu("SlashBond", slash_cu);

    // AcceptWork
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(2);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 2);
    env.submit_receipt(&job, &receipt);
    let accept_work_cu = env.accept_work(&job);
    print_cu("AcceptWork", accept_work_cu);

    // ResolveTimeoutSettle Submitted
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(3);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 3);
    env.submit_receipt(&job, &receipt);
    env.set_clock(START_TS + 400);
    let settle_sub_cu = env.resolve_timeout_settle(&job, false);
    print_cu("ResolveTimeoutSettle_Submitted", settle_sub_cu);

    // ResolveTimeoutSettle Challenged
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(4);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 4);
    env.submit_receipt(&job, &receipt);
    env.challenge_work(&job);
    env.set_clock(START_TS + 10 + 3_600);
    let settle_ch_cu = env.resolve_timeout_settle(&job, true);
    print_cu("ResolveTimeoutSettle_Challenged", settle_ch_cu);

    // ResolveTimeoutRefund Funded
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(5);
    env.fund_job(&job);
    env.set_clock(START_TS + 200);
    let refund_funded_cu = env.resolve_timeout_refund(&job);
    print_cu("ResolveTimeoutRefund_Funded", refund_funded_cu);

    // ResolveTimeoutRefund Accepted
    let mut env = setup();
    env.bootstrap_ready();
    let job = env.create_job(6);
    env.fund_job(&job);
    env.accept_job(&job);
    env.set_clock(START_TS + 301);
    let refund_accepted_cu = env.resolve_timeout_refund(&job);
    print_cu("ResolveTimeoutRefund_Accepted", refund_accepted_cu);
}
