#![allow(dead_code)]

use std::path::PathBuf;

use agentbond::{
    bond_address, challenge_address, config_address, job_address, provider_address, ID,
};
use agentbond_types::{
    encode_add_execution_key, encode_challenge_work, encode_create_job, encode_deposit_bond,
    encode_empty, encode_initialize_config, encode_set_paused, encode_submit_receipt,
    AgentBondWorkReceiptV1, CreateJobPayload, InitializeConfigPayload, InstructionKind, JobAccount,
    JobState, ProtocolError, ProviderBondAccount, RECEIPT_ENCODED_LEN,
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
        self.svm.send_transaction(tx)
    }

    pub fn send_ok(&mut self, payer: &Keypair, ixs: &[Instruction], signers: &[&Keypair]) {
        self.send(payer, ixs, signers)
            .expect("transaction should succeed");
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
                None,
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

    pub fn config_pda(&self) -> Pubkey {
        let (addr, _) = config_address(&ID).expect("config");
        pk_from_bytes(addr.to_bytes())
    }

    pub fn provider_pda(&self) -> Pubkey {
        let authority = address_bytes(&self.provider.pubkey());
        let (addr, _) = provider_address(&ID, &authority).expect("provider");
        pk_from_bytes(addr.to_bytes())
    }

    pub fn bond_pda(&self) -> Pubkey {
        let authority = address_bytes(&self.provider.pubkey());
        let mint = address_bytes(&self.mint);
        let (addr, _) = bond_address(&ID, &authority, &mint).expect("bond");
        pk_from_bytes(addr.to_bytes())
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

    pub fn initialize_config(&mut self) {
        let config = self.config_pda();
        let payload = InitializeConfigPayload {
            genesis_hash: GENESIS,
            allowed_mint: address_bytes(&self.mint),
            token_program: TOKEN_PROGRAM_ID.to_bytes(),
            mint_decimals: DECIMALS,
            min_provider_bond: MIN_BOND,
            challenge_duration_seconds: 3_600,
        };
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(self.admin.pubkey(), true),
                AccountMeta::new(config, false),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
            data: encode_initialize_config(&payload).to_vec(),
        };
        let payer = self.admin.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn set_paused(&mut self, paused: bool) {
        let config = self.config_pda();
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.admin.pubkey(), true),
                AccountMeta::new(config, false),
            ],
            data: encode_set_paused(paused).to_vec(),
        };
        let payer = self.admin.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn register_provider(&mut self) {
        let config = self.config_pda();
        let provider_pda = self.provider_pda();
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(self.provider.pubkey(), true),
                AccountMeta::new_readonly(config, false),
                AccountMeta::new(provider_pda, false),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
            data: encode_empty(InstructionKind::RegisterProvider)
                .expect("empty")
                .to_vec(),
        };
        let payer = self.provider.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn add_execution_key(&mut self) {
        let provider_pda = self.provider_pda();
        let key = self.exec.public.to_bytes();
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.provider.pubkey(), true),
                AccountMeta::new(provider_pda, false),
            ],
            data: encode_add_execution_key(&key).to_vec(),
        };
        let payer = self.provider.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn deposit_bond(&mut self, amount: u64) {
        let config = self.config_pda();
        let provider_pda = self.provider_pda();
        let bond_pda = self.bond_pda();
        let provider_ata = self.create_ata(&self.provider.pubkey());
        self.mint_to(&provider_ata, amount.saturating_mul(2).max(amount));
        let bond_vault = get_associated_token_address(&bond_pda, &self.mint);
        if self.svm.get_account(&bond_vault).is_none() {
            let create_vault = create_associated_token_account(
                &self.provider.pubkey(),
                &bond_pda,
                &self.mint,
                &TOKEN_PROGRAM_ID,
            );
            let payer = self.provider.insecure_clone();
            self.send_ok(&payer, &[create_vault], &[]);
        }
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(self.provider.pubkey(), true),
                AccountMeta::new_readonly(config, false),
                AccountMeta::new_readonly(provider_pda, false),
                AccountMeta::new(bond_pda, false),
                AccountMeta::new(bond_vault, false),
                AccountMeta::new(provider_ata, false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
            data: encode_deposit_bond(amount).to_vec(),
        };
        let payer = self.provider.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn create_job(&mut self, nonce: u64) -> Pubkey {
        let config = self.config_pda();
        let provider_pda = self.provider_pda();
        let job = self.job_pda(nonce);
        let payload = CreateJobPayload {
            job_nonce: nonce,
            amount: JOB_AMOUNT,
            request_hash: [9u8; 32],
            fund_deadline: START_TS + 100,
            accept_deadline: START_TS + 200,
            work_deadline: START_TS + 300,
            auto_settle_deadline: START_TS + 400,
        };
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(self.buyer.pubkey(), true),
                AccountMeta::new_readonly(config, false),
                AccountMeta::new_readonly(provider_pda, false),
                AccountMeta::new(job, false),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
            data: encode_create_job(&payload).to_vec(),
        };
        let payer = self.buyer.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
        job
    }

    pub fn fund_job(&mut self, job: &Pubkey) {
        let config = self.config_pda();
        let buyer_ata = self.create_ata(&self.buyer.pubkey());
        self.mint_to(&buyer_ata, JOB_AMOUNT * 2);
        let escrow = get_associated_token_address(job, &self.mint);
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
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.buyer.pubkey(), true),
                AccountMeta::new_readonly(config, false),
                AccountMeta::new(*job, false),
                AccountMeta::new(buyer_ata, false),
                AccountMeta::new(escrow, false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ],
            data: encode_empty(InstructionKind::FundJob)
                .expect("empty")
                .to_vec(),
        };
        let payer = self.buyer.insecure_clone();
        self.send_ok(&payer, &[ix], &[]);
    }

    pub fn accept_job(&mut self, job: &Pubkey) {
        let config = self.config_pda();
        let provider_pda = self.provider_pda();
        let bond_pda = self.bond_pda();
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.provider.pubkey(), true),
                AccountMeta::new_readonly(config, false),
                AccountMeta::new_readonly(provider_pda, false),
                AccountMeta::new(bond_pda, false),
                AccountMeta::new(*job, false),
            ],
            data: encode_empty(InstructionKind::AcceptJob)
                .expect("empty")
                .to_vec(),
        };
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
        let config = self.config_pda();
        let provider_pda = self.provider_pda();
        let encoded = receipt.encode().expect("encode");
        let ed_ix = Self::ed25519_ix(&encoded, keypair);
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(config, false),
                AccountMeta::new_readonly(provider_pda, false),
                AccountMeta::new(*job, false),
                AccountMeta::new_readonly(instructions_sysvar_id(), false),
            ],
            data: encode_submit_receipt(receipt).expect("submit").to_vec(),
        };
        vec![ed_ix, ix]
    }

    pub fn submit_receipt(&mut self, job: &Pubkey, receipt: &AgentBondWorkReceiptV1) -> u64 {
        let ixs = self.submit_receipt_ixs(job, receipt, &self.exec);
        let mut all = vec![budget_ix()];
        all.extend(ixs);
        let payer = self.provider.insecure_clone();
        let meta = self.send(&payer, &all, &[]).expect("submit receipt");
        meta.compute_units_consumed
    }

    pub fn accept_work(&mut self, job: &Pubkey) -> u64 {
        let bond_pda = self.bond_pda();
        let escrow = get_associated_token_address(job, &self.mint);
        let provider_ata = self.create_ata(&self.provider.pubkey());
        let buyer_ata = self.create_ata(&self.buyer.pubkey());
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.buyer.pubkey(), true),
                AccountMeta::new(*job, false),
                AccountMeta::new(bond_pda, false),
                AccountMeta::new(escrow, false),
                AccountMeta::new(provider_ata, false),
                AccountMeta::new(buyer_ata, false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ],
            data: encode_empty(InstructionKind::AcceptWork)
                .expect("empty")
                .to_vec(),
        };
        let payer = self.buyer.insecure_clone();
        let meta = self
            .send(&payer, &[budget_ix(), ix], &[])
            .expect("accept work");
        meta.compute_units_consumed
    }

    pub fn challenge_work(&mut self, job: &Pubkey) -> u64 {
        let config = self.config_pda();
        let challenge = self.challenge_pda(job);
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(self.buyer.pubkey(), true),
                AccountMeta::new_readonly(config, false),
                AccountMeta::new(*job, false),
                AccountMeta::new(challenge, false),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
            data: encode_challenge_work(&[8u8; 32]).to_vec(),
        };
        let payer = self.buyer.insecure_clone();
        let meta = self
            .send(&payer, &[budget_ix(), ix], &[])
            .expect("challenge");
        meta.compute_units_consumed
    }

    pub fn resolve_timeout_settle(&mut self, job: &Pubkey, with_challenge: bool) -> u64 {
        let bond_pda = self.bond_pda();
        let escrow = get_associated_token_address(job, &self.mint);
        let provider_ata = self.create_ata(&self.provider.pubkey());
        let buyer_ata = self.create_ata(&self.buyer.pubkey());
        let mut accounts = vec![
            AccountMeta::new_readonly(self.admin.pubkey(), true),
            AccountMeta::new(*job, false),
            AccountMeta::new(bond_pda, false),
            AccountMeta::new(escrow, false),
            AccountMeta::new(provider_ata, false),
            AccountMeta::new(buyer_ata, false),
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
        let payer = self.admin.insecure_clone();
        let meta = self
            .send(&payer, &[budget_ix(), ix], &[])
            .expect("timeout settle");
        meta.compute_units_consumed
    }

    pub fn resolve_timeout_refund(&mut self, job: &Pubkey) -> u64 {
        let bond_pda = self.bond_pda();
        let escrow = get_associated_token_address(job, &self.mint);
        let buyer_ata = self.create_ata(&self.buyer.pubkey());
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.admin.pubkey(), true),
                AccountMeta::new(*job, false),
                AccountMeta::new(bond_pda, false),
                AccountMeta::new(escrow, false),
                AccountMeta::new(buyer_ata, false),
                AccountMeta::new(self.buyer.pubkey(), false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ],
            data: encode_empty(InstructionKind::ResolveTimeoutRefund)
                .expect("empty")
                .to_vec(),
        };
        let payer = self.admin.insecure_clone();
        let meta = self
            .send(&payer, &[budget_ix(), ix], &[])
            .expect("timeout refund");
        meta.compute_units_consumed
    }

    pub fn slash_bond(&mut self, job: &Pubkey) -> u64 {
        let config = self.config_pda();
        let bond_pda = self.bond_pda();
        let bond_vault = get_associated_token_address(&bond_pda, &self.mint);
        let escrow = get_associated_token_address(job, &self.mint);
        let buyer_ata = self.create_ata(&self.buyer.pubkey());
        let challenge = self.challenge_pda(job);
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.admin.pubkey(), true),
                AccountMeta::new_readonly(config, false),
                AccountMeta::new(*job, false),
                AccountMeta::new(bond_pda, false),
                AccountMeta::new(bond_vault, false),
                AccountMeta::new(escrow, false),
                AccountMeta::new(buyer_ata, false),
                AccountMeta::new(self.buyer.pubkey(), false),
                AccountMeta::new(challenge, false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ],
            data: encode_empty(InstructionKind::SlashBond)
                .expect("empty")
                .to_vec(),
        };
        let payer = self.admin.insecure_clone();
        let meta = self.send(&payer, &[budget_ix(), ix], &[]).expect("slash");
        meta.compute_units_consumed
    }

    pub fn close_job(&mut self, job: &Pubkey) {
        let escrow = get_associated_token_address(job, &self.mint);
        let mut accounts = vec![
            AccountMeta::new_readonly(self.buyer.pubkey(), true),
            AccountMeta::new(*job, false),
            AccountMeta::new(self.buyer.pubkey(), false),
        ];
        if self.svm.get_account(&escrow).is_some() {
            accounts.push(AccountMeta::new(escrow, false));
            accounts.push(AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false));
        }
        let ix = Instruction {
            program_id: self.program_id,
            accounts,
            data: encode_empty(InstructionKind::CloseJob)
                .expect("empty")
                .to_vec(),
        };
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

    pub fn token_balance(&self, ata: &Pubkey) -> u64 {
        let acc = self.svm.get_account(ata).expect("ata");
        u64::from_le_bytes(acc.data[64..72].try_into().expect("amount"))
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
}

fn budget_ix() -> Instruction {
    ComputeBudgetInstruction::set_compute_unit_limit(1_400_000)
}

fn instructions_sysvar_id() -> Pubkey {
    Pubkey::from_str_const("Sysvar1nstructions1111111111111111111111111")
}

fn new_ed25519_instruction(message: &[u8], signature: &[u8; 64], pubkey: &[u8; 32]) -> Instruction {
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
    assert_eq!(message.len(), RECEIPT_ENCODED_LEN);
    Instruction {
        program_id: Pubkey::from_str_const("Ed25519SigVerify111111111111111111111111111"),
        accounts: vec![],
        data,
    }
}
