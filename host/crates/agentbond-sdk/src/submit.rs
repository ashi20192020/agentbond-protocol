use std::collections::HashSet;
use std::time::{Duration, Instant};

use base64::Engine;
use serde::Deserialize;
use solana_hash::Hash;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;

use crate::error::SdkError;
use crate::plan::InstructionPlan;
use crate::rpc::{ChainReader, HttpChainReader};

pub const ED25519_PROGRAM_ID: &str = "Ed25519SigVerify111111111111111111111111111";

/// Known mainnet-beta genesis hash (base58).
pub const MAINNET_GENESIS_HASH: &str = "5eykt4UsLoyBJaSfb9PRppPPjqSV4kt3Kg8ndRVYMQ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClusterKind {
    MainnetBeta,
    Devnet,
    Testnet,
    LocalOrUnknown,
}

pub fn cluster_from_genesis_hash(genesis: &str) -> ClusterKind {
    match genesis {
        MAINNET_GENESIS_HASH => ClusterKind::MainnetBeta,
        "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG" => ClusterKind::Devnet,
        "4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY" => ClusterKind::Testnet,
        _ => ClusterKind::LocalOrUnknown,
    }
}

pub fn validate_plan_instructions(
    plan: &InstructionPlan,
    expected_program: &Pubkey,
    now: i64,
) -> Result<(), SdkError> {
    let plan_program: Pubkey = plan
        .program_id
        .parse()
        .map_err(|_| SdkError::InvalidPubkey(plan.program_id.clone()))?;
    if &plan_program != expected_program {
        return Err(SdkError::InvalidInput(
            "plan.program_id does not match configured program".into(),
        ));
    }
    if plan.expires_at.is_some_and(|expires| now > expires) {
        return Err(SdkError::InvalidInput("plan has expired".into()));
    }

    let ixs = plan.to_solana_instructions()?;
    if ixs.is_empty() {
        return Err(SdkError::InvalidInput("plan has no instructions".into()));
    }

    let ed25519: Pubkey = ED25519_PROGRAM_ID
        .parse()
        .map_err(|_| SdkError::InvalidInput("bad ed25519 program id".into()))?;

    for (idx, ix) in ixs.iter().enumerate() {
        if ix.program_id == plan_program {
            continue;
        }
        if ix.program_id == ed25519 {
            let next = ixs.get(idx + 1).ok_or_else(|| {
                SdkError::InvalidInput("Ed25519 instruction must precede SubmitReceipt".into())
            })?;
            if next.program_id != plan_program {
                return Err(SdkError::InvalidInput(
                    "Ed25519 instruction must immediately precede AgentBond SubmitReceipt".into(),
                ));
            }
            continue;
        }
        return Err(SdkError::InvalidInput(format!(
            "plan contains disallowed program {}",
            ix.program_id
        )));
    }

    let mut meta_signers = HashSet::new();
    for ix in &ixs {
        for acc in &ix.accounts {
            if acc.is_signer {
                meta_signers.insert(acc.pubkey.to_string());
            }
        }
    }
    for required in &plan.required_signers {
        if !meta_signers.contains(required) {
            return Err(SdkError::InvalidInput(format!(
                "required signer {required} missing from instruction account metadata"
            )));
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct SubmitResult {
    pub signature: String,
    pub status: String,
    pub cluster: String,
    pub genesis_hash: String,
}

pub async fn simulate_and_send_plan(
    rpc: &HttpChainReader,
    plan: &InstructionPlan,
    expected_program: &Pubkey,
    payer: &Keypair,
    extra_signers: &[&Keypair],
    allow_mainnet: bool,
    confirm_deadline: Duration,
) -> Result<SubmitResult, SdkError> {
    let genesis = rpc.get_genesis_hash().await?;
    let cluster = cluster_from_genesis_hash(&genesis);
    if cluster == ClusterKind::MainnetBeta && !allow_mainnet {
        return Err(SdkError::InvalidInput(
            "mainnet blocked without --allow-mainnet".into(),
        ));
    }
    let now = rpc.get_unix_timestamp().await?;
    validate_plan_instructions(plan, expected_program, now)?;

    let blockhash = rpc.get_latest_blockhash().await?;
    let ixs = plan.to_solana_instructions()?;
    let mut signers: Vec<&Keypair> = Vec::with_capacity(1 + extra_signers.len());
    signers.push(payer);
    for s in extra_signers {
        if s.pubkey() != payer.pubkey() {
            signers.push(s);
        }
    }

    let present: HashSet<String> = signers.iter().map(|k| k.pubkey().to_string()).collect();
    for required in &plan.required_signers {
        if !present.contains(required) {
            return Err(SdkError::InvalidInput(format!(
                "missing required signer {required}"
            )));
        }
    }

    let msg = Message::new(&ixs, Some(&payer.pubkey()));
    let mut tx = Transaction::new_unsigned(msg);
    tx.try_sign(&signers, blockhash)
        .map_err(|e| SdkError::InvalidInput(format!("signing failed: {e}")))?;

    let sim = rpc.simulate_transaction(&tx).await?;
    if let Some(err) = sim.err {
        return Err(SdkError::Rpc(format!("simulation failed: {err}")));
    }

    let signature = rpc.send_transaction(&tx).await?;
    let status = rpc.confirm_signature(&signature, confirm_deadline).await?;

    Ok(SubmitResult {
        signature,
        status,
        cluster: format!("{cluster:?}"),
        genesis_hash: genesis,
    })
}

#[derive(Clone, Debug, Deserialize)]
pub struct SimulateOutcome {
    pub err: Option<serde_json::Value>,
    pub logs: Option<Vec<String>>,
}

impl HttpChainReader {
    pub async fn get_genesis_hash(&self) -> Result<String, SdkError> {
        self.rpc_call_string("getGenesisHash", serde_json::json!([]))
            .await
    }

    pub async fn get_latest_blockhash(&self) -> Result<Hash, SdkError> {
        #[derive(Deserialize)]
        struct Value {
            value: BlockhashValue,
        }
        #[derive(Deserialize)]
        struct BlockhashValue {
            blockhash: String,
        }
        let result: Value = self
            .rpc_call(
                "getLatestBlockhash",
                serde_json::json!([{ "commitment": "confirmed" }]),
            )
            .await?;
        result
            .value
            .blockhash
            .parse()
            .map_err(|e| SdkError::Rpc(format!("bad blockhash: {e}")))
    }

    pub async fn simulate_transaction(
        &self,
        tx: &Transaction,
    ) -> Result<SimulateOutcome, SdkError> {
        let wire = bincode::serialize(tx).map_err(|e| SdkError::Rpc(e.to_string()))?;
        let encoded = Engine::encode(&base64::engine::general_purpose::STANDARD, wire);
        #[derive(Deserialize)]
        struct Wrap {
            value: SimulateOutcome,
        }
        let result: Wrap = self
            .rpc_call(
                "simulateTransaction",
                serde_json::json!([
                    encoded,
                    { "encoding": "base64", "commitment": "confirmed" }
                ]),
            )
            .await?;
        Ok(result.value)
    }

    pub async fn send_transaction(&self, tx: &Transaction) -> Result<String, SdkError> {
        let wire = bincode::serialize(tx).map_err(|e| SdkError::Rpc(e.to_string()))?;
        let encoded = Engine::encode(&base64::engine::general_purpose::STANDARD, wire);
        self.rpc_call_string(
            "sendTransaction",
            serde_json::json!([
                encoded,
                { "encoding": "base64", "preflightCommitment": "confirmed", "maxRetries": 0 }
            ]),
        )
        .await
    }

    pub async fn confirm_signature(
        &self,
        signature: &str,
        deadline: Duration,
    ) -> Result<String, SdkError> {
        let start = Instant::now();
        let mut attempts = 0u32;
        loop {
            if start.elapsed() > deadline || attempts >= 20 {
                return Err(SdkError::Rpc("confirmation deadline exceeded".into()));
            }
            attempts += 1;
            #[derive(Deserialize)]
            struct Value {
                value: Option<Vec<Option<StatusValue>>>,
            }
            #[derive(Deserialize)]
            struct StatusValue {
                confirmation_status: Option<String>,
                err: Option<serde_json::Value>,
            }
            let result: Value = self
                .rpc_call(
                    "getSignatureStatuses",
                    serde_json::json!([[signature], { "searchTransactionHistory": true }]),
                )
                .await?;
            if let Some(Some(Some(status))) = result.value.map(|v| v.into_iter().next()) {
                if let Some(err) = status.err {
                    return Err(SdkError::Rpc(format!("transaction failed: {err}")));
                }
                if let Some(cs) = status
                    .confirmation_status
                    .filter(|cs| matches!(cs.as_str(), "confirmed" | "finalized"))
                {
                    return Ok(cs);
                }
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    }
}
