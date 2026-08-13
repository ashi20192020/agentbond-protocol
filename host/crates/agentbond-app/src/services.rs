use agentbond_sdk::{
    ChainReader, InstructionPlan, build_submit_receipt_plan, decode_job, decode_provider,
    plan_accept_job, plan_accept_work, plan_challenge_work, plan_create_job,
    plan_expire_unaccepted, plan_expire_unfunded, plan_fund_job, plan_resolve_timeout_refund,
    plan_resolve_timeout_settle,
};
use agentbond_types::{CreateJobPayload, JobState};
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;

use crate::catalog::ServiceCatalog;
use crate::config::AppConfig;
use crate::error::AppError;
use crate::receipt_dto::ReceiptDto;

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
    Ok(plan_create_job(
        &program_id,
        &buyer,
        &provider,
        now,
        &payload,
    )?)
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

pub fn build_submit_receipt_plan_uc(
    cfg: &AppConfig,
    req: &SubmitReceiptRequest,
) -> Result<InstructionPlan, AppError> {
    let program_id = cfg.program_pubkey()?;
    let job = pk(&req.job, "job")?;
    let provider = pk(&req.provider, "provider")?;
    let pubkey = hex32(&req.execution_pubkey_hex, "execution_pubkey")?;
    let signature = hex64(&req.signature_hex, "signature")?;
    let receipt = req.receipt.to_receipt()?;
    Ok(build_submit_receipt_plan(
        &program_id,
        &job,
        &provider,
        &receipt,
        &pubkey,
        &signature,
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
    let buyer = pk(&req.buyer, "buyer")?;
    let provider = pk(&req.provider, "provider")?;
    let payer = pk(&req.payer, "payer")?;
    let job_addr = agentbond_sdk::job_pda(&program_id, &buyer, &provider, req.job_nonce)?.address;
    let job = inspect_job(reader, &program_id, &job_addr).await?;
    let now = reader.get_unix_timestamp().await?;

    match job.state {
        JobState::Submitted if now >= job.auto_settle_deadline => Ok(plan_resolve_timeout_settle(
            &program_id,
            &payer,
            &buyer,
            &provider,
            &mint,
            req.job_nonce,
            false,
        )?),
        JobState::Challenged => Ok(plan_resolve_timeout_settle(
            &program_id,
            &payer,
            &buyer,
            &provider,
            &mint,
            req.job_nonce,
            true,
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
        JobState::Created if now >= job.fund_deadline => Ok(plan_expire_unfunded(
            &program_id,
            &payer,
            &buyer,
            &provider,
            req.job_nonce,
        )?),
        JobState::Funded => Ok(plan_expire_unaccepted(
            &program_id,
            &payer,
            &buyer,
            &provider,
            &mint,
            req.job_nonce,
        )?),
        _ => Err(AppError::Validation(
            "job is not eligible for timeout resolution".into(),
        )),
    }
}
