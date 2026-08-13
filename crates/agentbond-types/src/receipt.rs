use crate::error::ProtocolError;
use sha2::{Digest, Sha256};

/// Frozen signature boundary: changing this layout breaks all existing receipt signatures.
pub const RECEIPT_DOMAIN: &[u8; 20] = b"AGENTBOND_RECEIPT_V1";
pub const RECEIPT_VERSION_V1: u16 = 1;
pub const RECEIPT_ENCODED_LEN: usize = 334;

pub const RECEIPT_OFFSET_DOMAIN: usize = 0;
pub const RECEIPT_OFFSET_VERSION: usize = 20;
pub const RECEIPT_OFFSET_PROGRAM_ID: usize = 22;
pub const RECEIPT_OFFSET_GENESIS_HASH: usize = 54;
pub const RECEIPT_OFFSET_JOB: usize = 86;
pub const RECEIPT_OFFSET_BUYER: usize = 118;
pub const RECEIPT_OFFSET_PROVIDER: usize = 150;
pub const RECEIPT_OFFSET_REQUEST_HASH: usize = 182;
pub const RECEIPT_OFFSET_RESULT_HASH: usize = 214;
pub const RECEIPT_OFFSET_ARTIFACT_HASH: usize = 246;
pub const RECEIPT_OFFSET_SOFTWARE_HASH: usize = 278;
pub const RECEIPT_OFFSET_NONCE: usize = 310;
pub const RECEIPT_OFFSET_CREATED_AT: usize = 318;
pub const RECEIPT_OFFSET_EXPIRES_AT: usize = 326;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentBondWorkReceiptV1 {
    pub program_id: [u8; 32],
    pub genesis_hash: [u8; 32],
    pub job: [u8; 32],
    pub buyer: [u8; 32],
    pub provider: [u8; 32],
    pub request_hash: [u8; 32],
    pub result_hash: [u8; 32],
    pub artifact_hash: [u8; 32],
    pub software_hash: [u8; 32],
    pub job_nonce: u64,
    pub created_at: i64,
    pub expires_at: i64,
}

impl AgentBondWorkReceiptV1 {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.expires_at < self.created_at {
            return Err(ProtocolError::InvalidReceiptTimestamps);
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<[u8; RECEIPT_ENCODED_LEN], ProtocolError> {
        self.validate()?;

        let mut out = [0u8; RECEIPT_ENCODED_LEN];
        out[RECEIPT_OFFSET_DOMAIN..RECEIPT_OFFSET_VERSION].copy_from_slice(RECEIPT_DOMAIN);
        out[RECEIPT_OFFSET_VERSION..RECEIPT_OFFSET_PROGRAM_ID]
            .copy_from_slice(&RECEIPT_VERSION_V1.to_le_bytes());
        out[RECEIPT_OFFSET_PROGRAM_ID..RECEIPT_OFFSET_GENESIS_HASH]
            .copy_from_slice(&self.program_id);
        out[RECEIPT_OFFSET_GENESIS_HASH..RECEIPT_OFFSET_JOB].copy_from_slice(&self.genesis_hash);
        out[RECEIPT_OFFSET_JOB..RECEIPT_OFFSET_BUYER].copy_from_slice(&self.job);
        out[RECEIPT_OFFSET_BUYER..RECEIPT_OFFSET_PROVIDER].copy_from_slice(&self.buyer);
        out[RECEIPT_OFFSET_PROVIDER..RECEIPT_OFFSET_REQUEST_HASH].copy_from_slice(&self.provider);
        out[RECEIPT_OFFSET_REQUEST_HASH..RECEIPT_OFFSET_RESULT_HASH]
            .copy_from_slice(&self.request_hash);
        out[RECEIPT_OFFSET_RESULT_HASH..RECEIPT_OFFSET_ARTIFACT_HASH]
            .copy_from_slice(&self.result_hash);
        out[RECEIPT_OFFSET_ARTIFACT_HASH..RECEIPT_OFFSET_SOFTWARE_HASH]
            .copy_from_slice(&self.artifact_hash);
        out[RECEIPT_OFFSET_SOFTWARE_HASH..RECEIPT_OFFSET_NONCE]
            .copy_from_slice(&self.software_hash);
        out[RECEIPT_OFFSET_NONCE..RECEIPT_OFFSET_CREATED_AT]
            .copy_from_slice(&self.job_nonce.to_le_bytes());
        out[RECEIPT_OFFSET_CREATED_AT..RECEIPT_OFFSET_EXPIRES_AT]
            .copy_from_slice(&self.created_at.to_le_bytes());
        out[RECEIPT_OFFSET_EXPIRES_AT..RECEIPT_ENCODED_LEN]
            .copy_from_slice(&self.expires_at.to_le_bytes());
        Ok(out)
    }

    pub fn decode(data: &[u8]) -> Result<Self, ProtocolError> {
        if data.len() != RECEIPT_ENCODED_LEN {
            return Err(ProtocolError::InvalidReceiptLength);
        }

        if data[RECEIPT_OFFSET_DOMAIN..RECEIPT_OFFSET_VERSION] != RECEIPT_DOMAIN[..] {
            return Err(ProtocolError::InvalidReceiptDomain);
        }

        let version = u16::from_le_bytes([
            data[RECEIPT_OFFSET_VERSION],
            data[RECEIPT_OFFSET_VERSION + 1],
        ]);
        if version != RECEIPT_VERSION_V1 {
            return Err(ProtocolError::UnsupportedReceiptVersion);
        }

        let mut program_id = [0u8; 32];
        let mut genesis_hash = [0u8; 32];
        let mut job = [0u8; 32];
        let mut buyer = [0u8; 32];
        let mut provider = [0u8; 32];
        let mut request_hash = [0u8; 32];
        let mut result_hash = [0u8; 32];
        let mut artifact_hash = [0u8; 32];
        let mut software_hash = [0u8; 32];

        program_id.copy_from_slice(&data[RECEIPT_OFFSET_PROGRAM_ID..RECEIPT_OFFSET_GENESIS_HASH]);
        genesis_hash.copy_from_slice(&data[RECEIPT_OFFSET_GENESIS_HASH..RECEIPT_OFFSET_JOB]);
        job.copy_from_slice(&data[RECEIPT_OFFSET_JOB..RECEIPT_OFFSET_BUYER]);
        buyer.copy_from_slice(&data[RECEIPT_OFFSET_BUYER..RECEIPT_OFFSET_PROVIDER]);
        provider.copy_from_slice(&data[RECEIPT_OFFSET_PROVIDER..RECEIPT_OFFSET_REQUEST_HASH]);
        request_hash
            .copy_from_slice(&data[RECEIPT_OFFSET_REQUEST_HASH..RECEIPT_OFFSET_RESULT_HASH]);
        result_hash
            .copy_from_slice(&data[RECEIPT_OFFSET_RESULT_HASH..RECEIPT_OFFSET_ARTIFACT_HASH]);
        artifact_hash
            .copy_from_slice(&data[RECEIPT_OFFSET_ARTIFACT_HASH..RECEIPT_OFFSET_SOFTWARE_HASH]);
        software_hash.copy_from_slice(&data[RECEIPT_OFFSET_SOFTWARE_HASH..RECEIPT_OFFSET_NONCE]);

        let job_nonce = u64::from_le_bytes(
            data[RECEIPT_OFFSET_NONCE..RECEIPT_OFFSET_CREATED_AT]
                .try_into()
                .map_err(|_| ProtocolError::InvalidReceiptLength)?,
        );
        let created_at = i64::from_le_bytes(
            data[RECEIPT_OFFSET_CREATED_AT..RECEIPT_OFFSET_EXPIRES_AT]
                .try_into()
                .map_err(|_| ProtocolError::InvalidReceiptLength)?,
        );
        let expires_at = i64::from_le_bytes(
            data[RECEIPT_OFFSET_EXPIRES_AT..RECEIPT_ENCODED_LEN]
                .try_into()
                .map_err(|_| ProtocolError::InvalidReceiptLength)?,
        );

        let receipt = Self {
            program_id,
            genesis_hash,
            job,
            buyer,
            provider,
            request_hash,
            result_hash,
            artifact_hash,
            software_hash,
            job_nonce,
            created_at,
            expires_at,
        };
        receipt.validate()?;
        Ok(receipt)
    }

    pub fn digest(&self) -> Result<[u8; 32], ProtocolError> {
        let encoded = self.encode()?;
        let hash = Sha256::digest(encoded);
        let mut out = [0u8; 32];
        out.copy_from_slice(&hash);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sample_receipt() -> AgentBondWorkReceiptV1 {
        AgentBondWorkReceiptV1 {
            program_id: [1u8; 32],
            genesis_hash: [2u8; 32],
            job: [3u8; 32],
            buyer: [4u8; 32],
            provider: [5u8; 32],
            request_hash: [6u8; 32],
            result_hash: [7u8; 32],
            artifact_hash: [8u8; 32],
            software_hash: [9u8; 32],
            job_nonce: 0x0123_4567_89ab_cdef,
            created_at: 1_700_000_000,
            expires_at: 1_700_000_360,
        }
    }

    #[test]
    fn encoded_length_is_334() {
        let encoded = sample_receipt().encode().expect("encode");
        assert_eq!(encoded.len(), 334);
        assert_eq!(RECEIPT_ENCODED_LEN, 334);
        assert_eq!(RECEIPT_DOMAIN.len(), 20);
    }

    #[test]
    fn golden_encoded_vector() {
        let encoded = sample_receipt().encode().expect("encode");
        assert_eq!(encoded, GOLDEN_RECEIPT_BYTES);
    }

    #[test]
    fn encode_decode_round_trip() {
        let receipt = sample_receipt();
        let encoded = receipt.encode().expect("encode");
        let decoded = AgentBondWorkReceiptV1::decode(&encoded).expect("decode");
        assert_eq!(decoded, receipt);
    }

    #[test]
    fn stable_sha256_digest_golden_vector() {
        let digest = sample_receipt().digest().expect("digest");
        assert_eq!(
            digest,
            [
                0x9c, 0x13, 0x7a, 0xd8, 0xb6, 0x5d, 0x1f, 0x6d, 0x13, 0x3e, 0x70, 0xba, 0x40, 0xf8,
                0x00, 0xe4, 0xf7, 0x26, 0x13, 0x94, 0xd8, 0xbb, 0xe7, 0x9a, 0x57, 0x86, 0x75, 0x54,
                0x2b, 0x0a, 0x16, 0x6a,
            ]
        );
    }

    #[test]
    fn wrong_length_rejected() {
        assert_eq!(
            AgentBondWorkReceiptV1::decode(&[]),
            Err(ProtocolError::InvalidReceiptLength)
        );
        assert_eq!(
            AgentBondWorkReceiptV1::decode(&[0u8; 333]),
            Err(ProtocolError::InvalidReceiptLength)
        );
        assert_eq!(
            AgentBondWorkReceiptV1::decode(&[0u8; 335]),
            Err(ProtocolError::InvalidReceiptLength)
        );
    }

    #[test]
    fn trailing_bytes_rejected() {
        let mut data = sample_receipt().encode().expect("encode").to_vec();
        data.push(0xff);
        assert_eq!(
            AgentBondWorkReceiptV1::decode(&data),
            Err(ProtocolError::InvalidReceiptLength)
        );
    }

    #[test]
    fn wrong_domain_rejected() {
        let mut encoded = sample_receipt().encode().expect("encode");
        encoded[0] = b'X';
        assert_eq!(
            AgentBondWorkReceiptV1::decode(&encoded),
            Err(ProtocolError::InvalidReceiptDomain)
        );
    }

    #[test]
    fn unsupported_version_rejected() {
        let mut encoded = sample_receipt().encode().expect("encode");
        encoded[RECEIPT_OFFSET_VERSION] = 2;
        encoded[RECEIPT_OFFSET_VERSION + 1] = 0;
        assert_eq!(
            AgentBondWorkReceiptV1::decode(&encoded),
            Err(ProtocolError::UnsupportedReceiptVersion)
        );
    }

    #[test]
    fn expiration_before_creation_rejected() {
        let mut receipt = sample_receipt();
        receipt.created_at = 100;
        receipt.expires_at = 99;
        assert_eq!(
            receipt.encode(),
            Err(ProtocolError::InvalidReceiptTimestamps)
        );

        let mut encoded = sample_receipt().encode().expect("encode");
        encoded[RECEIPT_OFFSET_CREATED_AT..RECEIPT_OFFSET_EXPIRES_AT]
            .copy_from_slice(&100i64.to_le_bytes());
        encoded[RECEIPT_OFFSET_EXPIRES_AT..RECEIPT_ENCODED_LEN]
            .copy_from_slice(&99i64.to_le_bytes());
        assert_eq!(
            AgentBondWorkReceiptV1::decode(&encoded),
            Err(ProtocolError::InvalidReceiptTimestamps)
        );
    }

    #[test]
    fn min_max_integer_values() {
        let mut receipt = sample_receipt();
        receipt.job_nonce = u64::MIN;
        receipt.created_at = i64::MIN;
        receipt.expires_at = i64::MIN;
        let decoded =
            AgentBondWorkReceiptV1::decode(&receipt.encode().expect("encode")).expect("decode");
        assert_eq!(decoded, receipt);

        receipt.job_nonce = u64::MAX;
        receipt.created_at = i64::MAX;
        receipt.expires_at = i64::MAX;
        let decoded =
            AgentBondWorkReceiptV1::decode(&receipt.encode().expect("encode")).expect("decode");
        assert_eq!(decoded, receipt);
    }

    #[test]
    fn equal_timestamps_allowed() {
        let mut receipt = sample_receipt();
        receipt.created_at = 42;
        receipt.expires_at = 42;
        AgentBondWorkReceiptV1::decode(&receipt.encode().expect("encode")).expect("decode");
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic(data in prop::collection::vec(any::<u8>(), 0..400)) {
            let _ = AgentBondWorkReceiptV1::decode(&data);
        }

        #[test]
        fn round_trip_arbitrary_valid_receipt(
            program_id in any::<[u8; 32]>(),
            genesis_hash in any::<[u8; 32]>(),
            job in any::<[u8; 32]>(),
            buyer in any::<[u8; 32]>(),
            provider in any::<[u8; 32]>(),
            request_hash in any::<[u8; 32]>(),
            result_hash in any::<[u8; 32]>(),
            artifact_hash in any::<[u8; 32]>(),
            software_hash in any::<[u8; 32]>(),
            job_nonce in any::<u64>(),
            created_at in any::<i64>(),
            delta in 0i64..=1_000_000,
        ) {
            let expires_at = created_at.saturating_add(delta);
            let receipt = AgentBondWorkReceiptV1 {
                program_id,
                genesis_hash,
                job,
                buyer,
                provider,
                request_hash,
                result_hash,
                artifact_hash,
                software_hash,
                job_nonce,
                created_at,
                expires_at,
            };
            let encoded = receipt.encode().expect("encode");
            let decoded = AgentBondWorkReceiptV1::decode(&encoded).expect("decode");
            prop_assert_eq!(decoded, receipt);
        }
    }

    const GOLDEN_RECEIPT_BYTES: [u8; RECEIPT_ENCODED_LEN] = [
        0x41, 0x47, 0x45, 0x4e, 0x54, 0x42, 0x4f, 0x4e, 0x44, 0x5f, 0x52, 0x45, 0x43, 0x45, 0x49,
        0x50, 0x54, 0x5f, 0x56, 0x31, 0x01, 0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
        0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
        0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02,
        0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02,
        0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x03, 0x03, 0x03, 0x03,
        0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03,
        0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x04, 0x04,
        0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,
        0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04,
        0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
        0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05, 0x05,
        0x05, 0x05, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06,
        0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06, 0x06,
        0x06, 0x06, 0x06, 0x06, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
        0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
        0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08,
        0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08,
        0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09,
        0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09,
        0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0x09, 0xef, 0xcd, 0xab, 0x89, 0x67,
        0x45, 0x23, 0x01, 0x00, 0xf1, 0x53, 0x65, 0x00, 0x00, 0x00, 0x00, 0x68, 0xf2, 0x53, 0x65,
        0x00, 0x00, 0x00, 0x00,
    ];
}
