//! The text of every refusal this crate moved from `Error::OutOfRange`
//! text to the static `Error::Range` form (#1833), each from its own site:
//! the label and its numbers print what the old message did, prefix and
//! all. `RangeError` doesn't check a label's placeholders against its
//! numbers, so this table is what does.

use alloc::collections::VecDeque;
use alloc::sync::Arc;

use crate::prelude::*;

use bacnet_types::constructed::{
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, LogDatum,
};
use bacnet_types::enums::AuditLevel;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};

use crate::audit::{
    AuditLogObject, AuditLogPersistence, AuditLogSnapshot, AuditReporterObject,
    CompletedAuditReceipt, MAX_AUDIT_RECEIPT_KEY_BYTES, MAX_AUDIT_RECORDS,
    MAX_COMPLETED_AUDIT_RECEIPTS,
};
use crate::log_buffer::LogRecordBuffer;
use crate::log_persistence::codec::{SnapshotRecord, check_buffer_size, max_buffer_size};
use crate::multistate::{MAX_NUMBER_OF_STATES, MultiStateValueObject};
use crate::trend::TrendLogObject;

/// Storage holding nothing; the capacity check runs before it is read.
struct Empty;

impl AuditLogPersistence for Empty {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(None)
    }

    fn commit(&self, _: &AuditLogSnapshot) -> Result<(), Error> {
        Ok(())
    }
}

/// A record at hour `hour` of an actual day.
fn record(hour: u8) -> BACnetLogRecord {
    BACnetLogRecord {
        date: DATE,
        time: Time { hour, ..TIME },
        log_datum: LogDatum::UnsignedValue(u64::from(hour)),
        status_flags: None,
    }
}

/// Monday 2 November 2026, 08:00.
const DATE: Date = Date {
    year: 126,
    month: 11,
    day: 2,
    day_of_week: 1,
};
const TIME: Time = Time {
    hour: 8,
    minute: 0,
    second: 0,
    hundredths: 0,
};

/// The Buffer_Size refusal of a durable log of `R`'s records, and the text
/// the old message printed for it.
fn durable<R: SnapshotRecord>() -> (Result<(), Error>, alloc::string::String) {
    let max = max_buffer_size::<R>();
    (
        check_buffer_size::<R>(max + 1),
        alloc::format!(
            "value out of range: a durable {:?} keeps at most {max} records, not Buffer_Size {}",
            R::OBJECT_TYPE,
            max + 1
        ),
    )
}

#[test]
fn every_range_refusal_prints_its_old_text() {
    let one_record = || LogRecordBuffer::<BACnetLogRecord>::new(1);
    let receipts: Vec<CompletedAuditReceipt> = (0..=MAX_COMPLETED_AUDIT_RECEIPTS as u32)
        .map(|key| CompletedAuditReceipt::new(key.to_be_bytes().to_vec(), 0).unwrap())
        .collect();
    let mut cases: Vec<(&str, Result<(), Error>, alloc::string::String)> = vec![
        (
            "audit.rs AuditLog capacity",
            AuditLogObject::new(1, "AL", MAX_AUDIT_RECORDS + 1, Arc::new(Empty)).map(drop),
            alloc::format!(
                "value out of range: AuditLog capacity {} exceeds {MAX_AUDIT_RECORDS}",
                MAX_AUDIT_RECORDS + 1
            ),
        ),
        (
            "audit/receipt.rs key length",
            CompletedAuditReceipt::new(vec![7; MAX_AUDIT_RECEIPT_KEY_BYTES + 1], 0).map(drop),
            alloc::format!(
                "value out of range: AuditLog completed receipt key length {} exceeds \
                 {MAX_AUDIT_RECEIPT_KEY_BYTES}",
                MAX_AUDIT_RECEIPT_KEY_BYTES + 1
            ),
        ),
        (
            "audit/receipt.rs receipt count",
            crate::audit::validate_receipts(&receipts),
            alloc::format!(
                "value out of range: AuditLog completed receipt count {} exceeds \
                 {MAX_COMPLETED_AUDIT_RECEIPTS}",
                MAX_COMPLETED_AUDIT_RECEIPTS + 1
            ),
        ),
        (
            "audit/reporter_change.rs DEFAULT level",
            AuditReporterObject::new(1, "AR")
                .unwrap()
                .set_audit_level(AuditLevel::DEFAULT),
            "value out of range: Audit Reporter audit level must not be DEFAULT".into(),
        ),
        (
            "log_buffer.rs records past Buffer_Size",
            one_record().restore(2, VecDeque::from([record(1), record(2)]), false),
            "value out of range: 2 log records exceed Buffer_Size 1".into(),
        ),
        (
            "log_buffer.rs records with a zero count",
            one_record().restore(0, VecDeque::from([record(1)]), false),
            "value out of range: 1 log records restored with a Total_Record_Count of 0".into(),
        ),
        (
            "log_buffer.rs a full Stop_When_Full buffer",
            one_record().restore(1, VecDeque::from([record(1)]), true),
            "value out of range: a log with Stop_When_Full and Enable TRUE can't hold a full \
             buffer"
                .into(),
        ),
        (
            "log_lifecycle.rs LOG_INTERRUPTED time",
            TrendLogObject::new(1, "TL", 4)
                .unwrap()
                .record_interruption(
                    Date {
                        day_of_week: 2,
                        ..DATE
                    },
                    TIME,
                ),
            "value out of range: a LOG_INTERRUPTED record needs an actual date and time".into(),
        ),
        (
            "multistate number_of_states 0",
            MultiStateValueObject::new(1, "MSV", 0).map(drop),
            "value out of range: number_of_states must be at least 1".into(),
        ),
        (
            "multistate number_of_states past the cap",
            MultiStateValueObject::new(1, "MSV", MAX_NUMBER_OF_STATES + 1).map(drop),
            alloc::format!(
                "value out of range: number_of_states must be at most {MAX_NUMBER_OF_STATES}"
            ),
        ),
    ];
    // log_persistence/codec.rs: one site, one label per record family.
    for (family, (result, text)) in [
        ("Trend Log", durable::<BACnetLogRecord>()),
        ("Event Log", durable::<BACnetEventLogRecord>()),
        ("Trend Log Multiple", durable::<BACnetLogMultipleRecord>()),
    ] {
        cases.push((family, result, text));
    }
    assert_eq!(cases.len(), 13, "eleven sites, one of them three labels");
    for (site, result, text) in cases {
        match result {
            Err(error @ Error::Range(_)) => assert_eq!(error.to_string(), text, "{site}"),
            other => panic!("{site}: expected Error::Range, got {other:?}"),
        }
    }
}
