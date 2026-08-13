use sha2::{Digest, Sha256};

use crate::error::PaymentError;
use crate::facilitator::FacilitatorClient;
use crate::headers::{
    decode_payment_signature_header, encode_payment_required_header, encode_payment_response_header,
};
use crate::models::{PaymentRequired, ResourceInfo, SettleRequest, VerifyRequest, X402_VERSION};
use crate::settlement::SettlementBinding;
use crate::stores::{BeginOutcome, ChallengeStore, SettlementStore, tx_digest};
use crate::validate::validate_payment_payload;

#[derive(Clone, Debug)]
pub struct X402ResourceConfig {
    pub network: String,
    pub asset: String,
    pub pay_to: String,
    pub fee_payer: String,
    pub amount: String,
    pub max_timeout_seconds: u64,
    pub resource_url: String,
    pub description: String,
    pub service_id: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaidDemoResult {
    pub body: serde_json::Value,
    pub payment_response_header: String,
}

pub fn input_digest(input: &serde_json::Value) -> Result<String, PaymentError> {
    let bytes = serde_json::to_vec(input).map_err(|_| PaymentError::InvalidJson)?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

/// Result: Ok(Ok(paid)) | Ok(Err(payment_required_header)) | Err(...)
pub async fn invoke_paid_demo(
    cfg: &X402ResourceConfig,
    facilitator: &dyn FacilitatorClient,
    challenges: &dyn ChallengeStore,
    settlements: &dyn SettlementStore,
    payment_header: Option<&str>,
    input: &serde_json::Value,
    now_unix: i64,
) -> Result<Result<PaidDemoResult, String>, PaymentError> {
    let digest = input_digest(input)?;
    let resource = ResourceInfo {
        url: cfg.resource_url.clone(),
        description: cfg.description.clone(),
        mime_type: "application/json".into(),
    };

    let Some(header) = payment_header else {
        let (requirements, _) = challenges.issue(cfg, &resource, &digest, now_unix).await?;
        let required = PaymentRequired {
            x402_version: X402_VERSION,
            error: Some("PAYMENT-SIGNATURE header is required".into()),
            resource,
            accepts: vec![requirements],
        };
        let encoded = encode_payment_required_header(&required)?;
        return Ok(Err(encoded));
    };

    let payload = decode_payment_signature_header(header)?;
    let memo = payload
        .accepted
        .extra
        .memo
        .clone()
        .ok_or(PaymentError::InvalidChallenge)?;
    let challenge = challenges.get_valid(&memo, now_unix).await?;
    let expected = crate::models::PaymentRequirements {
        scheme: crate::models::SCHEME_EXACT.into(),
        network: challenge.network.clone(),
        amount: challenge.amount.clone(),
        asset: challenge.asset.clone(),
        pay_to: challenge.merchant.clone(),
        max_timeout_seconds: challenge.max_timeout_seconds,
        extra: crate::models::SvmExactExtra {
            fee_payer: challenge.fee_payer.clone(),
            memo: Some(challenge.memo.clone()),
            recent_blockhash: None,
            last_valid_block_height: None,
        },
    };
    let tx_b64 = validate_payment_payload(&payload, &expected, &challenge, now_unix, &digest)?;
    let digest_key = tx_digest(&tx_b64);
    let binding = SettlementBinding {
        service_id: cfg.service_id.clone(),
        resource_url: cfg.resource_url.clone(),
        input_digest: digest,
        challenge_memo: challenge.memo.clone(),
    };

    let lease = match settlements.begin(&digest_key, binding.clone()).await? {
        BeginOutcome::Cached(cached) => return Ok(Ok(cached)),
        BeginOutcome::Acquired(lease) | BeginOutcome::RecoveredStale(lease) => lease,
    };

    let verify = match facilitator
        .verify(&VerifyRequest {
            x402_version: X402_VERSION,
            payment_payload: payload.clone(),
            payment_requirements: expected.clone(),
        })
        .await
    {
        Ok(v) => v,
        Err(e) => {
            let _ = settlements.fail(&digest_key, &binding, &lease).await;
            return Err(e);
        }
    };
    if !verify.is_valid {
        let _ = settlements.fail(&digest_key, &binding, &lease).await;
        return Err(PaymentError::VerifyRejected);
    }

    let input_bytes = serde_json::to_vec(input).map_err(|_| PaymentError::InvalidJson)?;
    let body_digest = Sha256::digest(&input_bytes);
    let body = serde_json::json!({
        "service": "agentbond-x402-demo",
        "input_sha256": body_digest.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "echo": input,
        "note": "deterministic paid demo resource",
    });

    let settle = match facilitator
        .settle(&SettleRequest {
            x402_version: X402_VERSION,
            payment_payload: payload,
            payment_requirements: expected,
        })
        .await
    {
        Ok(s) => s,
        Err(e) => {
            let _ = settlements.fail(&digest_key, &binding, &lease).await;
            return Err(e);
        }
    };
    if !settle.success {
        let _ = settlements.fail(&digest_key, &binding, &lease).await;
        return Err(PaymentError::SettleRejected);
    }

    let response_header = encode_payment_response_header(&settle)?;
    let result = PaidDemoResult {
        body,
        payment_response_header: response_header,
    };
    settlements
        .complete(&digest_key, &binding, &lease, result.clone())
        .await?;
    Ok(Ok(result))
}
