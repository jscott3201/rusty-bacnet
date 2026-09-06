use std::collections::HashSet;

use bacnet_types::error::Error;

/// Repository retention policy for completed confirmed Audit receipts.
pub const COMPLETED_AUDIT_RECEIPT_RETENTION_MILLIS: u64 = 60_000;
/// Maximum completed confirmed Audit receipts retained by one Audit Log.
pub const MAX_COMPLETED_AUDIT_RECEIPTS: usize = 256;
/// Maximum exact-key size, including the bounded 64-KiB service payload.
pub const MAX_AUDIT_RECEIPT_KEY_BYTES: usize = 64 * 1024 + 1024;

/// One completed confirmed Audit request retained in an Audit Log snapshot.
///
/// The opaque key is an injective server-owned encoding of the canonical
/// requester and complete confirmed request. Persistence implementations must
/// retain this field with the records from the same snapshot transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletedAuditReceipt {
    key: Vec<u8>,
    completed_at_unix_millis: u64,
}

impl CompletedAuditReceipt {
    /// Construct one bounded completed receipt at a Unix UTC millisecond.
    pub fn new(key: Vec<u8>, completed_at_unix_millis: u64) -> Result<Self, Error> {
        validate_key(&key)?;
        Ok(Self {
            key,
            completed_at_unix_millis,
        })
    }

    /// Exact opaque request identity.
    pub fn key(&self) -> &[u8] {
        &self.key
    }

    /// Restart-stable UTC completion time in Unix milliseconds.
    pub fn completed_at_unix_millis(&self) -> u64 {
        self.completed_at_unix_millis
    }
}

/// Result of atomically checking and storing a confirmed Audit receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmedAuditNotificationOutcome {
    /// Notifications and the newly completed receipt were durably accepted.
    Stored,
    /// The exact completed receipt was already retained and no state changed.
    Duplicate,
}

pub(super) fn validate_key(key: &[u8]) -> Result<(), Error> {
    if key.is_empty() {
        return Err(Error::Encoding(
            "AuditLog completed receipt key must not be empty".into(),
        ));
    }
    if key.len() > MAX_AUDIT_RECEIPT_KEY_BYTES {
        return Err(Error::OutOfRange(format!(
            "AuditLog completed receipt key length {} exceeds {MAX_AUDIT_RECEIPT_KEY_BYTES}",
            key.len()
        )));
    }
    Ok(())
}

pub(super) fn validate_receipts(receipts: &[CompletedAuditReceipt]) -> Result<(), Error> {
    if receipts.len() > MAX_COMPLETED_AUDIT_RECEIPTS {
        return Err(Error::OutOfRange(format!(
            "AuditLog completed receipt count {} exceeds {MAX_COMPLETED_AUDIT_RECEIPTS}",
            receipts.len()
        )));
    }
    let mut keys = HashSet::with_capacity(receipts.len());
    for receipt in receipts {
        validate_key(receipt.key())?;
        if !keys.insert(receipt.key()) {
            return Err(Error::Encoding(
                "AuditLog completed receipt keys must be unique".into(),
            ));
        }
    }
    Ok(())
}

pub(super) fn contains(
    receipts: &[CompletedAuditReceipt],
    key: &[u8],
    now_unix_millis: u64,
) -> Result<bool, Error> {
    validate_key(key)?;
    Ok(receipts.iter().any(|receipt| {
        receipt.key() == key
            && now_unix_millis >= receipt.completed_at_unix_millis()
            && now_unix_millis - receipt.completed_at_unix_millis()
                < COMPLETED_AUDIT_RECEIPT_RETENTION_MILLIS
    }))
}

pub(super) fn insert(
    receipts: &mut Vec<CompletedAuditReceipt>,
    receipt: CompletedAuditReceipt,
) -> Result<ConfirmedAuditNotificationOutcome, Error> {
    validate_key(receipt.key())?;
    let now = receipt.completed_at_unix_millis();
    receipts.retain(|stored| {
        now >= stored.completed_at_unix_millis()
            && now - stored.completed_at_unix_millis() < COMPLETED_AUDIT_RECEIPT_RETENTION_MILLIS
    });
    if receipts.iter().any(|stored| stored.key() == receipt.key()) {
        return Ok(ConfirmedAuditNotificationOutcome::Duplicate);
    }
    if receipts.len() == MAX_COMPLETED_AUDIT_RECEIPTS {
        let oldest = receipts
            .iter()
            .enumerate()
            .min_by_key(|(index, stored)| (stored.completed_at_unix_millis(), *index))
            .map(|(index, _)| index)
            .expect("a full receipt ledger is nonempty");
        receipts.remove(oldest);
    }
    receipts.push(receipt);
    Ok(ConfirmedAuditNotificationOutcome::Stored)
}
