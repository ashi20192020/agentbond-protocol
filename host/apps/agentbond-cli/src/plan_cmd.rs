use agentbond_app::ReceiptDto;
use agentbond_sdk::{
    InstructionPlan, build_submit_receipt_plan, parse_pubkey, plan_accept_job, plan_accept_work,
    plan_add_execution_key, plan_challenge_work, plan_close_job, plan_create_job,
    plan_deposit_bond, plan_expire_unaccepted, plan_expire_unfunded, plan_fund_job,
    plan_initialize_config, plan_register_provider, plan_resolve_timeout_refund,
    plan_resolve_timeout_settle, plan_revoke_execution_key, plan_set_paused, plan_slash_bond,
    plan_withdraw_bond,
};
use agentbond_types::{CreateJobPayload, InitializeConfigPayload};
use anyhow::{Context, Result, bail};
use clap::Subcommand;
use solana_pubkey::Pubkey;
use spl_token::ID as TOKEN_PROGRAM_ID;

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum PlanCmd {
    InitializeConfig {
        #[arg(long)]
        admin: String,
        #[arg(long)]
        genesis_hash: String,
        #[arg(long)]
        mint: String,
        #[arg(long)]
        mint_decimals: u8,
        #[arg(long)]
        min_provider_bond: u64,
        #[arg(long)]
        challenge_duration_seconds: i64,
    },
    SetPaused {
        #[arg(long)]
        admin: String,
        #[arg(long)]
        paused: bool,
    },
    RegisterProvider {
        #[arg(long)]
        authority: String,
    },
    AddExecutionKey {
        #[arg(long)]
        authority: String,
        #[arg(long)]
        key_hex: String,
    },
    RevokeExecutionKey {
        #[arg(long)]
        authority: String,
        #[arg(long)]
        key_hex: String,
    },
    DepositBond {
        #[arg(long)]
        authority: String,
        #[arg(long)]
        mint: String,
        #[arg(long)]
        amount: u64,
    },
    WithdrawBond {
        #[arg(long)]
        authority: String,
        #[arg(long)]
        mint: String,
        #[arg(long)]
        amount: u64,
    },
    CreateJob {
        #[arg(long)]
        buyer: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        nonce: u64,
        #[arg(long)]
        amount: u64,
        #[arg(long)]
        request_hash: String,
        #[arg(long)]
        fund_deadline: i64,
        #[arg(long)]
        accept_deadline: i64,
        #[arg(long)]
        work_deadline: i64,
        #[arg(long)]
        auto_settle_deadline: i64,
        #[arg(long)]
        now: i64,
    },
    FundJob {
        #[arg(long)]
        buyer: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        mint: String,
        #[arg(long)]
        nonce: u64,
    },
    AcceptJob {
        #[arg(long)]
        buyer: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        mint: String,
        #[arg(long)]
        nonce: u64,
    },
    SubmitReceipt {
        #[arg(long)]
        job: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        receipt_file: std::path::PathBuf,
        #[arg(long)]
        execution_pubkey_hex: String,
        #[arg(long)]
        signature_hex: String,
    },
    AcceptWork {
        #[arg(long)]
        buyer: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        mint: String,
        #[arg(long)]
        nonce: u64,
    },
    ChallengeWork {
        #[arg(long)]
        buyer: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        nonce: u64,
        #[arg(long)]
        reason_hash: String,
    },
    ResolveTimeout {
        #[arg(long)]
        payer: String,
        #[arg(long)]
        buyer: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        mint: String,
        #[arg(long)]
        nonce: u64,
        #[arg(long)]
        mode: String,
    },
    ExpireUnfunded {
        #[arg(long)]
        payer: String,
        #[arg(long)]
        buyer: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        nonce: u64,
    },
    ExpireUnaccepted {
        #[arg(long)]
        payer: String,
        #[arg(long)]
        buyer: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        mint: String,
        #[arg(long)]
        nonce: u64,
    },
    SlashBond {
        #[arg(long)]
        admin: String,
        #[arg(long)]
        buyer: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        mint: String,
        #[arg(long)]
        nonce: u64,
    },
    CloseJob {
        #[arg(long)]
        buyer: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        mint: String,
        #[arg(long)]
        nonce: u64,
        #[arg(long, default_value_t = true)]
        include_escrow: bool,
    },
}

pub fn run_plan(program: &Pubkey, cmd: PlanCmd, json: bool) -> Result<()> {
    let plan = build_plan(program, cmd)?;
    print_plan(&plan, json)
}

fn build_plan(program: &Pubkey, cmd: PlanCmd) -> Result<InstructionPlan> {
    Ok(match cmd {
        PlanCmd::InitializeConfig {
            admin,
            genesis_hash,
            mint,
            mint_decimals,
            min_provider_bond,
            challenge_duration_seconds,
        } => {
            let payload = InitializeConfigPayload {
                genesis_hash: hex32(&genesis_hash)?,
                allowed_mint: parse_pubkey(&mint)?.to_bytes(),
                token_program: TOKEN_PROGRAM_ID.to_bytes(),
                mint_decimals,
                min_provider_bond,
                challenge_duration_seconds,
            };
            plan_initialize_config(program, &parse_pubkey(&admin)?, &payload)?
        }
        PlanCmd::SetPaused { admin, paused } => {
            plan_set_paused(program, &parse_pubkey(&admin)?, paused)?
        }
        PlanCmd::RegisterProvider { authority } => {
            plan_register_provider(program, &parse_pubkey(&authority)?)?
        }
        PlanCmd::AddExecutionKey { authority, key_hex } => {
            plan_add_execution_key(program, &parse_pubkey(&authority)?, &hex32(&key_hex)?)?
        }
        PlanCmd::RevokeExecutionKey { authority, key_hex } => {
            plan_revoke_execution_key(program, &parse_pubkey(&authority)?, &hex32(&key_hex)?)?
        }
        PlanCmd::DepositBond {
            authority,
            mint,
            amount,
        } => {
            if amount == 0 {
                bail!("invalid amount");
            }
            plan_deposit_bond(
                program,
                &parse_pubkey(&authority)?,
                &parse_pubkey(&mint)?,
                amount,
            )?
            .with_mint_amount(&parse_pubkey(&mint)?, amount)
        }
        PlanCmd::WithdrawBond {
            authority,
            mint,
            amount,
        } => {
            if amount == 0 {
                bail!("invalid amount");
            }
            plan_withdraw_bond(
                program,
                &parse_pubkey(&authority)?,
                &parse_pubkey(&mint)?,
                amount,
            )?
            .with_mint_amount(&parse_pubkey(&mint)?, amount)
        }
        PlanCmd::CreateJob {
            buyer,
            provider,
            nonce,
            amount,
            request_hash,
            fund_deadline,
            accept_deadline,
            work_deadline,
            auto_settle_deadline,
            now,
        } => {
            if amount == 0 {
                bail!("invalid amount");
            }
            let payload = CreateJobPayload {
                job_nonce: nonce,
                amount,
                request_hash: hex32(&request_hash)?,
                fund_deadline,
                accept_deadline,
                work_deadline,
                auto_settle_deadline,
            };
            plan_create_job(
                program,
                &parse_pubkey(&buyer)?,
                &parse_pubkey(&provider)?,
                now,
                &payload,
            )?
        }
        PlanCmd::FundJob {
            buyer,
            provider,
            mint,
            nonce,
        } => plan_fund_job(
            program,
            &parse_pubkey(&buyer)?,
            &parse_pubkey(&provider)?,
            &parse_pubkey(&mint)?,
            nonce,
        )?,
        PlanCmd::AcceptJob {
            buyer,
            provider,
            mint,
            nonce,
        } => plan_accept_job(
            program,
            &parse_pubkey(&provider)?,
            &parse_pubkey(&buyer)?,
            &parse_pubkey(&mint)?,
            nonce,
        )?,
        PlanCmd::SubmitReceipt {
            job,
            provider,
            receipt_file,
            execution_pubkey_hex,
            signature_hex,
        } => {
            let dto: ReceiptDto =
                serde_json::from_str(&std::fs::read_to_string(receipt_file)?).context("receipt")?;
            let receipt = dto.to_receipt().map_err(|e| anyhow::anyhow!("{e}"))?;
            let pk = hex32(&execution_pubkey_hex)?;
            let sig = hex64(&signature_hex)?;
            build_submit_receipt_plan(
                program,
                &parse_pubkey(&job)?,
                &parse_pubkey(&provider)?,
                &receipt,
                &pk,
                &sig,
            )?
        }
        PlanCmd::AcceptWork {
            buyer,
            provider,
            mint,
            nonce,
        } => plan_accept_work(
            program,
            &parse_pubkey(&buyer)?,
            &parse_pubkey(&provider)?,
            &parse_pubkey(&mint)?,
            nonce,
        )?,
        PlanCmd::ChallengeWork {
            buyer,
            provider,
            nonce,
            reason_hash,
        } => plan_challenge_work(
            program,
            &parse_pubkey(&buyer)?,
            &parse_pubkey(&provider)?,
            nonce,
            &hex32(&reason_hash)?,
        )?,
        PlanCmd::ResolveTimeout {
            payer,
            buyer,
            provider,
            mint,
            nonce,
            mode,
        } => match mode.as_str() {
            "settle" => plan_resolve_timeout_settle(
                program,
                &parse_pubkey(&payer)?,
                &parse_pubkey(&buyer)?,
                &parse_pubkey(&provider)?,
                &parse_pubkey(&mint)?,
                nonce,
                false,
            )?,
            "settle-challenge" => plan_resolve_timeout_settle(
                program,
                &parse_pubkey(&payer)?,
                &parse_pubkey(&buyer)?,
                &parse_pubkey(&provider)?,
                &parse_pubkey(&mint)?,
                nonce,
                true,
            )?,
            "refund" => plan_resolve_timeout_refund(
                program,
                &parse_pubkey(&payer)?,
                &parse_pubkey(&buyer)?,
                &parse_pubkey(&provider)?,
                &parse_pubkey(&mint)?,
                nonce,
            )?,
            _ => bail!("mode must be settle|settle-challenge|refund"),
        },
        PlanCmd::ExpireUnfunded {
            payer,
            buyer,
            provider,
            nonce,
        } => plan_expire_unfunded(
            program,
            &parse_pubkey(&payer)?,
            &parse_pubkey(&buyer)?,
            &parse_pubkey(&provider)?,
            nonce,
        )?,
        PlanCmd::ExpireUnaccepted {
            payer,
            buyer,
            provider,
            mint,
            nonce,
        } => plan_expire_unaccepted(
            program,
            &parse_pubkey(&payer)?,
            &parse_pubkey(&buyer)?,
            &parse_pubkey(&provider)?,
            &parse_pubkey(&mint)?,
            nonce,
        )?,
        PlanCmd::SlashBond {
            admin,
            buyer,
            provider,
            mint,
            nonce,
        } => plan_slash_bond(
            program,
            &parse_pubkey(&admin)?,
            &parse_pubkey(&buyer)?,
            &parse_pubkey(&provider)?,
            &parse_pubkey(&mint)?,
            nonce,
        )?,
        PlanCmd::CloseJob {
            buyer,
            provider,
            mint,
            nonce,
            include_escrow,
        } => plan_close_job(
            program,
            &parse_pubkey(&buyer)?,
            &parse_pubkey(&provider)?,
            &parse_pubkey(&mint)?,
            nonce,
            include_escrow,
        )?,
    })
}

fn print_plan(plan: &InstructionPlan, json: bool) -> Result<()> {
    if json {
        println!("{}", plan.to_json()?);
    } else {
        let s = plan.summary();
        println!(
            "action={} program_id={} mint={:?} amount={:?} signers={:?}",
            s.action, s.program_id, s.mint, s.amount, s.required_signers
        );
        println!("{}", plan.to_json()?);
    }
    Ok(())
}

fn hex32(s: &str) -> Result<[u8; 32]> {
    let bytes = decode_hex(s)?;
    if bytes.len() != 32 {
        bail!("expected 32-byte hex");
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn hex64(s: &str) -> Result<[u8; 64]> {
    let bytes = decode_hex(s)?;
    if bytes.len() != 64 {
        bail!("expected 64-byte hex");
    }
    let mut out = [0u8; 64];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn decode_hex(s: &str) -> Result<Vec<u8>> {
    let s = s.trim().strip_prefix("0x").unwrap_or(s.trim());
    if !s.len().is_multiple_of(2) {
        bail!("bad hex");
    }
    (0..s.len())
        .step_by(2)
        .map(|i| Ok(u8::from_str_radix(&s[i..i + 2], 16)?))
        .collect()
}
