use agentbond_sdk::{
    ChainReader, InstructionPlan, build_submit_receipt_plan_at, challenge_pda, decode_challenge,
    decode_job, decode_provider, plan_accept_job, plan_accept_work, plan_challenge_work,
    plan_create_job, plan_expire_unfunded, plan_fund_job, plan_resolve_timeout_refund,
    plan_resolve_timeout_settle,
};
use agentbond_types::{CreateJobPayload, JobState};
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;

use crate::accounts::{ChallengeDto, ConfigDto, JobDto, ProviderBondDto, ProviderDto};
use crate::catalog::ServiceCatalog;
use crate::config::AppConfig;
use crate::error::AppError;
use crate::receipt_dto::ReceiptDto;
use agentbond_sdk::{config_pda, decode_config, decode_provider_bond};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateJobRequest {
    pub buyer: String,
    pub provider: String,
    pub job_nonce: u64,
    pub amount: u64,
    pub request_hash_hex: String,
    pub fund_deadline: i64,
    pub accept_deadline: i64,
    pub work_deadline: i64,
    pub auto_settle_deadline: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FundJobRequest {
    pub buyer: String,
    pub provider: String,
    pub job_nonce: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcceptJobRequest {
    pub buyer: String,
    pub provider: String,
    pub job_nonce: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubmitReceiptRequest {
    pub job: String,
    pub provider: String,
    pub receipt: ReceiptDto,
    pub execution_pubkey_hex: String,
    pub signature_hex: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcceptWorkRequest {
    pub buyer: String,
    pub provider: String,
    pub job_nonce: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChallengeRequest {
    pub buyer: String,
    pub provider: String,
    pub job_nonce: u64,
    pub reason_hash_hex: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimeoutRequest {
    pub payer: String,
    pub buyer: String,
    pub provider: String,
    pub job_nonce: u64,
}

fn pk(s: &str, label: &str) -> Result<Pubkey, AppError> {
    s.parse()
        .map_err(|_| AppError::Validation(format!("invalid {label}")))
}

fn hex32(s: &str, label: &str) -> Result<[u8; 32], AppError> {
    let bytes = hex_decode(s)?;
    if bytes.len() != 32 {
        return Err(AppError::Validation(format!("{label} must be 32 bytes")));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn hex64(s: &str, label: &str) -> Result<[u8; 64], AppError> {
    let bytes = hex_decode(s)?;
    if bytes.len() != 64 {
        return Err(AppError::Validation(format!("{label} must be 64 bytes")));
    }
    let mut out = [0u8; 64];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn hex_decode(s: &str) -> Result<Vec<u8>, AppError> {
    let s = s.trim().strip_prefix("0x").unwrap_or(s.trim());
    if !s.len().is_multiple_of(2) {
        return Err(AppError::Validation("invalid hex".into()));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|_| AppError::Validation("invalid hex".into()))
        })
        .collect()
}

pub fn list_services(catalog: &ServiceCatalog) -> Vec<crate::catalog::ServiceEntry> {
    catalog.list().to_vec()
}

pub fn get_service<'a>(
    catalog: &'a ServiceCatalog,
    service_id: &str,
) -> Result<&'a crate::catalog::ServiceEntry, AppError> {
    catalog.get(service_id)
}

pub async fn inspect_config_dto(
    reader: &dyn ChainReader,
    program_id: &Pubkey,
) -> Result<ConfigDto, AppError> {
    let addr = config_pda(program_id)?.address;
    let acc = reader
        .get_account(&addr)
        .await?
        .ok_or_else(|| AppError::NotFound("config".into()))?;
    let cfg = decode_config(program_id, &addr, &acc.owner, &acc.data)?;
    Ok(ConfigDto::from_account(&cfg))
}

pub async fn inspect_provider_dto(
    reader: &dyn ChainReader,
    program_id: &Pubkey,
    address: &Pubkey,
) -> Result<ProviderDto, AppError> {
    let acc = reader
        .get_account(address)
        .await?
        .ok_or_else(|| AppError::NotFound("provider".into()))?;
    let account = decode_provider(program_id, address, &acc.owner, &acc.data)?;
    Ok(ProviderDto::from_account(&account))
}

pub async fn inspect_bond_dto(
    reader: &dyn ChainReader,
    program_id: &Pubkey,
    address: &Pubkey,
) -> Result<ProviderBondDto, AppError> {
    let acc = reader
        .get_account(address)
        .await?
        .ok_or_else(|| AppError::NotFound("bond".into()))?;
    let account = decode_provider_bond(program_id, address, &acc.owner, &acc.data)?;
    Ok(ProviderBondDto::from_account(&account))
}

pub async fn inspect_job_dto(
    reader: &dyn ChainReader,
    program_id: &Pubkey,
    address: &Pubkey,
) -> Result<JobDto, AppError> {
    let acc = reader
        .get_account(address)
        .await?
        .ok_or_else(|| AppError::NotFound("job".into()))?;
    let account = decode_job(program_id, address, &acc.owner, &acc.data)?;
    Ok(JobDto::from_account(&account))
}

pub async fn inspect_challenge_dto(
    reader: &dyn ChainReader,
    program_id: &Pubkey,
    address: &Pubkey,
) -> Result<ChallengeDto, AppError> {
    let acc = reader
        .get_account(address)
        .await?
        .ok_or_else(|| AppError::NotFound("challenge".into()))?;
    let account = decode_challenge(program_id, address, &acc.owner, &acc.data)?;
    Ok(ChallengeDto::from_account(&account))
}

pub async fn inspect_provider(
    reader: &dyn ChainReader,
    program_id: &Pubkey,
    address: &Pubkey,
) -> Result<agentbond_types::ProviderAccount, AppError> {
    let acc = reader
        .get_account(address)
        .await?
        .ok_or_else(|| AppError::NotFound("provider".into()))?;
    Ok(decode_provider(program_id, address, &acc.owner, &acc.data)?)
}

pub async fn inspect_job(
    reader: &dyn ChainReader,
    program_id: &Pubkey,
    address: &Pubkey,
) -> Result<agentbond_types::JobAccount, AppError> {
    let acc = reader
        .get_account(address)
        .await?
        .ok_or_else(|| AppError::NotFound("job".into()))?;
    Ok(decode_job(program_id, address, &acc.owner, &acc.data)?)
}

pub async fn build_create_job_plan(
    cfg: &AppConfig,
    reader: &dyn ChainReader,
    req: &CreateJobRequest,
) -> Result<InstructionPlan, AppError> {
    let program_id = cfg.program_pubkey()?;
    let buyer = pk(&req.buyer, "buyer")?;
    let provider = pk(&req.provider, "provider")?;
    let request_hash = hex32(&req.request_hash_hex, "request_hash")?;
    let now = reader.get_unix_timestamp().await?;
    let payload = CreateJobPayload {
        job_nonce: req.job_nonce,
        amount: req.amount,
        request_hash,
        fund_deadline: req.fund_deadline,
        accept_deadline: req.accept_deadline,
        work_deadline: req.work_deadline,
        auto_settle_deadline: req.auto_settle_deadline,
    };
    let mint = cfg.mint_pubkey()?;
    Ok(
        plan_create_job(&program_id, &buyer, &provider, now, &payload)?
            .with_mint_amount(&mint, req.amount),
    )
}

pub fn build_fund_job_plan(
    cfg: &AppConfig,
    req: &FundJobRequest,
) -> Result<InstructionPlan, AppError> {
    let program_id = cfg.program_pubkey()?;
    let mint = cfg.mint_pubkey()?;
    Ok(plan_fund_job(
        &program_id,
        &pk(&req.buyer, "buyer")?,
        &pk(&req.provider, "provider")?,
        &mint,
        req.job_nonce,
    )?)
}

pub fn build_accept_job_plan(
    cfg: &AppConfig,
    req: &AcceptJobRequest,
) -> Result<InstructionPlan, AppError> {
    let program_id = cfg.program_pubkey()?;
    let mint = cfg.mint_pubkey()?;
    Ok(plan_accept_job(
        &program_id,
        &pk(&req.provider, "provider")?,
        &pk(&req.buyer, "buyer")?,
        &mint,
        req.job_nonce,
    )?)
}

pub async fn build_submit_receipt_plan_uc(
    cfg: &AppConfig,
    reader: &dyn ChainReader,
    req: &SubmitReceiptRequest,
) -> Result<InstructionPlan, AppError> {
    let program_id = cfg.program_pubkey()?;
    let job = pk(&req.job, "job")?;
    let provider = pk(&req.provider, "provider")?;
    let pubkey = hex32(&req.execution_pubkey_hex, "execution_pubkey")?;
    let signature = hex64(&req.signature_hex, "signature")?;
    let receipt = req.receipt.to_receipt()?;
    let now = reader.get_unix_timestamp().await?;
    Ok(build_submit_receipt_plan_at(
        &program_id,
        &job,
        &provider,
        &receipt,
        &pubkey,
        &signature,
        Some(now),
    )?)
}

pub fn build_accept_work_plan(
    cfg: &AppConfig,
    req: &AcceptWorkRequest,
) -> Result<InstructionPlan, AppError> {
    let program_id = cfg.program_pubkey()?;
    let mint = cfg.mint_pubkey()?;
    Ok(plan_accept_work(
        &program_id,
        &pk(&req.buyer, "buyer")?,
        &pk(&req.provider, "provider")?,
        &mint,
        req.job_nonce,
    )?)
}

pub fn build_challenge_plan(
    cfg: &AppConfig,
    req: &ChallengeRequest,
) -> Result<InstructionPlan, AppError> {
    let program_id = cfg.program_pubkey()?;
    let reason = hex32(&req.reason_hash_hex, "reason_hash")?;
    Ok(plan_challenge_work(
        &program_id,
        &pk(&req.buyer, "buyer")?,
        &pk(&req.provider, "provider")?,
        req.job_nonce,
        &reason,
    )?)
}

pub async fn build_timeout_plan(
    cfg: &AppConfig,
    reader: &dyn ChainReader,
    req: &TimeoutRequest,
) -> Result<InstructionPlan, AppError> {
    let program_id = cfg.program_pubkey()?;
    let mint = cfg.mint_pubkey()?;
    let token_program = pk(&cfg.token_program, "token_program")?;
    let buyer = pk(&req.buyer, "buyer")?;
    let provider = pk(&req.provider, "provider")?;
    let payer = pk(&req.payer, "payer")?;
    let job_addr = agentbond_sdk::job_pda(&program_id, &buyer, &provider, req.job_nonce)?.address;
    let job = inspect_job(reader, &program_id, &job_addr).await?;
    let now = reader.get_unix_timestamp().await?;

    if job.buyer != buyer.to_bytes()
        || job.provider != provider.to_bytes()
        || job.job_nonce != req.job_nonce
        || job.mint != mint.to_bytes()
        || job.token_program != token_program.to_bytes()
    {
        return Err(AppError::Validation(
            "fetched job does not match request and configured protocol".into(),
        ));
    }

    if job.state.is_terminal() {
        return Err(AppError::Validation(
            "terminal job is never eligible for timeout resolution".into(),
        ));
    }

    match job.state {
        JobState::Created if now >= job.fund_deadline => Ok(plan_expire_unfunded(
            &program_id,
            &payer,
            &buyer,
            &provider,
            req.job_nonce,
        )?),
        JobState::Funded if now >= job.accept_deadline => Ok(plan_resolve_timeout_refund(
            &program_id,
            &payer,
            &buyer,
            &provider,
            &mint,
            req.job_nonce,
        )?),
        JobState::Accepted if now > job.work_deadline => Ok(plan_resolve_timeout_refund(
            &program_id,
            &payer,
            &buyer,
            &provider,
            &mint,
            req.job_nonce,
        )?),
        JobState::Submitted if now >= job.auto_settle_deadline => Ok(plan_resolve_timeout_settle(
            &program_id,
            &payer,
            &buyer,
            &provider,
            &mint,
            req.job_nonce,
            false,
        )?),
        JobState::Challenged => {
            let challenge_addr = challenge_pda(&program_id, &job_addr)?.address;
            let acc = reader
                .get_account(&challenge_addr)
                .await?
                .ok_or_else(|| AppError::NotFound("challenge".into()))?;
            let challenge = decode_challenge(&program_id, &challenge_addr, &acc.owner, &acc.data)?;
            if challenge.job != job_addr.to_bytes() {
                return Err(AppError::Validation(
                    "challenge does not belong to job".into(),
                ));
            }
            if now < challenge.deadline {
                return Err(AppError::Validation(
                    "challenge deadline not reached".into(),
                ));
            }
            Ok(plan_resolve_timeout_settle(
                &program_id,
                &payer,
                &buyer,
                &provider,
                &mint,
                req.job_nonce,
                true,
            )?)
        }
        _ => Err(AppError::Validation(
            "job is not eligible for timeout resolution".into(),
        )),
    }
}
