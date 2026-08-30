use std::sync::{Arc, Mutex};

use bacnet_types::bitstring::AuditOperationFlags;
use bacnet_types::constructed::{
    AuditPropertyReference, BACnetAddress, BACnetAuditLogDatum, BACnetAuditLogQueryParameters,
    BACnetAuditLogRecord, BACnetAuditLogRecordResult, BACnetAuditNotification, BACnetRecipient,
};
use bacnet_types::enums::{AuditOperation, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bacnet_types::MacAddr;

use crate::traits::BACnetObject;

use super::{AuditLogObject, AuditLogPersistence, AuditLogSnapshot, MAX_AUDIT_RECORDS};

#[derive(Default)]
struct MemoryPersistence(Mutex<Option<AuditLogSnapshot>>);

impl MemoryPersistence {
    fn with_snapshot(snapshot: AuditLogSnapshot) -> Self {
        Self(Mutex::new(Some(snapshot)))
    }
}

impl AuditLogPersistence for MemoryPersistence {
    fn load(&self, _expected_object: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

fn device(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

fn object(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn address(network_number: u16, mac: &[u8]) -> BACnetAddress {
    BACnetAddress {
        network_number,
        mac_address: MacAddr::from_slice(mac),
    }
}

fn record(sequence_number: u64, datum: BACnetAuditLogDatum) -> BACnetAuditLogRecordResult {
    BACnetAuditLogRecordResult {
        sequence_number,
        record: BACnetAuditLogRecord {
            timestamp: (
                Date {
                    year: 124,
                    month: 2,
                    day: 29,
                    day_of_week: 4,
                },
                Time {
                    hour: 12,
                    minute: 0,
                    second: 0,
                    hundredths: 0,
                },
            ),
            datum,
        },
    }
}

fn notification(
    source_device: BACnetRecipient,
    target_device: BACnetRecipient,
) -> BACnetAuditNotification {
    BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: None,
        source_device,
        source_object: Some(object(ObjectType::ANALOG_INPUT, 10)),
        operation: AuditOperation::WRITE,
        source_comment: None,
        target_comment: None,
        invoke_id: None,
        source_user_id: None,
        source_user_role: None,
        target_device,
        target_object: Some(object(ObjectType::ANALOG_VALUE, 20)),
        target_property: Some(AuditPropertyReference {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: Some(3),
        }),
        target_priority: Some(8),
        target_value: None,
        current_value: None,
        result: None,
    }
}

fn log_with_records(records: Vec<BACnetAuditLogRecordResult>) -> AuditLogObject {
    let total_record_count = records.last().map_or(0, |entry| entry.sequence_number);
    let capacity = u32::try_from(records.len()).unwrap();
    let persistence = Arc::new(MemoryPersistence::with_snapshot(AuditLogSnapshot {
        object_identifier: object(ObjectType::AUDIT_LOG, 1),
        generation: 1,
        capacity,
        log_enable: true,
        total_record_count,
        records,
    }));
    AuditLogObject::new(1, "Audit-1", capacity, persistence).unwrap()
}

fn query(
    log: &AuditLogObject,
    parameters: &BACnetAuditLogQueryParameters,
    start: Option<u32>,
    count: u16,
) -> super::AuditLogQueryPage {
    BACnetObject::audit_log_storage_internal(log)
        .unwrap()
        .query(parameters, start, count)
}

fn target_query() -> BACnetAuditLogQueryParameters {
    let mut operations = AuditOperationFlags::empty();
    assert!(operations.insert(AuditOperation::WRITE));
    BACnetAuditLogQueryParameters::ByTarget {
        target_device_identifier: device(2),
        target_device_address: Some(address(7, &[0x22])),
        target_object_identifier: Some(object(ObjectType::ANALOG_VALUE, 20)),
        target_property_identifier: Some(PropertyIdentifier::PRESENT_VALUE),
        target_array_index: Some(3),
        target_priority: Some(8),
        operations: Some(operations),
        successful_actions_only: true,
    }
}

#[test]
fn target_filter_matches_every_field_and_device_identifier_or_address() {
    let by_identifier = notification(
        BACnetRecipient::Device(device(1)),
        BACnetRecipient::Device(device(2)),
    );
    let by_address = notification(
        BACnetRecipient::Device(device(1)),
        BACnetRecipient::Address(address(7, &[0x22])),
    );
    let log = log_with_records(vec![
        record(1, BACnetAuditLogDatum::LogStatus(0)),
        record(2, BACnetAuditLogDatum::TimeChange(1.5)),
        record(3, BACnetAuditLogDatum::AuditNotification(by_identifier)),
        record(4, BACnetAuditLogDatum::AuditNotification(by_address)),
    ]);

    let page = query(&log, &target_query(), None, 10);
    assert_eq!(
        page.records
            .iter()
            .map(|entry| entry.sequence_number)
            .collect::<Vec<_>>(),
        vec![4, 3]
    );
    assert!(page.no_more_items);

    // The index comparison is independent: the service does not reject or
    // reinterpret a supplied array index merely because the property filter
    // itself was omitted.
    let index_without_property = BACnetAuditLogQueryParameters::ByTarget {
        target_device_identifier: device(2),
        target_device_address: Some(address(7, &[0x22])),
        target_object_identifier: None,
        target_property_identifier: None,
        target_array_index: Some(3),
        target_priority: None,
        operations: None,
        successful_actions_only: false,
    };
    assert_eq!(
        query(&log, &index_without_property, None, 10).records.len(),
        2
    );

    let BACnetAuditLogQueryParameters::ByTarget {
        target_device_identifier,
        target_device_address,
        target_object_identifier,
        target_property_identifier,
        target_array_index,
        target_priority,
        operations,
        successful_actions_only,
    } = target_query()
    else {
        unreachable!();
    };

    let mismatches = [
        BACnetAuditLogQueryParameters::ByTarget {
            target_device_identifier: device(99),
            target_device_address: Some(address(8, &[0x99])),
            target_object_identifier,
            target_property_identifier,
            target_array_index,
            target_priority,
            operations,
            successful_actions_only,
        },
        BACnetAuditLogQueryParameters::ByTarget {
            target_device_identifier,
            target_device_address: target_device_address.clone(),
            target_object_identifier: Some(object(ObjectType::ANALOG_VALUE, 99)),
            target_property_identifier,
            target_array_index,
            target_priority,
            operations,
            successful_actions_only,
        },
        BACnetAuditLogQueryParameters::ByTarget {
            target_device_identifier,
            target_device_address: target_device_address.clone(),
            target_object_identifier,
            target_property_identifier: Some(PropertyIdentifier::DESCRIPTION),
            target_array_index,
            target_priority,
            operations,
            successful_actions_only,
        },
        BACnetAuditLogQueryParameters::ByTarget {
            target_device_identifier,
            target_device_address: target_device_address.clone(),
            target_object_identifier,
            target_property_identifier,
            target_array_index: Some(4),
            target_priority,
            operations,
            successful_actions_only,
        },
        BACnetAuditLogQueryParameters::ByTarget {
            target_device_identifier,
            target_device_address: target_device_address.clone(),
            target_object_identifier,
            target_property_identifier,
            target_array_index,
            target_priority: Some(9),
            operations,
            successful_actions_only,
        },
        BACnetAuditLogQueryParameters::ByTarget {
            target_device_identifier,
            target_device_address,
            target_object_identifier,
            target_property_identifier,
            target_array_index,
            target_priority,
            operations: Some(AuditOperationFlags::empty()),
            successful_actions_only,
        },
    ];
    for mismatch in mismatches {
        assert!(query(&log, &mismatch, None, 10).records.is_empty());
    }
}

#[test]
fn target_optional_values_are_wildcards_and_absent_record_priority_matches() {
    let mut audit = notification(
        BACnetRecipient::Device(device(1)),
        BACnetRecipient::Device(device(2)),
    );
    audit.target_object = None;
    audit.target_property = None;
    audit.target_priority = None;
    let log = log_with_records(vec![record(
        1,
        BACnetAuditLogDatum::AuditNotification(audit),
    )]);

    let wildcard = BACnetAuditLogQueryParameters::ByTarget {
        target_device_identifier: device(2),
        target_device_address: None,
        target_object_identifier: None,
        target_property_identifier: None,
        target_array_index: None,
        target_priority: Some(16),
        operations: None,
        successful_actions_only: true,
    };
    assert_eq!(query(&log, &wildcard, None, 1).records.len(), 1);
}

#[test]
fn source_filter_and_success_boolean_follow_the_wire_contract() {
    let source_address = address(5, &[0x11]);
    let success = notification(
        BACnetRecipient::Address(source_address.clone()),
        BACnetRecipient::Device(device(2)),
    );
    let mut failure = success.clone();
    failure.result = Some((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED));
    let log = log_with_records(vec![
        record(1, BACnetAuditLogDatum::AuditNotification(success)),
        record(2, BACnetAuditLogDatum::AuditNotification(failure)),
    ]);
    let mut operations = AuditOperationFlags::empty();
    assert!(operations.insert(AuditOperation::WRITE));

    let query_parameters = |successful_actions_only| BACnetAuditLogQueryParameters::BySource {
        source_device_identifier: device(99),
        source_device_address: Some(source_address.clone()),
        source_object_identifier: Some(object(ObjectType::ANALOG_INPUT, 10)),
        operations: Some(operations),
        successful_actions_only,
    };
    assert_eq!(
        query(&log, &query_parameters(true), None, 10)
            .records
            .iter()
            .map(|entry| entry.sequence_number)
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert_eq!(
        query(&log, &query_parameters(false), None, 10)
            .records
            .iter()
            .map(|entry| entry.sequence_number)
            .collect::<Vec<_>>(),
        vec![2, 1]
    );

    let wrong_source = BACnetAuditLogQueryParameters::BySource {
        source_device_identifier: device(98),
        source_device_address: Some(address(6, &[0x66])),
        source_object_identifier: None,
        operations: None,
        successful_actions_only: false,
    };
    assert!(query(&log, &wrong_source, None, 10).records.is_empty());

    let wrong_source_object = BACnetAuditLogQueryParameters::BySource {
        source_device_identifier: device(99),
        source_device_address: Some(source_address),
        source_object_identifier: Some(object(ObjectType::ANALOG_INPUT, 99)),
        operations: Some(operations),
        successful_actions_only: false,
    };
    assert!(query(&log, &wrong_source_object, None, 10)
        .records
        .is_empty());
}

#[test]
fn query_uses_insertion_order_literal_start_and_complete_scan_for_no_more_items() {
    let audit = notification(
        BACnetRecipient::Device(device(1)),
        BACnetRecipient::Device(device(2)),
    );
    let log = log_with_records(vec![
        record(
            u64::MAX,
            BACnetAuditLogDatum::AuditNotification(audit.clone()),
        ),
        record(1, BACnetAuditLogDatum::AuditNotification(audit)),
    ]);
    let query_parameters = BACnetAuditLogQueryParameters::ByTarget {
        target_device_identifier: device(2),
        target_device_address: None,
        target_object_identifier: None,
        target_property_identifier: None,
        target_array_index: None,
        target_priority: None,
        operations: None,
        successful_actions_only: false,
    };

    let all = query(&log, &query_parameters, None, 10);
    assert_eq!(
        all.records
            .iter()
            .map(|entry| entry.sequence_number)
            .collect::<Vec<_>>(),
        vec![1, u64::MAX]
    );
    assert!(all.no_more_items);

    let after_literal_start = query(&log, &query_parameters, Some(2), 10);
    assert_eq!(after_literal_start.records[0].sequence_number, 1);
    assert_eq!(after_literal_start.records.len(), 1);
    assert!(after_literal_start.no_more_items);

    let truncated = query(&log, &query_parameters, None, 1);
    assert_eq!(truncated.records[0].sequence_number, 1);
    assert!(!truncated.no_more_items);

    let zero = query(&log, &query_parameters, None, 0);
    assert!(zero.records.is_empty());
    assert!(!zero.no_more_items);

    let mut no_match = query_parameters.clone();
    let BACnetAuditLogQueryParameters::ByTarget {
        target_device_identifier,
        ..
    } = &mut no_match
    else {
        unreachable!();
    };
    *target_device_identifier = device(99);
    let zero_without_match = query(&log, &no_match, None, 0);
    assert!(zero_without_match.records.is_empty());
    assert!(zero_without_match.no_more_items);
}

#[test]
fn query_observes_retained_ring_eviction_and_newest_first_order() {
    let audit = notification(
        BACnetRecipient::Device(device(1)),
        BACnetRecipient::Device(device(2)),
    );
    let mut log = log_with_records(vec![
        record(1, BACnetAuditLogDatum::AuditNotification(audit.clone())),
        record(2, BACnetAuditLogDatum::AuditNotification(audit.clone())),
    ]);
    let newest = record(3, BACnetAuditLogDatum::AuditNotification(audit)).record;
    assert_eq!(log.add_record(newest).unwrap(), Some(3));

    let query_parameters = BACnetAuditLogQueryParameters::BySource {
        source_device_identifier: device(1),
        source_device_address: None,
        source_object_identifier: None,
        operations: None,
        successful_actions_only: false,
    };
    let page = query(&log, &query_parameters, None, 10);

    assert_eq!(
        page.records
            .iter()
            .map(|entry| entry.sequence_number)
            .collect::<Vec<_>>(),
        vec![3, 2]
    );
    assert!(page.no_more_items);
}

#[test]
fn query_never_returns_more_than_the_retained_storage_cap() {
    let audit = notification(
        BACnetRecipient::Device(device(1)),
        BACnetRecipient::Device(device(2)),
    );
    let records = (1..=MAX_AUDIT_RECORDS)
        .map(|sequence_number| {
            record(
                u64::from(sequence_number),
                BACnetAuditLogDatum::AuditNotification(audit.clone()),
            )
        })
        .collect();
    let log = log_with_records(records);
    let query_parameters = BACnetAuditLogQueryParameters::BySource {
        source_device_identifier: device(1),
        source_device_address: None,
        source_object_identifier: None,
        operations: None,
        successful_actions_only: false,
    };

    let page = query(&log, &query_parameters, None, u16::MAX);
    assert_eq!(page.records.len(), MAX_AUDIT_RECORDS as usize);
    assert_eq!(page.records[0].sequence_number, MAX_AUDIT_RECORDS as u64);
    assert_eq!(page.records.last().unwrap().sequence_number, 1);
    assert!(page.no_more_items);
}

#[test]
fn non_audit_objects_keep_the_default_capability_absent() {
    let reporter = super::AuditReporterObject::new(1, "Reporter").unwrap();
    assert!(reporter.audit_log_storage_internal().is_none());
}
