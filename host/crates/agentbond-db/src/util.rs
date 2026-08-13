use rust_decimal::Decimal;

use crate::error::DbError;

pub fn u64_to_numeric(v: u64) -> Decimal {
    Decimal::from(v)
}

pub fn numeric_to_u64(v: Decimal) -> Result<u64, DbError> {
    if !v.fract().is_zero() || v.is_sign_negative() {
        return Err(DbError::Validation("expected non-negative integer".into()));
    }
    let s = v.normalize().to_string();
    s.parse::<u64>()
        .map_err(|_| DbError::Validation(format!("u64 overflow: {s}")))
}

pub fn pk_bytes(pk: &str) -> Result<[u8; 32], DbError> {
    let bytes = bs58::decode(pk)
        .into_vec()
        .map_err(|_| DbError::Validation("invalid base58 pubkey".into()))?;
    if bytes.len() != 32 {
        return Err(DbError::Validation("pubkey must be 32 bytes".into()));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

pub fn pk_str(bytes: &[u8]) -> Result<String, DbError> {
    if bytes.len() != 32 {
        return Err(DbError::Validation("pubkey bytes length".into()));
    }
    Ok(bs58::encode(bytes).into_string())
}

pub fn hex32(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
