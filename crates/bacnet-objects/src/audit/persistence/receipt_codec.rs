use crate::prelude::*;

use super::{take, take_u32, take_u64};
use crate::audit::receipt::{
    self, CompletedAuditReceipt, MAX_AUDIT_RECEIPT_KEY_BYTES, MAX_COMPLETED_AUDIT_RECEIPTS,
};
use crate::snapshot::PersistenceError;

/// The encoded length of receipts `validate_receipts` already passed, as
/// `check` and the snapshot encoder have them (#2233).
pub(super) fn checked_len(receipts: &[CompletedAuditReceipt]) -> Result<usize, PersistenceError> {
    receipts.iter().try_fold(4usize, |length, receipt| {
        length
            .checked_add(8 + 4)
            .and_then(|length| length.checked_add(receipt.key().len()))
            .ok_or(crate::snapshot::refused_on_encode(
                "completed receipt length overflows",
            ))
    })
}

pub(super) fn encode(
    receipts: &[CompletedAuditReceipt],
    payload: &mut Vec<u8>,
) -> Result<(), PersistenceError> {
    // Its one caller, the snapshot encoder, ran `check` first, which
    // validated the receipts (#2233).
    checked_len(receipts)?;
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
) -> Result<Vec<CompletedAuditReceipt>, PersistenceError> {
    let count = take_u32(payload, offset, "completed-receipt-count")? as usize;
    if count > MAX_COMPLETED_AUDIT_RECEIPTS {
        return Err(crate::snapshot::damaged(
            "completed receipt count exceeds the cap",
        ));
    }
    // Each receipt takes at least its timestamp and key length.
    let room = (payload.len() - *offset) / (8 + 4);
    let mut receipts = Vec::with_capacity(count.min(room));
    for _ in 0..count {
        let completed_at = take_u64(payload, offset, "completed-receipt-timestamp")?;
        let key_len = take_u32(payload, offset, "completed-receipt-key-length")? as usize;
        if key_len > MAX_AUDIT_RECEIPT_KEY_BYTES {
            return Err(crate::snapshot::damaged(
                "completed receipt key length exceeds the cap",
            ));
        }
        let key = take(payload, offset, key_len, "completed-receipt-key")?.to_vec();
        receipts.push(
            CompletedAuditReceipt::new(key, completed_at)
                .map_err(|_| crate::snapshot::damaged("holds an empty completed receipt key"))?,
        );
    }
    receipt::validate_receipts(&receipts)
        .map_err(|_| crate::snapshot::damaged("completed receipt keys must be unique"))?;
    Ok(receipts)
}
