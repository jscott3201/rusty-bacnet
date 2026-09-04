use bacnet_types::error::Error;

use super::{take, take_u32, take_u64};
use crate::audit::receipt::{
    self, CompletedAuditReceipt, MAX_AUDIT_RECEIPT_KEY_BYTES, MAX_COMPLETED_AUDIT_RECEIPTS,
};

pub(super) fn encoded_len(receipts: &[CompletedAuditReceipt]) -> Result<usize, Error> {
    receipt::validate_receipts(receipts)?;
    receipts.iter().try_fold(4usize, |length, receipt| {
        length
            .checked_add(8 + 4)
            .and_then(|length| length.checked_add(receipt.key().len()))
            .ok_or_else(|| Error::OutOfRange("AuditLog completed receipt length overflow".into()))
    })
}

pub(super) fn encode(
    receipts: &[CompletedAuditReceipt],
    payload: &mut Vec<u8>,
) -> Result<(), Error> {
    encoded_len(receipts)?;
    payload.extend_from_slice(&(receipts.len() as u32).to_be_bytes());
    for receipt in receipts {
        payload.extend_from_slice(&receipt.completed_at_unix_millis().to_be_bytes());
        payload.extend_from_slice(&(receipt.key().len() as u32).to_be_bytes());
        payload.extend_from_slice(receipt.key());
    }
    Ok(())
}

pub(super) fn decode(
    payload: &[u8],
    offset: &mut usize,
) -> Result<Vec<CompletedAuditReceipt>, Error> {
    let count = take_u32(payload, offset, "completed-receipt-count")? as usize;
    if count > MAX_COMPLETED_AUDIT_RECEIPTS {
        return Err(Error::OutOfRange(format!(
            "AuditLog completed receipt count {count} exceeds {MAX_COMPLETED_AUDIT_RECEIPTS}"
        )));
    }
    let mut receipts = Vec::with_capacity(count);
    for _ in 0..count {
        let completed_at = take_u64(payload, offset, "completed-receipt-timestamp")?;
        let key_len = take_u32(payload, offset, "completed-receipt-key-length")? as usize;
        if key_len > MAX_AUDIT_RECEIPT_KEY_BYTES {
            return Err(Error::OutOfRange(format!(
                "AuditLog completed receipt key length {key_len} exceeds {MAX_AUDIT_RECEIPT_KEY_BYTES}"
            )));
        }
        let key = take(payload, offset, key_len, "completed-receipt-key")?.to_vec();
        receipts.push(CompletedAuditReceipt::new(key, completed_at)?);
    }
    receipt::validate_receipts(&receipts)?;
    Ok(receipts)
}
