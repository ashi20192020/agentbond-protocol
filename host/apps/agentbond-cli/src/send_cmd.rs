use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use agentbond_sdk::{
    ClusterKind, HttpChainReader, InstructionPlan, parse_pubkey, program_id, simulate_and_send_plan,
};
use anyhow::{Context, Result, bail};
use solana_keypair::Keypair;

pub async fn run_send(
    rpc_url: &str,
    payer_path: &PathBuf,
    plan_path: &PathBuf,
    yes: bool,
    allow_mainnet: bool,
    signer_paths: &[PathBuf],
    expected_program: Option<&str>,
) -> Result<()> {
    refuse_insecure_key_perms(payer_path)?;
    for p in signer_paths {
        refuse_insecure_key_perms(p)?;
    }
    let plan = InstructionPlan::from_json(&fs::read_to_string(plan_path)?)?;
    let program = match expected_program {
        Some(s) => parse_pubkey(s)?,
        None => program_id(),
    };
    let rpc = HttpChainReader::new(rpc_url, Duration::from_secs(10))?;
    let genesis = rpc.get_genesis_hash().await?;
    let cluster = agentbond_sdk::cluster_from_genesis_hash(&genesis);
    if cluster == ClusterKind::MainnetBeta && !allow_mainnet {
        bail!("mainnet blocked without --allow-mainnet (detected via genesis hash)");
    }

    let summary = plan.summary();
    println!("network/cluster: {cluster:?}");
    println!("genesis_hash: {genesis}");
    println!("rpc: {rpc_url}");
    println!("program_id: {}", summary.program_id);
    println!("action: {}", summary.action);
    println!("mint: {:?}", summary.mint);
    println!("amount: {:?}", summary.amount);
    println!("required_signers: {:?}", summary.required_signers);
    if !yes {
        bail!("pass --yes to submit after reviewing the summary above");
    }

    let payer = read_keypair(payer_path)?;
    let mut extras = Vec::new();
    for path in signer_paths {
        extras.push(read_keypair(path)?);
    }
    let extra_refs: Vec<&Keypair> = extras.iter().collect();
    let result = simulate_and_send_plan(
        &rpc,
        &plan,
        &program,
        &payer,
        &extra_refs,
        allow_mainnet,
        Duration::from_secs(30),
    )
    .await?;
    println!(
        "{}",
        serde_json::json!({
            "signature": result.signature,
            "status": result.status,
            "cluster": result.cluster,
            "genesis_hash": result.genesis_hash,
        })
    );
    Ok(())
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

fn read_keypair(path: &PathBuf) -> Result<Keypair> {
    let bytes = fs::read(path)?;
    let arr: Vec<u8> = serde_json::from_slice(&bytes)?;
    Keypair::from_bytes(arr.as_slice()).context("keypair")
}
