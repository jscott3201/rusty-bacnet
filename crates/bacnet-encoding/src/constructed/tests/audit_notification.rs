//! BACnetAuditNotification object identifiers (#1303): the source and target
//! objects read like every other context-tagged object identifier, so a
//! wrong length is malformed even when the data also stops early, and four
//! octets cut short are a short buffer.

use crate::constructed::{
    decode_audit_log_record, decode_audit_log_record_result_at, decode_audit_notification_at,
    encode_audit_log_record, encode_audit_log_record_result, encode_audit_notification,
};
use bacnet_types::constructed::{
    AuditPropertyReference, BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditLogRecordResult,
    BACnetAuditNotification, BACnetRecipient,
};
use bacnet_types::enums::{AuditOperation, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, Time};
use bytes::BytesMut;

/// `[2]` source-device around a Device recipient, then `field`, which is
/// where the source-object `[3]` goes.
fn with_source_object(field: &[u8]) -> Vec<u8> {
    let mut data = vec![0x2E, 0x0C, 0x02, 0x00, 0x00, 0x01, 0x2F];
    data.extend_from_slice(field);
    data
}

#[test]
fn an_object_identifier_of_the_wrong_length_is_malformed_even_when_cut_short() {
    // [3] says three octets and holds one.
    let data = with_source_object(&[0x3B, 0x02]);
    match decode_audit_notification_at(&data, 0) {
        Err(Error::Decoding(error)) => {
            assert_eq!(error.offset, 7);
            assert_eq!(
                error.message().to_string(),
                "AuditNotification source-object: [3] object identifier has 3 contents \
                 octets, expected 4"
            );
        }
        other => panic!("expected a decoding error, got {other:?}"),
    }
}

#[test]
fn an_object_identifier_cut_short_is_a_short_buffer() {
    // [3] says four octets and holds two.
    let data = with_source_object(&[0x3C, 0x02, 0x00]);
    assert!(
        matches!(
            decode_audit_notification_at(&data, 0),
            Err(Error::BufferTooShort { need: 12, have: 10 })
        ),
        "{:?}",
        decode_audit_notification_at(&data, 0)
    );
}

/// A notification with every optional member present.
fn every_member() -> BACnetAuditNotification {
    let device = |instance| {
        BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap())
    };
    let value = |instance| ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap();
    let time = Time {
        hour: 1,
        minute: 2,
        second: 3,
        hundredths: 4,
    };
    BACnetAuditNotification {
        source_timestamp: Some(BACnetTimeStamp::SequenceNumber(500)),
        target_timestamp: Some(BACnetTimeStamp::Time(time)),
        source_device: device(1),
        source_object: Some(value(7)),
        operation: AuditOperation::WRITE,
        source_comment: Some("from".into()),
        target_comment: Some("to".into()),
        invoke_id: Some(9),
        source_user_id: Some(300),
        source_user_role: Some(2),
        target_device: device(2),
        target_object: Some(value(8)),
        target_property: Some(AuditPropertyReference {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: Some(3),
        }),
        target_priority: Some(8),
        target_value: Some(vec![0x44, 0x42, 0x48, 0x00, 0x00]),
        current_value: Some(vec![0x44, 0x41, 0x20, 0x00, 0x00]),
        result: Some((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)),
    }
}

#[test]
fn members_cut_short_are_a_short_buffer() {
    let mut notification = BytesMut::new();
    encode_audit_notification(&every_member(), &mut notification).unwrap();
    let framed =
        super::assert_members_cut_short("BACnetAuditNotification", &notification, |data| {
            decode_audit_notification_at(data, 0)
        });
    assert!(framed > 0);

    let date = Date {
        year: 126,
        month: 10,
        day: 4,
        day_of_week: 7,
    };
    let time = Time {
        hour: 12,
        minute: 0,
        second: 0,
        hundredths: 0,
    };
    for datum in [
        BACnetAuditLogDatum::AuditNotification(every_member()),
        BACnetAuditLogDatum::TimeChange(1.5),
    ] {
        let record = BACnetAuditLogRecord {
            timestamp: (date, time),
            datum,
        };
        let mut octets = BytesMut::new();
        encode_audit_log_record(&record, &mut octets).unwrap();
        super::assert_members_cut_short("BACnetAuditLogRecord", &octets, decode_audit_log_record);
        let result = BACnetAuditLogRecordResult {
            sequence_number: 70_000,
            record,
        };
        let mut octets = BytesMut::new();
        encode_audit_log_record_result(&result, &mut octets).unwrap();
        super::assert_members_cut_short("BACnetAuditLogRecordResult", &octets, |data| {
            decode_audit_log_record_result_at(data, 0)
        });
    }
}
