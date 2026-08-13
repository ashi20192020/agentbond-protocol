mod common;

use agentbond::ed25519::parse_ed25519_instruction_data;
use agentbond_types::{encode_submit_receipt, ProtocolError, RECEIPT_ENCODED_LEN};
use common::{
    ed25519_ix_custom, ed25519_program_id, instructions_sysvar_id, new_ed25519_instruction, setup,
    Env, START_TS,
};
use ed25519_dalek::{Keypair as DalekKeypair, PublicKey, SecretKey, Signer as DalekSigner};
use solana_instruction::{AccountMeta, Instruction};
use solana_signer::Signer;

type ReceiptMut = Box<dyn Fn(&mut agentbond_types::AgentBondWorkReceiptV1)>;

fn ready_accepted(
    env: &mut Env,
    nonce: u64,
) -> (
    solana_pubkey::Pubkey,
    agentbond_types::AgentBondWorkReceiptV1,
) {
    env.bootstrap_ready();
    let job = env.create_job(nonce);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, nonce);
    (job, receipt)
}

#[test]
fn valid_receipt_and_digest() {
    let mut env = setup();
    let (job, receipt) = ready_accepted(&mut env, 1);
    let digest = receipt.digest().expect("digest");
    env.submit_receipt(&job, &receipt);
    assert_eq!(env.read_job(&job).receipt_digest, digest);
    env.assert_job_state(&job, agentbond_types::JobState::Submitted);
}

#[test]
fn receipt_field_mismatches() {
    let cases: Vec<(&str, ReceiptMut)> = vec![
        ("program", Box::new(|r| r.program_id = [0xab; 32])),
        ("genesis", Box::new(|r| r.genesis_hash = [0xcd; 32])),
        ("job", Box::new(|r| r.job = [0x11; 32])),
        ("buyer", Box::new(|r| r.buyer = [0x22; 32])),
        ("provider", Box::new(|r| r.provider = [0x33; 32])),
        ("request", Box::new(|r| r.request_hash = [0x44; 32])),
        ("nonce", Box::new(|r| r.job_nonce = 999)),
    ];
    for (i, (name, mutate)) in cases.into_iter().enumerate() {
        let mut env = setup();
        let (job, mut receipt) = ready_accepted(&mut env, 10 + i as u64);
        mutate(&mut receipt);
        let ixs = env.submit_receipt_ixs(&job, &receipt, &env.exec);
        let payer = env.provider.insecure_clone();
        env.send_err_code(&payer, &ixs, &[], ProtocolError::InvalidReceiptField);
        let _ = name;
    }
}

#[test]
fn receipt_time_boundaries() {
    let mut env = setup();
    let (job, mut receipt) = ready_accepted(&mut env, 20);
    receipt.created_at = START_TS + 1;
    let ixs = env.submit_receipt_ixs(&job, &receipt, &env.exec);
    let payer = env.provider.insecure_clone();
    env.send_err_code(&payer, &ixs, &[], ProtocolError::FutureTimestamp);

    let mut env = setup();
    let (job, mut receipt) = ready_accepted(&mut env, 21);
    receipt.expires_at = START_TS - 1;
    receipt.created_at = START_TS - 10;
    let ixs = env.submit_receipt_ixs(&job, &receipt, &env.exec);
    let payer = env.provider.insecure_clone();
    env.send_err_code(&payer, &ixs, &[], ProtocolError::ReceiptExpired);

    // Exact expiry boundary allowed (expires_at == now)
    let mut env = setup();
    let (job, mut receipt) = ready_accepted(&mut env, 22);
    receipt.expires_at = START_TS;
    env.submit_receipt(&job, &receipt);
    env.assert_job_state(&job, agentbond_types::JobState::Submitted);
}

#[test]
fn revoked_and_unregistered_keys() {
    let mut env = setup();
    env.bootstrap_ready();
    let key = env.exec.public.to_bytes();
    env.revoke_execution_key(&key);
    let job = env.create_job(30);
    env.fund_job(&job);
    env.accept_job(&job);
    let receipt = env.make_receipt(&job, 30);
    let ixs = env.submit_receipt_ixs(&job, &receipt, &env.exec);
    let payer = env.provider.insecure_clone();
    env.send_err_code(&payer, &ixs, &[], ProtocolError::InvalidSignature);

    let mut env = setup();
    let (job, receipt) = ready_accepted(&mut env, 31);
    let secret = SecretKey::from_bytes(&[9u8; 32]).expect("secret");
    let public = PublicKey::from(&secret);
    let unknown = DalekKeypair { secret, public };
    let ixs = env.submit_receipt_ixs(&job, &receipt, &unknown);
    let payer = env.provider.insecure_clone();
    env.send_err_code(&payer, &ixs, &[], ProtocolError::InvalidSignature);
}

#[test]
fn cross_job_program_cluster_replay() {
    let mut env = setup();
    env.bootstrap_ready();
    let job_a = env.create_job(40);
    env.fund_job(&job_a);
    env.accept_job(&job_a);
    let job_b = env.create_job(41);
    env.fund_job(&job_b);
    env.accept_job(&job_b);
    let receipt_a = env.make_receipt(&job_a, 40);
    // Replay receipt A against job B
    let ixs = env.submit_receipt_ixs(&job_b, &receipt_a, &env.exec);
    let payer = env.provider.insecure_clone();
    env.send_err_code(&payer, &ixs, &[], ProtocolError::InvalidReceiptField);

    // Cross-program: wrong program_id already covered; cluster via genesis
    let mut receipt = env.make_receipt(&job_b, 41);
    receipt.genesis_hash = [1u8; 32];
    let ixs = env.submit_receipt_ixs(&job_b, &receipt, &env.exec);
    env.send_err_code(&payer, &ixs, &[], ProtocolError::InvalidReceiptField);
}

#[test]
fn reuse_after_terminal_settlement() {
    let mut env = setup();
    let (job, receipt) = ready_accepted(&mut env, 50);
    env.submit_receipt(&job, &receipt);
    env.accept_work(&job);
    let ixs = env.submit_receipt_ixs(&job, &receipt, &env.exec);
    let payer = env.provider.insecure_clone();
    env.send_err_code(&payer, &ixs, &[], ProtocolError::InvalidStateTransition);
}

#[test]
fn ed25519_missing_wrong_position_wrong_program() {
    let mut env = setup();
    let (job, receipt) = ready_accepted(&mut env, 60);
    let payer = env.provider.insecure_clone();
    let submit = Instruction {
        program_id: env.program_id,
        accounts: vec![
            AccountMeta::new_readonly(env.config_pda(), false),
            AccountMeta::new_readonly(env.provider_pda(), false),
            AccountMeta::new(job, false),
            AccountMeta::new_readonly(instructions_sysvar_id(), false),
        ],
        data: encode_submit_receipt(&receipt).expect("e").to_vec(),
    };
    env.send_err_code(
        &payer,
        std::slice::from_ref(&submit),
        &[],
        ProtocolError::MissingEd25519Instruction,
    );

    // Ed25519 not immediately before AgentBond.
    let encoded = receipt.encode().expect("enc");
    let ed = Env::ed25519_ix(&encoded, &env.exec);
    let ed_data = ed.data.clone();
    let spacer =
        solana_system_interface::instruction::transfer(&payer.pubkey(), &payer.pubkey(), 0);
    env.send_err_any(&payer, &[ed, spacer, submit.clone()], &[]);

    // Non-Ed25519 program immediately before AgentBond.
    let _ = ed_data;
    let bad_prog = common::budget_ix();
    env.send_err_code(
        &payer,
        &[bad_prog, submit],
        &[],
        ProtocolError::MissingEd25519Instruction,
    );
}

#[test]
fn ed25519_malformed_layouts_via_litesvm() {
    let mut env = setup();
    let (job, receipt) = ready_accepted(&mut env, 70);
    let encoded = receipt.encode().expect("enc");
    let payer = env.provider.insecure_clone();
    let submit = Instruction {
        program_id: env.program_id,
        accounts: vec![
            AccountMeta::new_readonly(env.config_pda(), false),
            AccountMeta::new_readonly(env.provider_pda(), false),
            AccountMeta::new(job, false),
            AccountMeta::new_readonly(instructions_sysvar_id(), false),
        ],
        data: encode_submit_receipt(&receipt).expect("e").to_vec(),
    };

    let cases: Vec<Vec<u8>> = vec![
        vec![],     // truncated header
        vec![0, 0], // zero signatures
        vec![2, 0], // multi sig header only
        {
            let mut d = vec![1u8, 0];
            d.extend_from_slice(&[0u8; 10]); // truncated offsets
            d
        },
        {
            // wrong message length
            let sig = env.exec.sign(&encoded).to_bytes();
            let pk = env.exec.public.to_bytes();
            let mut ix = new_ed25519_instruction(&encoded, &sig, &pk);
            // patch message size to 10
            ix.data[12..14].copy_from_slice(&10u16.to_le_bytes());
            ix.data
        },
        {
            // wrong message bytes
            let mut msg = encoded;
            msg[0] ^= 0xff;
            let sig = env.exec.sign(&encoded).to_bytes();
            let pk = env.exec.public.to_bytes();
            new_ed25519_instruction(&msg, &sig, &pk).data
        },
        {
            // wrong pubkey
            let sig = env.exec.sign(&encoded).to_bytes();
            let pk = [9u8; 32];
            new_ed25519_instruction(&encoded, &sig, &pk).data
        },
        {
            // out-of-bounds signature offset
            let mut d = vec![0u8; 16];
            d[0] = 1;
            d[2] = 0xff;
            d[3] = 0xff;
            d
        },
        {
            // wrong instruction indices (not self / not u16::MAX)
            let sig = env.exec.sign(&encoded).to_bytes();
            let pk = env.exec.public.to_bytes();
            let mut ix = new_ed25519_instruction(&encoded, &sig, &pk);
            ix.data[4..6].copy_from_slice(&1u16.to_le_bytes()); // sig ix index = 1
            ix.data
        },
    ];

    for data in cases {
        let ed = ed25519_ix_custom(data, ed25519_program_id());
        env.send_err_any(&payer, &[ed, submit.clone()], &[]);
    }
}

#[test]
fn ed25519_parser_unit_coverage() {
    let message = [3u8; RECEIPT_ENCODED_LEN];
    assert_eq!(
        parse_ed25519_instruction_data(&[1, 0], &message, 0),
        Err(ProtocolError::InvalidEd25519Instruction)
    );
    let mut data = vec![0u8; 16];
    data[0] = 0;
    assert_eq!(
        parse_ed25519_instruction_data(&data, &message, 0),
        Err(ProtocolError::InvalidEd25519Instruction)
    );
    data[0] = 2;
    assert_eq!(
        parse_ed25519_instruction_data(&data, &message, 0),
        Err(ProtocolError::InvalidEd25519Instruction)
    );
}
