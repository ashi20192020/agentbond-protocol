mod plan_cmd;
mod send_cmd;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use agentbond_app::{
    ReceiptDto, inspect_bond_dto, inspect_challenge_dto, inspect_config_dto, inspect_job_dto,
    inspect_provider_dto,
};
use agentbond_sdk::{
    HttpChainReader, challenge_pda, config_pda, job_escrow_ata, job_pda, parse_pubkey, program_id,
    provider_bond_pda, provider_pda, receipt_digest, user_settlement_ata, validate_receipt,
};
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey};
use solana_pubkey::Pubkey;

use plan_cmd::{PlanCmd, run_plan};
use send_cmd::run_send;

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
#[allow(clippy::large_enum_variant)]
enum ReceiptCmd {
    Create {
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        program_id_hex: String,
        #[arg(long)]
        genesis_hash_hex: String,
        #[arg(long)]
        job_hex: String,
        #[arg(long)]
        buyer_hex: String,
        #[arg(long)]
        provider_hex: String,
        #[arg(long)]
        request_hash_hex: String,
        #[arg(long)]
        result_hash_hex: String,
        #[arg(long)]
        artifact_hash_hex: String,
        #[arg(long)]
        software_hash_hex: String,
        #[arg(long)]
        job_nonce: u64,
        #[arg(long)]
        created_at: i64,
        #[arg(long)]
        expires_at: i64,
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
                .ok_or_else(|| anyhow::anyhow!("--rpc-url is required for inspect"))?;
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
        } => {
            run_send(
                &rpc_url,
                &payer,
                &plan,
                yes,
                allow_mainnet,
                &signer,
                cli.program_id.as_deref(),
            )
            .await?
        }
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
    let value = match cmd {
        InspectCmd::Config => serde_json::to_value(inspect_config_dto(&reader, program).await?)?,
        InspectCmd::Provider { address } => serde_json::to_value(
            inspect_provider_dto(&reader, program, &parse_pubkey(&address)?).await?,
        )?,
        InspectCmd::Bond { address } => serde_json::to_value(
            inspect_bond_dto(&reader, program, &parse_pubkey(&address)?).await?,
        )?,
        InspectCmd::Job { address } => serde_json::to_value(
            inspect_job_dto(&reader, program, &parse_pubkey(&address)?).await?,
        )?,
        InspectCmd::Challenge { address } => serde_json::to_value(
            inspect_challenge_dto(&reader, program, &parse_pubkey(&address)?).await?,
        )?,
    };
    if json {
        println!("{value}");
    } else {
        println!("{}", serde_json::to_string_pretty(&value)?);
    }
    Ok(())
}

fn run_receipt(cmd: ReceiptCmd, json: bool) -> Result<()> {
    match cmd {
        ReceiptCmd::Create {
            file,
            program_id_hex,
            genesis_hash_hex,
            job_hex,
            buyer_hex,
            provider_hex,
            request_hash_hex,
            result_hash_hex,
            artifact_hash_hex,
            software_hash_hex,
            job_nonce,
            created_at,
            expires_at,
        } => {
            let dto = ReceiptDto {
                program_id_hex,
                genesis_hash_hex,
                job_hex,
                buyer_hex,
                provider_hex,
                request_hash_hex,
                result_hash_hex,
                artifact_hash_hex,
                software_hash_hex,
                job_nonce,
                created_at,
                expires_at,
            };
            let receipt = dto.to_receipt().map_err(|e| anyhow::anyhow!("{e}"))?;
            validate_receipt(&receipt)?;
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
                "signature": hex_encode(sig.to_bytes()),
                "public_key": hex_encode(secret.verifying_key().to_bytes()),
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

fn output_has_secret_material(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("secret") || lower.contains("private")
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

fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentbond_sdk::{ClusterKind, MAINNET_GENESIS_HASH, cluster_from_genesis_hash};

    #[test]
    fn mainnet_guard_uses_genesis_hash() {
        assert_eq!(
            cluster_from_genesis_hash(MAINNET_GENESIS_HASH),
            ClusterKind::MainnetBeta
        );
        assert_ne!(
            cluster_from_genesis_hash("11111111111111111111111111111111"),
            ClusterKind::MainnetBeta
        );
    }

    #[test]
    fn secret_material_detector() {
        assert!(output_has_secret_material(r#"{"private_key":"aa"}"#));
        assert!(!output_has_secret_material(
            r#"{"signature":"abcd","public_key":"ef01"}"#
        ));
    }
}
