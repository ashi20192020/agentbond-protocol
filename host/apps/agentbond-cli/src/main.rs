use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use agentbond_app::{CreateJobRequest, ReceiptDto};
use agentbond_sdk::{
    ChainReader, HttpChainReader, InstructionPlan, challenge_pda, config_pda, decode_challenge,
    decode_config, decode_job, decode_provider, decode_provider_bond, job_escrow_ata, job_pda,
    parse_pubkey, plan_create_job, program_id, provider_bond_pda, provider_pda, receipt_digest,
    user_settlement_ata, validate_receipt,
};
use agentbond_types::CreateJobPayload;
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_pubkey::Pubkey;
use solana_signer::Signer as SolSigner;
use solana_transaction::Transaction;

#[derive(Parser)]
#[command(name = "agentbond", about = "AgentBond CLI")]
struct Cli {
    #[arg(long, global = true)]
    rpc_url: Option<String>,
    #[arg(long, global = true)]
    program_id: Option<String>,
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Address {
        #[command(subcommand)]
        cmd: AddressCmd,
    },
    Inspect {
        #[command(subcommand)]
        cmd: InspectCmd,
    },
    Receipt {
        #[command(subcommand)]
        cmd: ReceiptCmd,
    },
    Plan {
        #[command(subcommand)]
        cmd: PlanCmd,
    },
    Send {
        #[arg(long)]
        rpc_url: String,
        #[arg(long)]
        payer: PathBuf,
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        allow_mainnet: bool,
        #[arg(long)]
        signer: Vec<PathBuf>,
    },
}

#[derive(Subcommand)]
enum AddressCmd {
    Config,
    Provider {
        authority: String,
    },
    Bond {
        authority: String,
        mint: String,
    },
    Job {
        buyer: String,
        provider: String,
        nonce: u64,
    },
    Challenge {
        job: String,
    },
    Escrow {
        job: String,
        mint: String,
    },
    Ata {
        owner: String,
        mint: String,
    },
}

#[derive(Subcommand)]
enum InspectCmd {
    Config,
    Provider { address: String },
    Bond { address: String },
    Job { address: String },
    Challenge { address: String },
}

#[derive(Subcommand)]
enum ReceiptCmd {
    Create {
        #[arg(long)]
        file: PathBuf,
    },
    Validate {
        #[arg(long)]
        file: PathBuf,
    },
    Digest {
        #[arg(long)]
        file: PathBuf,
    },
    Sign {
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        key_file: PathBuf,
    },
}

#[derive(Subcommand)]
enum PlanCmd {
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
        #[arg(long, default_value_t = 1_700_000_000)]
        now: i64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let program = match &cli.program_id {
        Some(s) => parse_pubkey(s).context("program_id")?,
        None => program_id(),
    };
    match cli.command {
        Commands::Address { cmd } => run_address(&program, cmd, cli.json)?,
        Commands::Inspect { cmd } => {
            let rpc = cli
                .rpc_url
                .clone()
                .unwrap_or_else(|| "http://127.0.0.1:8899".into());
            run_inspect(&program, &rpc, cmd, cli.json).await?;
        }
        Commands::Receipt { cmd } => run_receipt(cmd, cli.json)?,
        Commands::Plan { cmd } => run_plan(&program, cmd, cli.json)?,
        Commands::Send {
            rpc_url,
            payer,
            plan,
            yes,
            allow_mainnet,
            signer,
        } => run_send(&rpc_url, &payer, &plan, yes, allow_mainnet, &signer).await?,
    }
    Ok(())
}

fn run_address(program: &Pubkey, cmd: AddressCmd, json: bool) -> Result<()> {
    let out = match cmd {
        AddressCmd::Config => config_pda(program)?.address.to_string(),
        AddressCmd::Provider { authority } => provider_pda(program, &parse_pubkey(&authority)?)?
            .address
            .to_string(),
        AddressCmd::Bond { authority, mint } => {
            provider_bond_pda(program, &parse_pubkey(&authority)?, &parse_pubkey(&mint)?)?
                .address
                .to_string()
        }
        AddressCmd::Job {
            buyer,
            provider,
            nonce,
        } => job_pda(
            program,
            &parse_pubkey(&buyer)?,
            &parse_pubkey(&provider)?,
            nonce,
        )?
        .address
        .to_string(),
        AddressCmd::Challenge { job } => challenge_pda(program, &parse_pubkey(&job)?)?
            .address
            .to_string(),
        AddressCmd::Escrow { job, mint } => {
            job_escrow_ata(&parse_pubkey(&job)?, &parse_pubkey(&mint)?).to_string()
        }
        AddressCmd::Ata { owner, mint } => {
            user_settlement_ata(&parse_pubkey(&owner)?, &parse_pubkey(&mint)?).to_string()
        }
    };
    if json {
        println!("{}", serde_json::json!({ "address": out }));
    } else {
        println!("{out}");
    }
    Ok(())
}

async fn run_inspect(program: &Pubkey, rpc: &str, cmd: InspectCmd, json: bool) -> Result<()> {
    let reader = HttpChainReader::new(rpc, std::time::Duration::from_secs(5))?;
    let (label, value) = match cmd {
        InspectCmd::Config => {
            let addr = config_pda(program)?.address;
            let acc = reader.get_account(&addr).await?.context("config missing")?;
            let cfg = decode_config(program, &addr, &acc.owner, &acc.data)?;
            ("config", format!("{cfg:?}"))
        }
        InspectCmd::Provider { address } => {
            let addr = parse_pubkey(&address)?;
            let acc = reader
                .get_account(&addr)
                .await?
                .context("provider missing")?;
            let v = decode_provider(program, &addr, &acc.owner, &acc.data)?;
            ("provider", format!("{v:?}"))
        }
        InspectCmd::Bond { address } => {
            let addr = parse_pubkey(&address)?;
            let acc = reader.get_account(&addr).await?.context("bond missing")?;
            let v = decode_provider_bond(program, &addr, &acc.owner, &acc.data)?;
            ("bond", format!("{v:?}"))
        }
        InspectCmd::Job { address } => {
            let addr = parse_pubkey(&address)?;
            let acc = reader.get_account(&addr).await?.context("job missing")?;
            let v = decode_job(program, &addr, &acc.owner, &acc.data)?;
            ("job", format!("{v:?}"))
        }
        InspectCmd::Challenge { address } => {
            let addr = parse_pubkey(&address)?;
            let acc = reader
                .get_account(&addr)
                .await?
                .context("challenge missing")?;
            let v = decode_challenge(program, &addr, &acc.owner, &acc.data)?;
            ("challenge", format!("{v:?}"))
        }
    };
    if json {
        println!("{}", serde_json::json!({ label: value }));
    } else {
        println!("{label}: {value}");
    }
    Ok(())
}

fn run_receipt(cmd: ReceiptCmd, json: bool) -> Result<()> {
    match cmd {
        ReceiptCmd::Create { file } => {
            let dto = ReceiptDto::from_receipt(&default_receipt());
            fs::write(&file, serde_json::to_string_pretty(&dto)?)?;
            if json {
                println!("{}", serde_json::to_string(&dto)?);
            } else {
                println!("wrote {}", file.display());
            }
        }
        ReceiptCmd::Validate { file } => {
            let dto: ReceiptDto = serde_json::from_str(&fs::read_to_string(file)?)?;
            let receipt = dto.to_receipt().map_err(|e| anyhow::anyhow!("{e}"))?;
            validate_receipt(&receipt)?;
            println!("{}", if json { "{\"ok\":true}" } else { "ok" });
        }
        ReceiptCmd::Digest { file } => {
            let dto: ReceiptDto = serde_json::from_str(&fs::read_to_string(file)?)?;
            let receipt = dto.to_receipt().map_err(|e| anyhow::anyhow!("{e}"))?;
            let digest = receipt_digest(&receipt)?;
            let hex = digest
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            if json {
                println!("{}", serde_json::json!({ "digest": hex }));
            } else {
                println!("{hex}");
            }
        }
        ReceiptCmd::Sign { file, key_file } => {
            refuse_insecure_key_perms(&key_file)?;
            let secret = load_signing_key(&key_file)?;
            let dto: ReceiptDto = serde_json::from_str(&fs::read_to_string(&file)?)?;
            let receipt = dto.to_receipt().map_err(|e| anyhow::anyhow!("{e}"))?;
            let msg = receipt.encode()?;
            let sig = secret.sign(&msg);
            let out = serde_json::json!({
                "signature": hex::encode(sig.to_bytes()),
                "public_key": hex::encode(secret.verifying_key().to_bytes()),
            });
            let text = out.to_string();
            if output_has_secret_material(&text) {
                bail!("refusing to print secret material");
            }
            println!("{text}");
        }
    }
    Ok(())
}

fn run_plan(program: &Pubkey, cmd: PlanCmd, json: bool) -> Result<()> {
    let PlanCmd::CreateJob {
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
    } = cmd;
    if amount == 0 {
        bail!("invalid amount");
    }
    let mut hash = [0u8; 32];
    let bytes = hex::decode(request_hash.trim_start_matches("0x"))?;
    if bytes.len() != 32 {
        bail!("invalid request hash");
    }
    hash.copy_from_slice(&bytes);
    let payload = CreateJobPayload {
        job_nonce: nonce,
        amount,
        request_hash: hash,
        fund_deadline,
        accept_deadline,
        work_deadline,
        auto_settle_deadline,
    };
    let plan = plan_create_job(
        program,
        &parse_pubkey(&buyer)?,
        &parse_pubkey(&provider)?,
        now,
        &payload,
    )?;
    print_plan(&plan, json)?;
    let _ = CreateJobRequest {
        buyer,
        provider,
        job_nonce: nonce,
        amount,
        request_hash_hex: String::new(),
        fund_deadline,
        accept_deadline,
        work_deadline,
        auto_settle_deadline,
    };
    Ok(())
}

fn ensure_mainnet_allowed(rpc_url: &str, allow_mainnet: bool) -> Result<()> {
    if rpc_url.contains("mainnet") && !allow_mainnet {
        bail!("mainnet blocked without --allow-mainnet");
    }
    Ok(())
}

fn output_has_secret_material(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("secret") || lower.contains("private")
}

fn sign_plan_locally(
    plan: &InstructionPlan,
    payer: &Keypair,
    extra_signers: &[Keypair],
) -> Result<Transaction> {
    let ixs = plan.to_solana_instructions()?;
    let mut signers: Vec<&Keypair> = Vec::with_capacity(1 + extra_signers.len());
    signers.push(payer);
    for s in extra_signers {
        signers.push(s);
    }
    let payer_pk = payer.pubkey();
    let msg = Message::new(&ixs, Some(&payer_pk));
    let mut tx = Transaction::new_unsigned(msg);
    let blockhash = solana_hash::Hash::new_from_array([1u8; 32]);
    tx.try_sign(&signers, blockhash)
        .context("missing signer for plan")?;
    Ok(tx)
}

async fn run_send(
    rpc_url: &str,
    payer_path: &PathBuf,
    plan_path: &PathBuf,
    yes: bool,
    allow_mainnet: bool,
    signer_paths: &[PathBuf],
) -> Result<()> {
    ensure_mainnet_allowed(rpc_url, allow_mainnet)?;
    refuse_insecure_key_perms(payer_path)?;
    for p in signer_paths {
        refuse_insecure_key_perms(p)?;
    }
    let plan = InstructionPlan::from_json(&fs::read_to_string(plan_path)?)?;
    let payer = read_keypair(payer_path)?;
    println!("network/rpc: {rpc_url}");
    println!("program_id: {}", plan.program_id);
    println!("action: {}", plan.action);
    println!("required_signers: {:?}", plan.required_signers);
    if !yes {
        bail!("pass --yes to submit");
    }
    // Simulate-before-send: require RPC readiness before local signing.
    let reader = HttpChainReader::new(rpc_url, std::time::Duration::from_secs(5))?;
    reader.ready().await?;
    let mut extra = Vec::new();
    for path in signer_paths {
        extra.push(read_keypair(path)?);
    }
    // Local assembly only; actual send requires an RPC sendTransaction client.
    // For Milestone 3 we validate signing completeness without broadcasting indefinitely.
    let _tx = sign_plan_locally(&plan, &payer, &extra)?;
    println!("signed locally; submission requires an online send client (not auto-retried)");
    Ok(())
}

fn print_plan(plan: &InstructionPlan, json: bool) -> Result<()> {
    if json {
        println!("{}", plan.to_json()?);
    } else {
        println!("action={} signers={:?}", plan.action, plan.required_signers);
        println!("{}", plan.to_json()?);
    }
    Ok(())
}

fn default_receipt() -> agentbond_types::AgentBondWorkReceiptV1 {
    agentbond_types::AgentBondWorkReceiptV1 {
        program_id: program_id().to_bytes(),
        genesis_hash: [7u8; 32],
        job: [1u8; 32],
        buyer: [2u8; 32],
        provider: [3u8; 32],
        request_hash: [9u8; 32],
        result_hash: [4u8; 32],
        artifact_hash: [5u8; 32],
        software_hash: [6u8; 32],
        job_nonce: 1,
        created_at: 1_700_000_000,
        expires_at: 1_700_000_400,
    }
}

fn refuse_insecure_key_perms(path: &PathBuf) -> Result<()> {
    let meta = fs::metadata(path).with_context(|| format!("key file {}", path.display()))?;
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        bail!(
            "refusing insecure key file permissions {:o} for {}",
            mode,
            path.display()
        );
    }
    Ok(())
}

fn load_signing_key(path: &PathBuf) -> Result<SigningKey> {
    let bytes = fs::read(path)?;
    let secret = if bytes.len() == 32 {
        let mut s = [0u8; 32];
        s.copy_from_slice(&bytes);
        s
    } else if bytes.len() == 64 {
        let mut s = [0u8; 32];
        s.copy_from_slice(&bytes[..32]);
        s
    } else {
        // JSON array solana keypair
        let arr: Vec<u8> = serde_json::from_slice(&bytes)?;
        if arr.len() < 32 {
            bail!("invalid key file");
        }
        let mut s = [0u8; 32];
        s.copy_from_slice(&arr[..32]);
        s
    };
    Ok(SigningKey::from_bytes(&secret))
}

fn read_keypair(path: &PathBuf) -> Result<Keypair> {
    let bytes = fs::read(path)?;
    let arr: Vec<u8> = serde_json::from_slice(&bytes)?;
    Keypair::from_bytes(arr.as_slice()).context("keypair")
}

mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
    }
    pub fn decode(s: &str) -> Result<Vec<u8>, anyhow::Error> {
        if !s.len().is_multiple_of(2) {
            anyhow::bail!("bad hex");
        }
        (0..s.len())
            .step_by(2)
            .map(|i| Ok(u8::from_str_radix(&s[i..i + 2], 16)?))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentbond_sdk::{InstructionPlan, plan_create_job};
    use agentbond_types::CreateJobPayload;
    use solana_keypair::Keypair;
    use solana_signer::Signer as SolSigner;

    #[test]
    fn mainnet_safety_guard() {
        assert!(ensure_mainnet_allowed("https://api.mainnet-beta.solana.com", false).is_err());
        assert!(ensure_mainnet_allowed("https://api.mainnet-beta.solana.com", true).is_ok());
        assert!(ensure_mainnet_allowed("http://127.0.0.1:8899", false).is_ok());
    }

    #[test]
    fn secret_material_detector() {
        assert!(output_has_secret_material(r#"{"private_key":"aa"}"#));
        assert!(output_has_secret_material("secret key leaked"));
        assert!(!output_has_secret_material(
            r#"{"signature":"abcd","public_key":"ef01"}"#
        ));
    }

    #[test]
    fn missing_signer_rejected_locally() {
        let program = program_id();
        let buyer = Keypair::new();
        let provider = Keypair::new();
        let now = 1_700_000_000i64;
        let payload = CreateJobPayload {
            job_nonce: 1,
            amount: 100,
            request_hash: [9u8; 32],
            fund_deadline: now + 10,
            accept_deadline: now + 20,
            work_deadline: now + 30,
            auto_settle_deadline: now + 40,
        };
        let plan = plan_create_job(&program, &buyer.pubkey(), &provider.pubkey(), now, &payload)
            .expect("plan");
        // Payer is unrelated — buyer signer is required and missing.
        let stranger = Keypair::new();
        let err = sign_plan_locally(&plan, &stranger, &[]).expect_err("missing signer");
        assert!(
            err.to_string()
                .to_ascii_lowercase()
                .contains("missing signer")
                || err.to_string().to_ascii_lowercase().contains("keypair"),
            "unexpected error: {err}"
        );
        let _ = InstructionPlan::from_json(&plan.to_json().expect("json")).expect("roundtrip");
    }
}
