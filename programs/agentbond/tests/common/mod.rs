#![allow(dead_code)]

use std::path::PathBuf;

use agentbond::{
    bond_address, challenge_address, config_address, job_address, provider_address, ID,
};
use agentbond_types::{
    encode_add_execution_key, encode_challenge_work, encode_create_job, encode_deposit_bond,
    encode_empty, encode_initialize_config, encode_revoke_execution_key, encode_set_paused,
    encode_submit_receipt, encode_withdraw_bond, AgentBondWorkReceiptV1, ConfigAccount,
    CreateJobPayload, InitializeConfigPayload, InstructionKind, JobAccount, JobState,
    ProtocolError, ProviderAccount, ProviderBondAccount, PROVIDER_STATUS_INACTIVE,
};
use ed25519_dalek::{Keypair as DalekKeypair, PublicKey, SecretKey, Signer as DalekSigner};
use litesvm::types::FailedTransactionMetadata;
use litesvm::LiteSVM;
use solana_clock::Clock;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_system_interface::instruction as system_instruction;
use solana_transaction::Transaction;
use spl_associated_token_account_client::address::get_associated_token_address;
use spl_associated_token_account_client::instruction::create_associated_token_account;
use spl_token::instruction as token_instruction;
use spl_token::ID as TOKEN_PROGRAM_ID;

pub const DECIMALS: u8 = 6;
pub const MIN_BOND: u64 = 1_000;
pub const JOB_AMOUNT: u64 = 5_000;
pub const GENESIS: [u8; 32] = [7u8; 32];
pub const START_TS: i64 = 1_700_000_000;
pub const CHALLENGE_SECS: i64 = 3_600;
pub const TOKEN_2022: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

pub struct Env {
    pub svm: LiteSVM,
    pub program_id: Pubkey,
    pub admin: Keypair,
    pub buyer: Keypair,
    pub provider: Keypair,
    pub exec: DalekKeypair,
    pub mint: Pubkey,
    pub mint_authority: Keypair,
}

pub fn program_so_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/deploy/agentbond.so")
}

pub fn pk_from_bytes(bytes: [u8; 32]) -> Pubkey {
    Pubkey::new_from_array(bytes)
}

pub fn address_bytes(pk: &Pubkey) -> [u8; 32] {
    pk.to_bytes()
}

pub fn instructions_sysvar_id() -> Pubkey {
    Pubkey::from_str_const("Sysvar1nstructions1111111111111111111111111")
}

pub fn ed25519_program_id() -> Pubkey {
    Pubkey::from_str_const("Ed25519SigVerify111111111111111111111111111")
}

pub fn token_2022_id() -> Pubkey {
    Pubkey::from_str_const(TOKEN_2022)
}

pub fn budget_ix() -> Instruction {
    ComputeBudgetInstruction::set_compute_unit_limit(1_400_000)
}

pub fn setup() -> Env {
    let program_id = pk_from_bytes(ID.to_bytes());
    let mut svm = LiteSVM::new();
    svm.add_program_from_file(program_id, program_so_path())
        .expect("load agentbond.so — run cargo build-sbf first");

    let admin = Keypair::new();
    let buyer = Keypair::new();
    let provider = Keypair::new();
    let mint_authority = Keypair::new();
    let mint = Keypair::new();

    for kp in [&admin, &buyer, &provider, &mint_authority] {
        svm.airdrop(&kp.pubkey(), 100_000_000_000).expect("airdrop");
    }

    let secret = SecretKey::from_bytes(&[42u8; 32]).expect("secret");
    let public = PublicKey::from(&secret);
    let exec = DalekKeypair { secret, public };

    let mut env = Env {
        svm,
        program_id,
        admin,
        buyer,
        provider,
        exec,
        mint: mint.pubkey(),
        mint_authority,
    };
    env.set_clock(START_TS);
    env.create_mint(&mint);
    env
}

pub fn default_deadlines() -> (i64, i64, i64, i64) {
    (
        START_TS + 100,
        START_TS + 200,
        START_TS + 300,
        START_TS + 400,
    )
}

pub fn new_ed25519_instruction(
    message: &[u8],
    signature: &[u8; 64],
    pubkey: &[u8; 32],
) -> Instruction {
    const OFFSETS_START: usize = 2;
    const OFFSETS_SIZE: usize = 14;
    const DATA_START: usize = OFFSETS_START + OFFSETS_SIZE;
    let public_key_offset = DATA_START;
    let signature_offset = public_key_offset + 32;
    let message_data_offset = signature_offset + 64;
    let mut data = Vec::with_capacity(DATA_START + 32 + 64 + message.len());
    data.extend_from_slice(&[1u8, 0u8]);
    data.extend_from_slice(&(signature_offset as u16).to_le_bytes());
    data.extend_from_slice(&u16::MAX.to_le_bytes());
    data.extend_from_slice(&(public_key_offset as u16).to_le_bytes());
    data.extend_from_slice(&u16::MAX.to_le_bytes());
    data.extend_from_slice(&(message_data_offset as u16).to_le_bytes());
    data.extend_from_slice(&(message.len() as u16).to_le_bytes());
    data.extend_from_slice(&u16::MAX.to_le_bytes());
    data.extend_from_slice(pubkey);
    data.extend_from_slice(signature);
    data.extend_from_slice(message);
    Instruction {
        program_id: ed25519_program_id(),
        accounts: vec![],
        data,
    }
}

pub fn ed25519_ix_custom(data: Vec<u8>, program_id: Pubkey) -> Instruction {
    Instruction {
        program_id,
        accounts: vec![],
        data,
    }
}

impl Env {
    pub fn set_clock(&mut self, unix_timestamp: i64) {
        let mut clock = self.svm.get_sysvar::<Clock>();
        clock.unix_timestamp = unix_timestamp;
        self.svm.set_sysvar(&clock);
    }

    #[allow(clippy::result_large_err)]
    pub fn send(
        &mut self,
        payer: &Keypair,
        ixs: &[Instruction],
        signers: &[&Keypair],
    ) -> Result<litesvm::types::TransactionMetadata, FailedTransactionMetadata> {
        let blockhash = self.svm.latest_blockhash();
        let mut all_signers: Vec<&Keypair> = vec![payer];
        for signer in signers {
            if signer.pubkey() != payer.pubkey() {
                all_signers.push(signer);
            }
        }
        let msg = Message::new(ixs, Some(&payer.pubkey()));
        let tx = Transaction::new(&all_signers, msg, blockhash);
        let result = self.svm.send_transaction(tx);
        // LiteSVM rejects identical signatures across retries.
        self.svm.expire_blockhash();
        result
    }

    pub fn send_ok(&mut self, payer: &Keypair, ixs: &[Instruction], signers: &[&Keypair]) {
        self.send(payer, ixs, signers)
            .expect("transaction should succeed");
    }

    pub fn send_cu(&mut self, payer: &Keypair, ixs: &[Instruction], signers: &[&Keypair]) -> u64 {
        let mut all = vec![budget_ix()];
        all.extend_from_slice(ixs);
        self.send(payer, &all, signers)
            .expect("cu tx")
            .compute_units_consumed
    }

    pub fn send_err_code(
        &mut self,
        payer: &Keypair,
        ixs: &[Instruction],
        signers: &[&Keypair],
        code: ProtocolError,
    ) {
        let err = self
            .send(payer, ixs, signers)
            .expect_err("expected failure");
        let text = format!("{:?}", err.err);
        let expected = format!("Custom({})", code.code());
        assert!(
            text.contains(&expected),
            "expected {expected}, got {text} ({err:?})"
        );
    }

    pub fn send_err_any(&mut self, payer: &Keypair, ixs: &[Instruction], signers: &[&Keypair]) {
        let _ = self
            .send(payer, ixs, signers)
            .expect_err("expected failure");
    }

    pub fn create_mint(&mut self, mint: &Keypair) {
        let rent = self.svm.minimum_balance_for_rent_exemption(82);
        let ixs = [
            system_instruction::create_account(
                &self.admin.pubkey(),
                &mint.pubkey(),
                rent,
                82,
                &TOKEN_PROGRAM_ID,
            ),
            token_instruction::initialize_mint(
                &TOKEN_PROGRAM_ID,
                &mint.pubkey(),
                &self.mint_authority.pubkey(),
                Some(&self.mint_authority.pubkey()),
                DECIMALS,
            )
            .expect("initialize_mint"),
        ];
        let payer = self.admin.insecure_clone();
        self.send_ok(&payer, &ixs, &[mint]);
        self.mint = mint.pubkey();
    }

    pub fn create_ata(&mut self, owner: &Pubkey) -> Pubkey {
        let ata = get_associated_token_address(owner, &self.mint);
        if self.svm.get_account(&ata).is_some() {
            return ata;
        }
        let ix = create_associated_token_account(
            &self.admin.pubkey(),
            owner,
            &self.mint,
            &TOKEN_PROGRAM_ID,
        );
        let payer = self.admin.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
        ata
    }

    pub fn ensure_ata(&mut self, owner: &Pubkey) -> Pubkey {
        self.create_ata(owner)
    }

    pub fn mint_to(&mut self, ata: &Pubkey, amount: u64) {
        let ix = token_instruction::mint_to(
            &TOKEN_PROGRAM_ID,
            &self.mint,
            ata,
            &self.mint_authority.pubkey(),
            &[],
            amount,
        )
        .expect("mint_to");
        let payer = self.mint_authority.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn freeze_account(&mut self, ata: &Pubkey) {
        let ix = token_instruction::freeze_account(
            &TOKEN_PROGRAM_ID,
            ata,
            &self.mint,
            &self.mint_authority.pubkey(),
            &[],
        )
        .expect("freeze");
        let payer = self.mint_authority.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn approve_delegate(&mut self, ata: &Pubkey, delegate: &Pubkey, amount: u64) {
        let owner = self.buyer.insecure_clone();
        // Resolve owner from token account when possible; tests pass the owning keypair via signers.
        let ix = token_instruction::approve(
            &TOKEN_PROGRAM_ID,
            ata,
            delegate,
            &owner.pubkey(),
            &[],
            amount,
        )
        .expect("approve");
        // Caller must use the correct owner; this helper assumes buyer for buyer ATAs.
        self.send_ok(&owner, &[ix], &[]);
    }

    pub fn approve_delegate_for(
        &mut self,
        owner: &Keypair,
        ata: &Pubkey,
        delegate: &Pubkey,
        amount: u64,
    ) {
        let ix = token_instruction::approve(
            &TOKEN_PROGRAM_ID,
            ata,
            delegate,
            &owner.pubkey(),
            &[],
            amount,
        )
        .expect("approve");
        self.send_ok(owner, &[ix], &[]);
    }

    pub fn config_pda(&self) -> Pubkey {
        let (addr, _) = config_address(&ID).expect("config");
        pk_from_bytes(addr.to_bytes())
    }

    pub fn provider_pda(&self) -> Pubkey {
        let authority = address_bytes(&self.provider.pubkey());
        let (addr, _) = provider_address(&ID, &authority).expect("provider");
        pk_from_bytes(addr.to_bytes())
    }

    pub fn provider_pda_for(&self, authority: &Pubkey) -> Pubkey {
        let (addr, _) = provider_address(&ID, &address_bytes(authority)).expect("provider");
        pk_from_bytes(addr.to_bytes())
    }

    pub fn bond_pda(&self) -> Pubkey {
        let authority = address_bytes(&self.provider.pubkey());
        let mint = address_bytes(&self.mint);
        let (addr, _) = bond_address(&ID, &authority, &mint).expect("bond");
        pk_from_bytes(addr.to_bytes())
    }

    pub fn bond_vault(&self) -> Pubkey {
        get_associated_token_address(&self.bond_pda(), &self.mint)
    }

    pub fn job_pda(&self, nonce: u64) -> Pubkey {
        let buyer = address_bytes(&self.buyer.pubkey());
        let provider = address_bytes(&self.provider.pubkey());
        let (addr, _) = job_address(&ID, &buyer, &provider, nonce).expect("job");
        pk_from_bytes(addr.to_bytes())
    }

    pub fn challenge_pda(&self, job: &Pubkey) -> Pubkey {
        let (addr, _) = challenge_address(&ID, &address_bytes(job)).expect("challenge");
        pk_from_bytes(addr.to_bytes())
    }

    pub fn escrow_ata(&self, job: &Pubkey) -> Pubkey {
        get_associated_token_address(job, &self.mint)
    }

    pub fn ensure_escrow(&mut self, job: &Pubkey) -> Pubkey {
        let escrow = self.escrow_ata(job);
        if self.svm.get_account(&escrow).is_none() {
            let create_escrow = create_associated_token_account(
                &self.buyer.pubkey(),
                job,
                &self.mint,
                &TOKEN_PROGRAM_ID,
            );
            let payer = self.buyer.insecure_clone();
            self.send_ok(&payer, &[create_escrow], &[]);
        }
        escrow
    }

    pub fn ensure_bond_vault(&mut self) -> Pubkey {
        let bond_pda = self.bond_pda();
        let vault = self.bond_vault();
        if self.svm.get_account(&vault).is_none() {
            let create_vault = create_associated_token_account(
                &self.provider.pubkey(),
                &bond_pda,
                &self.mint,
                &TOKEN_PROGRAM_ID,
            );
            let payer = self.provider.insecure_clone();
            self.send_ok(&payer, &[create_vault], &[]);
        }
        vault
    }

    pub fn ix_initialize_config(&self, payload: &InitializeConfigPayload) -> Instruction {
        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(self.admin.pubkey(), true),
                AccountMeta::new(self.config_pda(), false),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
            data: encode_initialize_config(payload).to_vec(),
        }
    }

    pub fn default_config_payload(&self) -> InitializeConfigPayload {
        InitializeConfigPayload {
            genesis_hash: GENESIS,
            allowed_mint: address_bytes(&self.mint),
            token_program: TOKEN_PROGRAM_ID.to_bytes(),
            mint_decimals: DECIMALS,
            min_provider_bond: MIN_BOND,
            challenge_duration_seconds: CHALLENGE_SECS,
        }
    }

    pub fn initialize_config(&mut self) {
        let payload = self.default_config_payload();
        let ix = self.ix_initialize_config(&payload);
        let payer = self.admin.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn ix_set_paused(&self, paused: bool) -> Instruction {
        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.admin.pubkey(), true),
                AccountMeta::new(self.config_pda(), false),
            ],
            data: encode_set_paused(paused).to_vec(),
        }
    }

    pub fn set_paused(&mut self, paused: bool) {
        let ix = self.ix_set_paused(paused);
        let payer = self.admin.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn ix_register_provider(&self) -> Instruction {
        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(self.provider.pubkey(), true),
                AccountMeta::new_readonly(self.config_pda(), false),
                AccountMeta::new(self.provider_pda(), false),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
            data: encode_empty(InstructionKind::RegisterProvider)
                .expect("empty")
                .to_vec(),
        }
    }

    pub fn register_provider(&mut self) {
        let ix = self.ix_register_provider();
        let payer = self.provider.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn ix_add_key(&self, key: &[u8; 32]) -> Instruction {
        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.provider.pubkey(), true),
                AccountMeta::new(self.provider_pda(), false),
            ],
            data: encode_add_execution_key(key).to_vec(),
        }
    }

    pub fn add_execution_key(&mut self) {
        let key = self.exec.public.to_bytes();
        let ix = self.ix_add_key(&key);
        let payer = self.provider.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn add_execution_key_bytes(&mut self, key: &[u8; 32]) {
        let ix = self.ix_add_key(key);
        let payer = self.provider.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn ix_revoke_key(&self, key: &[u8; 32]) -> Instruction {
        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.provider.pubkey(), true),
                AccountMeta::new(self.provider_pda(), false),
            ],
            data: encode_revoke_execution_key(key).to_vec(),
        }
    }

    pub fn revoke_execution_key(&mut self, key: &[u8; 32]) {
        let ix = self.ix_revoke_key(key);
        let payer = self.provider.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn ix_deposit_bond(&self, amount: u64, provider_ata: Pubkey, vault: Pubkey) -> Instruction {
        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(self.provider.pubkey(), true),
                AccountMeta::new_readonly(self.config_pda(), false),
                AccountMeta::new_readonly(self.provider_pda(), false),
                AccountMeta::new(self.bond_pda(), false),
                AccountMeta::new(vault, false),
                AccountMeta::new(provider_ata, false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
            data: encode_deposit_bond(amount).to_vec(),
        }
    }

    pub fn deposit_bond(&mut self, amount: u64) {
        let provider_ata = self.create_ata(&self.provider.pubkey());
        self.mint_to(&provider_ata, amount.saturating_mul(2).max(amount));
        let vault = self.ensure_bond_vault();
        let ix = self.ix_deposit_bond(amount, provider_ata, vault);
        let payer = self.provider.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn ix_withdraw_bond(
        &self,
        amount: u64,
        provider_ata: Pubkey,
        vault: Pubkey,
    ) -> Instruction {
        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.provider.pubkey(), true),
                AccountMeta::new(self.bond_pda(), false),
                AccountMeta::new(vault, false),
                AccountMeta::new(provider_ata, false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ],
            data: encode_withdraw_bond(amount).to_vec(),
        }
    }

    pub fn withdraw_bond(&mut self, amount: u64) {
        let provider_ata = self.create_ata(&self.provider.pubkey());
        let vault = self.bond_vault();
        let ix = self.ix_withdraw_bond(amount, provider_ata, vault);
        let payer = self.provider.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn create_job_payload(&self, nonce: u64) -> CreateJobPayload {
        let (fund, accept, work, auto) = default_deadlines();
        CreateJobPayload {
            job_nonce: nonce,
            amount: JOB_AMOUNT,
            request_hash: [9u8; 32],
            fund_deadline: fund,
            accept_deadline: accept,
            work_deadline: work,
            auto_settle_deadline: auto,
        }
    }

    pub fn ix_create_job(&self, payload: &CreateJobPayload) -> Instruction {
        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(self.buyer.pubkey(), true),
                AccountMeta::new_readonly(self.config_pda(), false),
                AccountMeta::new_readonly(self.provider_pda(), false),
                AccountMeta::new(self.job_pda(payload.job_nonce), false),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
            data: encode_create_job(payload).to_vec(),
        }
    }

    pub fn create_job(&mut self, nonce: u64) -> Pubkey {
        let payload = self.create_job_payload(nonce);
        let job = self.job_pda(nonce);
        let ix = self.ix_create_job(&payload);
        let payer = self.buyer.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
        job
    }

    pub fn ix_fund_job(
        &self,
        job: &Pubkey,
        buyer_ata: Pubkey,
        escrow: Pubkey,
        token_program: Pubkey,
    ) -> Instruction {
        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.buyer.pubkey(), true),
                AccountMeta::new_readonly(self.config_pda(), false),
                AccountMeta::new(*job, false),
                AccountMeta::new(buyer_ata, false),
                AccountMeta::new(escrow, false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new_readonly(token_program, false),
            ],
            data: encode_empty(InstructionKind::FundJob)
                .expect("empty")
                .to_vec(),
        }
    }

    pub fn fund_job(&mut self, job: &Pubkey) {
        let buyer_ata = self.create_ata(&self.buyer.pubkey());
        self.mint_to(&buyer_ata, JOB_AMOUNT * 2);
        let escrow = self.ensure_escrow(job);
        let ix = self.ix_fund_job(job, buyer_ata, escrow, TOKEN_PROGRAM_ID);
        let payer = self.buyer.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn ix_accept_job(&self, job: &Pubkey) -> Instruction {
        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.provider.pubkey(), true),
                AccountMeta::new_readonly(self.config_pda(), false),
                AccountMeta::new_readonly(self.provider_pda(), false),
                AccountMeta::new(self.bond_pda(), false),
                AccountMeta::new(*job, false),
            ],
            data: encode_empty(InstructionKind::AcceptJob)
                .expect("empty")
                .to_vec(),
        }
    }

    pub fn accept_job(&mut self, job: &Pubkey) {
        let ix = self.ix_accept_job(job);
        let payer = self.provider.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn make_receipt(&self, job: &Pubkey, nonce: u64) -> AgentBondWorkReceiptV1 {
        AgentBondWorkReceiptV1 {
            program_id: address_bytes(&self.program_id),
            genesis_hash: GENESIS,
            job: address_bytes(job),
            buyer: address_bytes(&self.buyer.pubkey()),
            provider: address_bytes(&self.provider.pubkey()),
            request_hash: [9u8; 32],
            result_hash: [1u8; 32],
            artifact_hash: [2u8; 32],
            software_hash: [3u8; 32],
            job_nonce: nonce,
            created_at: START_TS,
            expires_at: START_TS + 350,
        }
    }

    pub fn ed25519_ix(message: &[u8], keypair: &DalekKeypair) -> Instruction {
        let signature = keypair.sign(message).to_bytes();
        let pubkey = keypair.public.to_bytes();
        new_ed25519_instruction(message, &signature, &pubkey)
    }

    pub fn submit_receipt_ixs(
        &self,
        job: &Pubkey,
        receipt: &AgentBondWorkReceiptV1,
        keypair: &DalekKeypair,
    ) -> Vec<Instruction> {
        let encoded = receipt.encode().expect("encode");
        let ed_ix = Self::ed25519_ix(&encoded, keypair);
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.config_pda(), false),
                AccountMeta::new_readonly(self.provider_pda(), false),
                AccountMeta::new(*job, false),
                AccountMeta::new_readonly(instructions_sysvar_id(), false),
            ],
            data: encode_submit_receipt(receipt).expect("submit").to_vec(),
        };
        vec![ed_ix, ix]
    }

    pub fn submit_receipt(&mut self, job: &Pubkey, receipt: &AgentBondWorkReceiptV1) -> u64 {
        let ixs = self.submit_receipt_ixs(job, receipt, &self.exec);
        let payer = self.provider.insecure_clone();
        self.send_cu(&payer, &ixs, &[])
    }

    pub fn ix_accept_work(&self, job: &Pubkey) -> Instruction {
        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.buyer.pubkey(), true),
                AccountMeta::new(*job, false),
                AccountMeta::new(self.bond_pda(), false),
                AccountMeta::new(self.escrow_ata(job), false),
                AccountMeta::new(
                    get_associated_token_address(&self.provider.pubkey(), &self.mint),
                    false,
                ),
                AccountMeta::new(
                    get_associated_token_address(&self.buyer.pubkey(), &self.mint),
                    false,
                ),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ],
            data: encode_empty(InstructionKind::AcceptWork)
                .expect("empty")
                .to_vec(),
        }
    }

    pub fn accept_work(&mut self, job: &Pubkey) -> u64 {
        let _ = self.create_ata(&self.provider.pubkey());
        let _ = self.create_ata(&self.buyer.pubkey());
        let ix = self.ix_accept_work(job);
        let payer = self.buyer.insecure_clone();
        self.send_cu(&payer, &[ix], &[])
    }

    pub fn ix_challenge_work(&self, job: &Pubkey) -> Instruction {
        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(self.buyer.pubkey(), true),
                AccountMeta::new_readonly(self.config_pda(), false),
                AccountMeta::new(*job, false),
                AccountMeta::new(self.challenge_pda(job), false),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
            data: encode_challenge_work(&[8u8; 32]).to_vec(),
        }
    }

    pub fn challenge_work(&mut self, job: &Pubkey) -> u64 {
        let ix = self.ix_challenge_work(job);
        let payer = self.buyer.insecure_clone();
        self.send_cu(&payer, &[ix], &[])
    }

    pub fn ix_resolve_timeout_settle(&self, job: &Pubkey, with_challenge: bool) -> Instruction {
        let mut accounts = vec![
            AccountMeta::new_readonly(self.admin.pubkey(), true),
            AccountMeta::new(*job, false),
            AccountMeta::new(self.bond_pda(), false),
            AccountMeta::new(self.escrow_ata(job), false),
            AccountMeta::new(
                get_associated_token_address(&self.provider.pubkey(), &self.mint),
                false,
            ),
            AccountMeta::new(
                get_associated_token_address(&self.buyer.pubkey(), &self.mint),
                false,
            ),
            AccountMeta::new(self.buyer.pubkey(), false),
            AccountMeta::new_readonly(self.mint, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ];
        if with_challenge {
            accounts.push(AccountMeta::new(self.challenge_pda(job), false));
        }
        Instruction {
            program_id: self.program_id,
            accounts,
            data: encode_empty(InstructionKind::ResolveTimeoutSettle)
                .expect("empty")
                .to_vec(),
        }
    }

    pub fn resolve_timeout_settle(&mut self, job: &Pubkey, with_challenge: bool) -> u64 {
        let _ = self.create_ata(&self.provider.pubkey());
        let _ = self.create_ata(&self.buyer.pubkey());
        let ix = self.ix_resolve_timeout_settle(job, with_challenge);
        let payer = self.admin.insecure_clone();
        self.send_cu(&payer, &[ix], &[])
    }

    pub fn resolve_timeout_settle_as(
        &mut self,
        job: &Pubkey,
        with_challenge: bool,
        payer: &Keypair,
    ) -> u64 {
        let _ = self.create_ata(&self.provider.pubkey());
        let _ = self.create_ata(&self.buyer.pubkey());
        let mut accounts = vec![
            AccountMeta::new_readonly(payer.pubkey(), true),
            AccountMeta::new(*job, false),
            AccountMeta::new(self.bond_pda(), false),
            AccountMeta::new(self.escrow_ata(job), false),
            AccountMeta::new(
                get_associated_token_address(&self.provider.pubkey(), &self.mint),
                false,
            ),
            AccountMeta::new(
                get_associated_token_address(&self.buyer.pubkey(), &self.mint),
                false,
            ),
            AccountMeta::new(self.buyer.pubkey(), false),
            AccountMeta::new_readonly(self.mint, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ];
        if with_challenge {
            accounts.push(AccountMeta::new(self.challenge_pda(job), false));
        }
        let ix = Instruction {
            program_id: self.program_id,
            accounts,
            data: encode_empty(InstructionKind::ResolveTimeoutSettle)
                .expect("empty")
                .to_vec(),
        };
        self.send_cu(payer, &[ix], &[])
    }

    pub fn ix_resolve_timeout_refund(&self, job: &Pubkey) -> Instruction {
        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.admin.pubkey(), true),
                AccountMeta::new(*job, false),
                AccountMeta::new(self.bond_pda(), false),
                AccountMeta::new(self.escrow_ata(job), false),
                AccountMeta::new(
                    get_associated_token_address(&self.buyer.pubkey(), &self.mint),
                    false,
                ),
                AccountMeta::new(self.buyer.pubkey(), false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ],
            data: encode_empty(InstructionKind::ResolveTimeoutRefund)
                .expect("empty")
                .to_vec(),
        }
    }

    pub fn resolve_timeout_refund(&mut self, job: &Pubkey) -> u64 {
        let _ = self.create_ata(&self.buyer.pubkey());
        let ix = self.ix_resolve_timeout_refund(job);
        let payer = self.admin.insecure_clone();
        self.send_cu(&payer, &[ix], &[])
    }

    pub fn expire_unfunded(&mut self, job: &Pubkey) {
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.admin.pubkey(), true),
                AccountMeta::new(*job, false),
            ],
            data: encode_empty(InstructionKind::ExpireUnfunded)
                .expect("empty")
                .to_vec(),
        };
        let payer = self.admin.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn expire_unaccepted(&mut self, job: &Pubkey) {
        let _ = self.create_ata(&self.buyer.pubkey());
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.admin.pubkey(), true),
                AccountMeta::new(*job, false),
                AccountMeta::new(self.bond_pda(), false),
                AccountMeta::new(self.escrow_ata(job), false),
                AccountMeta::new(
                    get_associated_token_address(&self.buyer.pubkey(), &self.mint),
                    false,
                ),
                AccountMeta::new(self.buyer.pubkey(), false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ],
            data: encode_empty(InstructionKind::ExpireUnaccepted)
                .expect("empty")
                .to_vec(),
        };
        let payer = self.admin.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn slash_bond(&mut self, job: &Pubkey) -> u64 {
        let _ = self.create_ata(&self.buyer.pubkey());
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.admin.pubkey(), true),
                AccountMeta::new_readonly(self.config_pda(), false),
                AccountMeta::new(*job, false),
                AccountMeta::new(self.bond_pda(), false),
                AccountMeta::new(self.bond_vault(), false),
                AccountMeta::new(self.escrow_ata(job), false),
                AccountMeta::new(
                    get_associated_token_address(&self.buyer.pubkey(), &self.mint),
                    false,
                ),
                AccountMeta::new(self.buyer.pubkey(), false),
                AccountMeta::new(self.challenge_pda(job), false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ],
            data: encode_empty(InstructionKind::SlashBond)
                .expect("empty")
                .to_vec(),
        };
        let payer = self.admin.insecure_clone();
        self.send_cu(&payer, &[ix], &[])
    }

    pub fn ix_close_job(&self, job: &Pubkey, rent_to: Pubkey, include_escrow: bool) -> Instruction {
        let mut accounts = vec![
            AccountMeta::new_readonly(self.buyer.pubkey(), true),
            AccountMeta::new(*job, false),
            AccountMeta::new(rent_to, false),
        ];
        if include_escrow {
            accounts.push(AccountMeta::new(self.escrow_ata(job), false));
            accounts.push(AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false));
        }
        Instruction {
            program_id: self.program_id,
            accounts,
            data: encode_empty(InstructionKind::CloseJob)
                .expect("empty")
                .to_vec(),
        }
    }

    pub fn close_job(&mut self, job: &Pubkey) {
        let escrow = self.escrow_ata(job);
        let include = self.svm.get_account(&escrow).is_some();
        let ix = self.ix_close_job(job, self.buyer.pubkey(), include);
        let payer = self.buyer.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn read_job(&self, job: &Pubkey) -> JobAccount {
        let acc = self.svm.get_account(job).expect("job account");
        JobAccount::decode(&acc.data).expect("decode job")
    }

    pub fn read_bond(&self) -> ProviderBondAccount {
        let bond = self.bond_pda();
        let acc = self.svm.get_account(&bond).expect("bond");
        ProviderBondAccount::decode(&acc.data).expect("decode bond")
    }

    pub fn read_provider(&self) -> ProviderAccount {
        let acc = self
            .svm
            .get_account(&self.provider_pda())
            .expect("provider");
        ProviderAccount::decode(&acc.data).expect("decode provider")
    }

    pub fn read_config(&self) -> ConfigAccount {
        let acc = self.svm.get_account(&self.config_pda()).expect("config");
        ConfigAccount::decode(&acc.data).expect("decode config")
    }

    pub fn token_balance(&self, ata: &Pubkey) -> u64 {
        let acc = self.svm.get_account(ata).expect("ata");
        u64::from_le_bytes(acc.data[64..72].try_into().expect("amount"))
    }

    pub fn account_lamports(&self, key: &Pubkey) -> u64 {
        self.svm.get_account(key).map(|a| a.lamports).unwrap_or(0)
    }

    pub fn write_job(&mut self, job: &Pubkey, state: &JobAccount) {
        let mut acc = self.svm.get_account(job).expect("job");
        acc.data = state.encode().to_vec();
        self.svm.set_account(*job, acc).expect("set job");
    }

    pub fn write_bond(&mut self, bond: &ProviderBondAccount) {
        let key = self.bond_pda();
        let mut acc = self.svm.get_account(&key).expect("bond");
        acc.data = bond.encode().expect("encode").to_vec();
        self.svm.set_account(key, acc).expect("set bond");
    }

    /// Write bond bytes even when invariants would reject `encode`.
    pub fn write_bond_raw(&mut self, mutate: impl FnOnce(&mut [u8])) {
        let key = self.bond_pda();
        let mut acc = self.svm.get_account(&key).expect("bond");
        mutate(&mut acc.data);
        self.svm.set_account(key, acc).expect("set bond");
    }

    pub fn write_provider(&mut self, provider: &ProviderAccount) {
        let key = self.provider_pda();
        let mut acc = self.svm.get_account(&key).expect("provider");
        acc.data = provider.encode().expect("encode").to_vec();
        self.svm.set_account(key, acc).expect("set provider");
    }

    pub fn set_provider_inactive(&mut self) {
        let mut provider = self.read_provider();
        provider.status = PROVIDER_STATUS_INACTIVE;
        self.write_provider(&provider);
    }

    pub fn bootstrap_ready(&mut self) {
        self.initialize_config();
        self.register_provider();
        self.add_execution_key();
        self.deposit_bond(MIN_BOND * 2);
    }

    pub fn assert_job_state(&self, job: &Pubkey, state: JobState) {
        assert_eq!(self.read_job(job).state, state);
    }

    pub fn assert_account_closed(&self, key: &Pubkey) {
        match self.svm.get_account(key) {
            None => {}
            Some(acc) => {
                assert_eq!(acc.lamports, 0, "lamports remaining for {key}");
                assert!(acc.data.is_empty() || acc.data.iter().all(|b| *b == 0));
            }
        }
    }

    /// SPL Token account `state` byte offset (Initialized=1, Frozen=2).
    pub fn force_frozen(&mut self, ata: &Pubkey) {
        let mut acc = self.svm.get_account(ata).expect("token");
        acc.data[108] = 2;
        self.svm.set_account(*ata, acc).expect("set token");
    }

    /// Inject a COption::Some delegate without going through Token Program CPI.
    pub fn force_delegate(&mut self, ata: &Pubkey, delegate: &Pubkey, amount: u64) {
        let mut acc = self.svm.get_account(ata).expect("token");
        acc.data[72..76].copy_from_slice(&1u32.to_le_bytes());
        acc.data[76..108].copy_from_slice(&delegate.to_bytes());
        acc.data[121..129].copy_from_slice(&amount.to_le_bytes());
        self.svm.set_account(*ata, acc).expect("set token");
    }
}
