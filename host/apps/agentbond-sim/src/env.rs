//! LiteSVM helpers for the local AgentBond simulator.
//! Requires `cargo build-sbf` to have produced `target/deploy/agentbond.so`.

use std::path::PathBuf;

use agentbond_sdk::{
    PROGRAM_ID_BYTES, bond_vault_ata, challenge_pda, config_pda, job_escrow_ata, job_pda,
    program_id, provider_bond_pda, provider_pda, user_settlement_ata,
};
use agentbond_types::{
    AgentBondWorkReceiptV1, CreateJobPayload, InitializeConfigPayload, InstructionKind, JobAccount,
    JobState, ProtocolError, ProviderBondAccount, encode_add_execution_key, encode_challenge_work,
    encode_create_job, encode_deposit_bond, encode_empty, encode_initialize_config,
    encode_submit_receipt,
};
use anyhow::{Context, Result, anyhow, bail};
use ed25519_dalek::{Keypair as DalekKeypair, PublicKey, SecretKey, Signer as DalekSigner};
use litesvm::LiteSVM;
use litesvm::types::FailedTransactionMetadata;
use solana_clock::Clock;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_system_interface::instruction as system_instruction;
use solana_transaction::Transaction;
use spl_associated_token_account_client::instruction::create_associated_token_account;
use spl_token::ID as TOKEN_PROGRAM_ID;
use spl_token::instruction as token_instruction;

pub const DECIMALS: u8 = 6;
pub const MIN_BOND: u64 = 1_000;
pub const JOB_AMOUNT: u64 = 5_000;
pub const GENESIS: [u8; 32] = [7u8; 32];
pub const START_TS: i64 = 1_700_000_000;
pub const CHALLENGE_SECS: i64 = 3_600;

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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/deploy/agentbond.so")
}

pub fn require_program_so() -> Result<PathBuf> {
    let path = program_so_path();
    if !path.is_file() {
        bail!(
            "missing SBF program at {}\nrun `cargo build-sbf` from the repo root first",
            path.display()
        );
    }
    Ok(path)
}

fn address_bytes(pk: &Pubkey) -> [u8; 32] {
    pk.to_bytes()
}

fn instructions_sysvar_id() -> Pubkey {
    Pubkey::from_str_const("Sysvar1nstructions1111111111111111111111111")
}

fn ed25519_program_id() -> Pubkey {
    Pubkey::from_str_const("Ed25519SigVerify111111111111111111111111111")
}

fn budget_ix() -> Instruction {
    ComputeBudgetInstruction::set_compute_unit_limit(1_400_000)
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

pub fn setup() -> Result<Env> {
    let so = require_program_so()?;
    let program_id = program_id();
    debug_assert_eq!(program_id.to_bytes(), PROGRAM_ID_BYTES);

    let mut svm = LiteSVM::new();
    svm.add_program_from_file(program_id, &so)
        .map_err(|e| anyhow!("load agentbond.so failed: {e:?}"))?;

    let admin = Keypair::new();
    let buyer = Keypair::new();
    let provider = Keypair::new();
    let mint_authority = Keypair::new();
    let mint = Keypair::new();

    for kp in [&admin, &buyer, &provider, &mint_authority] {
        svm.airdrop(&kp.pubkey(), 100_000_000_000)
            .map_err(|e| anyhow!("airdrop failed: {e:?}"))?;
    }

    let secret = SecretKey::from_bytes(&[42u8; 32]).map_err(|e| anyhow!("exec secret: {e}"))?;
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
    env.create_mint(&mint)?;
    Ok(env)
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
        self.svm.expire_blockhash();
        result
    }

    pub fn send_ok(
        &mut self,
        payer: &Keypair,
        ixs: &[Instruction],
        signers: &[&Keypair],
    ) -> Result<()> {
        self.send(payer, ixs, signers)
            .map_err(|e| anyhow!("transaction failed: {:?}", e.err))?;
        Ok(())
    }

    pub fn send_err_code(
        &mut self,
        payer: &Keypair,
        ixs: &[Instruction],
        signers: &[&Keypair],
        code: ProtocolError,
    ) -> Result<()> {
        let err = self
            .send(payer, ixs, signers)
            .err()
            .ok_or_else(|| anyhow!("expected failure, transaction succeeded"))?;
        let text = format!("{:?}", err.err);
        let expected = format!("Custom({})", code.code());
        if !text.contains(&expected) {
            bail!("expected {expected}, got {text} ({err:?})");
        }
        Ok(())
    }

    pub fn create_mint(&mut self, mint: &Keypair) -> Result<()> {
        let rent = self.svm.minimum_balance_for_rent_exemption(82);
        let init = token_instruction::initialize_mint(
            &TOKEN_PROGRAM_ID,
            &mint.pubkey(),
            &self.mint_authority.pubkey(),
            Some(&self.mint_authority.pubkey()),
            DECIMALS,
        )
        .map_err(|e| anyhow!("initialize_mint: {e}"))?;
        let ixs = [
            system_instruction::create_account(
                &self.admin.pubkey(),
                &mint.pubkey(),
                rent,
                82,
                &TOKEN_PROGRAM_ID,
            ),
            init,
        ];
        let payer = self.admin.insecure_clone();
        self.send_ok(&payer, &ixs, &[mint])?;
        self.mint = mint.pubkey();
        Ok(())
    }

    pub fn create_ata(&mut self, owner: &Pubkey) -> Result<Pubkey> {
        let ata = user_settlement_ata(owner, &self.mint);
        if self.svm.get_account(&ata).is_some() {
            return Ok(ata);
        }
        let ix = create_associated_token_account(
            &self.admin.pubkey(),
            owner,
            &self.mint,
            &TOKEN_PROGRAM_ID,
        );
        let payer = self.admin.insecure_clone();
        self.send_ok(&payer, &[ix], &[])?;
        Ok(ata)
    }

    pub fn mint_to(&mut self, ata: &Pubkey, amount: u64) -> Result<()> {
        let ix = token_instruction::mint_to(
            &TOKEN_PROGRAM_ID,
            &self.mint,
            ata,
            &self.mint_authority.pubkey(),
            &[],
            amount,
        )
        .map_err(|e| anyhow!("mint_to: {e}"))?;
        let payer = self.mint_authority.insecure_clone();
        self.send_ok(&payer, &[ix], &[])
    }

    pub fn config_pda(&self) -> Result<Pubkey> {
        Ok(config_pda(&self.program_id)?.address)
    }

    pub fn provider_pda(&self) -> Result<Pubkey> {
        Ok(provider_pda(&self.program_id, &self.provider.pubkey())?.address)
    }

    pub fn bond_pda(&self) -> Result<Pubkey> {
        Ok(provider_bond_pda(&self.program_id, &self.provider.pubkey(), &self.mint)?.address)
    }

    pub fn bond_vault(&self) -> Result<Pubkey> {
        Ok(bond_vault_ata(&self.bond_pda()?, &self.mint))
    }

    pub fn job_pda(&self, nonce: u64) -> Result<Pubkey> {
        Ok(job_pda(
            &self.program_id,
            &self.buyer.pubkey(),
            &self.provider.pubkey(),
            nonce,
        )?
        .address)
    }

    pub fn challenge_pda(&self, job: &Pubkey) -> Result<Pubkey> {
        Ok(challenge_pda(&self.program_id, job)?.address)
    }

    pub fn escrow_ata(&self, job: &Pubkey) -> Pubkey {
        job_escrow_ata(job, &self.mint)
    }

    pub fn ensure_escrow(&mut self, job: &Pubkey) -> Result<Pubkey> {
        let escrow = self.escrow_ata(job);
        if self.svm.get_account(&escrow).is_none() {
            let create_escrow = create_associated_token_account(
                &self.buyer.pubkey(),
                job,
                &self.mint,
                &TOKEN_PROGRAM_ID,
            );
            let payer = self.buyer.insecure_clone();
            self.send_ok(&payer, &[create_escrow], &[])?;
        }
        Ok(escrow)
    }

    pub fn ensure_bond_vault(&mut self) -> Result<Pubkey> {
        let bond_pda = self.bond_pda()?;
        let vault = self.bond_vault()?;
        if self.svm.get_account(&vault).is_none() {
            let create_vault = create_associated_token_account(
                &self.provider.pubkey(),
                &bond_pda,
                &self.mint,
                &TOKEN_PROGRAM_ID,
            );
            let payer = self.provider.insecure_clone();
            self.send_ok(&payer, &[create_vault], &[])?;
        }
        Ok(vault)
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

    pub fn initialize_config(&mut self) -> Result<()> {
        let payload = self.default_config_payload();
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(self.admin.pubkey(), true),
                AccountMeta::new(self.config_pda()?, false),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
            data: encode_initialize_config(&payload).to_vec(),
        };
        let payer = self.admin.insecure_clone();
        self.send_ok(&payer, &[ix], &[])
    }

    pub fn register_provider(&mut self) -> Result<()> {
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(self.provider.pubkey(), true),
                AccountMeta::new_readonly(self.config_pda()?, false),
                AccountMeta::new(self.provider_pda()?, false),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
            data: encode_empty(InstructionKind::RegisterProvider)
                .context("encode RegisterProvider")?
                .to_vec(),
        };
        let payer = self.provider.insecure_clone();
        self.send_ok(&payer, &[ix], &[])
    }

    pub fn add_execution_key(&mut self) -> Result<()> {
        let key = self.exec.public.to_bytes();
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.provider.pubkey(), true),
                AccountMeta::new(self.provider_pda()?, false),
            ],
            data: encode_add_execution_key(&key).to_vec(),
        };
        let payer = self.provider.insecure_clone();
        self.send_ok(&payer, &[ix], &[])
    }

    pub fn deposit_bond(&mut self, amount: u64) -> Result<()> {
        let provider_ata = self.create_ata(&self.provider.pubkey())?;
        self.mint_to(&provider_ata, amount.saturating_mul(2).max(amount))?;
        let vault = self.ensure_bond_vault()?;
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(self.provider.pubkey(), true),
                AccountMeta::new_readonly(self.config_pda()?, false),
                AccountMeta::new_readonly(self.provider_pda()?, false),
                AccountMeta::new(self.bond_pda()?, false),
                AccountMeta::new(vault, false),
                AccountMeta::new(provider_ata, false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
            data: encode_deposit_bond(amount).to_vec(),
        };
        let payer = self.provider.insecure_clone();
        self.send_ok(&payer, &[ix], &[])
    }

    pub fn bootstrap_ready(&mut self) -> Result<()> {
        self.initialize_config()?;
        self.register_provider()?;
        self.add_execution_key()?;
        self.deposit_bond(MIN_BOND * 2)?;
        Ok(())
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

    pub fn create_job(&mut self, nonce: u64) -> Result<Pubkey> {
        let payload = self.create_job_payload(nonce);
        let job = self.job_pda(nonce)?;
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(self.buyer.pubkey(), true),
                AccountMeta::new_readonly(self.config_pda()?, false),
                AccountMeta::new_readonly(self.provider_pda()?, false),
                AccountMeta::new(job, false),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
            data: encode_create_job(&payload).to_vec(),
        };
        let payer = self.buyer.insecure_clone();
        self.send_ok(&payer, &[ix], &[])?;
        Ok(job)
    }

    pub fn fund_job(&mut self, job: &Pubkey) -> Result<()> {
        let buyer_ata = self.create_ata(&self.buyer.pubkey())?;
        self.mint_to(&buyer_ata, JOB_AMOUNT * 2)?;
        let escrow = self.ensure_escrow(job)?;
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.buyer.pubkey(), true),
                AccountMeta::new_readonly(self.config_pda()?, false),
                AccountMeta::new(*job, false),
                AccountMeta::new(buyer_ata, false),
                AccountMeta::new(escrow, false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ],
            data: encode_empty(InstructionKind::FundJob)
                .context("encode FundJob")?
                .to_vec(),
        };
        let payer = self.buyer.insecure_clone();
        self.send_ok(&payer, &[ix], &[])
    }

    pub fn accept_job(&mut self, job: &Pubkey) -> Result<()> {
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.provider.pubkey(), true),
                AccountMeta::new_readonly(self.config_pda()?, false),
                AccountMeta::new_readonly(self.provider_pda()?, false),
                AccountMeta::new(self.bond_pda()?, false),
                AccountMeta::new(*job, false),
            ],
            data: encode_empty(InstructionKind::AcceptJob)
                .context("encode AcceptJob")?
                .to_vec(),
        };
        let payer = self.provider.insecure_clone();
        self.send_ok(&payer, &[ix], &[])
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
    ) -> Result<Vec<Instruction>> {
        let encoded = receipt.encode().context("encode receipt")?;
        let ed_ix = Self::ed25519_ix(&encoded, keypair);
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.config_pda()?, false),
                AccountMeta::new_readonly(self.provider_pda()?, false),
                AccountMeta::new(*job, false),
                AccountMeta::new_readonly(instructions_sysvar_id(), false),
            ],
            data: encode_submit_receipt(receipt)
                .context("encode submit receipt")?
                .to_vec(),
        };
        Ok(vec![ed_ix, ix])
    }

    pub fn submit_receipt(&mut self, job: &Pubkey, receipt: &AgentBondWorkReceiptV1) -> Result<()> {
        let ixs = self.submit_receipt_ixs(job, receipt, &self.exec)?;
        let mut all = vec![budget_ix()];
        all.extend_from_slice(&ixs);
        let payer = self.provider.insecure_clone();
        self.send_ok(&payer, &all, &[])
    }

    pub fn accept_work(&mut self, job: &Pubkey) -> Result<()> {
        let _ = self.create_ata(&self.provider.pubkey())?;
        let _ = self.create_ata(&self.buyer.pubkey())?;
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.buyer.pubkey(), true),
                AccountMeta::new(*job, false),
                AccountMeta::new(self.bond_pda()?, false),
                AccountMeta::new(self.escrow_ata(job), false),
                AccountMeta::new(
                    user_settlement_ata(&self.provider.pubkey(), &self.mint),
                    false,
                ),
                AccountMeta::new(user_settlement_ata(&self.buyer.pubkey(), &self.mint), false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ],
            data: encode_empty(InstructionKind::AcceptWork)
                .context("encode AcceptWork")?
                .to_vec(),
        };
        let payer = self.buyer.insecure_clone();
        self.send_ok(&payer, &[budget_ix(), ix], &[])
    }

    pub fn challenge_work(&mut self, job: &Pubkey) -> Result<()> {
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new(self.buyer.pubkey(), true),
                AccountMeta::new_readonly(self.config_pda()?, false),
                AccountMeta::new(*job, false),
                AccountMeta::new(self.challenge_pda(job)?, false),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
            data: encode_challenge_work(&[8u8; 32]).to_vec(),
        };
        let payer = self.buyer.insecure_clone();
        self.send_ok(&payer, &[budget_ix(), ix], &[])
    }

    pub fn resolve_timeout_settle(&mut self, job: &Pubkey, with_challenge: bool) -> Result<()> {
        let _ = self.create_ata(&self.provider.pubkey())?;
        let _ = self.create_ata(&self.buyer.pubkey())?;
        let mut accounts = vec![
            AccountMeta::new_readonly(self.admin.pubkey(), true),
            AccountMeta::new(*job, false),
            AccountMeta::new(self.bond_pda()?, false),
            AccountMeta::new(self.escrow_ata(job), false),
            AccountMeta::new(
                user_settlement_ata(&self.provider.pubkey(), &self.mint),
                false,
            ),
            AccountMeta::new(user_settlement_ata(&self.buyer.pubkey(), &self.mint), false),
            AccountMeta::new(self.buyer.pubkey(), false),
            AccountMeta::new_readonly(self.mint, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ];
        if with_challenge {
            accounts.push(AccountMeta::new(self.challenge_pda(job)?, false));
        }
        let ix = Instruction {
            program_id: self.program_id,
            accounts,
            data: encode_empty(InstructionKind::ResolveTimeoutSettle)
                .context("encode ResolveTimeoutSettle")?
                .to_vec(),
        };
        let payer = self.admin.insecure_clone();
        self.send_ok(&payer, &[budget_ix(), ix], &[])
    }

    pub fn resolve_timeout_refund(&mut self, job: &Pubkey) -> Result<()> {
        let _ = self.create_ata(&self.buyer.pubkey())?;
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.admin.pubkey(), true),
                AccountMeta::new(*job, false),
                AccountMeta::new(self.bond_pda()?, false),
                AccountMeta::new(self.escrow_ata(job), false),
                AccountMeta::new(user_settlement_ata(&self.buyer.pubkey(), &self.mint), false),
                AccountMeta::new(self.buyer.pubkey(), false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ],
            data: encode_empty(InstructionKind::ResolveTimeoutRefund)
                .context("encode ResolveTimeoutRefund")?
                .to_vec(),
        };
        let payer = self.admin.insecure_clone();
        self.send_ok(&payer, &[budget_ix(), ix], &[])
    }

    pub fn slash_bond(&mut self, job: &Pubkey) -> Result<()> {
        let _ = self.create_ata(&self.buyer.pubkey())?;
        let ix = Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.admin.pubkey(), true),
                AccountMeta::new_readonly(self.config_pda()?, false),
                AccountMeta::new(*job, false),
                AccountMeta::new(self.bond_pda()?, false),
                AccountMeta::new(self.bond_vault()?, false),
                AccountMeta::new(self.escrow_ata(job), false),
                AccountMeta::new(user_settlement_ata(&self.buyer.pubkey(), &self.mint), false),
                AccountMeta::new(self.buyer.pubkey(), false),
                AccountMeta::new(self.challenge_pda(job)?, false),
                AccountMeta::new_readonly(self.mint, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            ],
            data: encode_empty(InstructionKind::SlashBond)
                .context("encode SlashBond")?
                .to_vec(),
        };
        let payer = self.admin.insecure_clone();
        self.send_ok(&payer, &[budget_ix(), ix], &[])
    }

    pub fn read_job(&self, job: &Pubkey) -> Result<JobAccount> {
        let acc = self
            .svm
            .get_account(job)
            .ok_or_else(|| anyhow!("job account missing"))?;
        JobAccount::decode(&acc.data).map_err(|e| anyhow!("decode job: {e:?}"))
    }

    pub fn read_bond(&self) -> Result<ProviderBondAccount> {
        let bond = self.bond_pda()?;
        let acc = self
            .svm
            .get_account(&bond)
            .ok_or_else(|| anyhow!("bond account missing"))?;
        ProviderBondAccount::decode(&acc.data).map_err(|e| anyhow!("decode bond: {e:?}"))
    }

    pub fn token_balance(&self, ata: &Pubkey) -> Result<u64> {
        let acc = self
            .svm
            .get_account(ata)
            .ok_or_else(|| anyhow!("ata missing"))?;
        let bytes: [u8; 8] = acc.data[64..72]
            .try_into()
            .map_err(|_| anyhow!("token amount slice"))?;
        Ok(u64::from_le_bytes(bytes))
    }

    pub fn job_state(&self, job: &Pubkey) -> Result<JobState> {
        Ok(self.read_job(job)?.state)
    }

    pub fn assert_job_state(&self, job: &Pubkey, expected: JobState) -> Result<()> {
        let got = self.job_state(job)?;
        if got != expected {
            bail!("job state: expected {expected:?}, got {got:?}");
        }
        Ok(())
    }

    pub fn balances_line(&self) -> Result<String> {
        let buyer_ata = user_settlement_ata(&self.buyer.pubkey(), &self.mint);
        let provider_ata = user_settlement_ata(&self.provider.pubkey(), &self.mint);
        let buyer = if self.svm.get_account(&buyer_ata).is_some() {
            self.token_balance(&buyer_ata)?
        } else {
            0
        };
        let provider = if self.svm.get_account(&provider_ata).is_some() {
            self.token_balance(&provider_ata)?
        } else {
            0
        };
        let bond = self.read_bond()?;
        let unlocked = bond
            .unlocked()
            .map_err(|e| anyhow!("bond unlocked: {e:?}"))?;
        Ok(format!(
            "buyer={buyer} provider={provider} bond_unlocked={unlocked} bond_locked={}",
            bond.locked
        ))
    }
}
