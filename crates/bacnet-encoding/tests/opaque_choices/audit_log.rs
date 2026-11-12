//! The Audit Log record's datum (#2483): an alternative under a tag above 2,
//! where its CHOICE assigns none, is kept as `BACnetAuditLogDatum::Opaque`
//! with the bounds the other log records' CHOICEs keep, alone and as an
//! AuditLogQuery result.

use super::*;
use bacnet_encoding::constructed::{
    decode_audit_log_record, decode_audit_log_record_at, decode_audit_log_record_result_at,
    encode_audit_log_record, encode_audit_log_record_result,
};
use bacnet_types::constructed::{
    BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditLogRecordResult,
};

fn audit(datum: BACnetAuditLogDatum) -> BACnetAuditLogRecord {
    BACnetAuditLogRecord {
        timestamp: (date(), time()),
        datum,
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::with_cases(256)
    })]

    /// The datum under an unassigned tag: the record is the timestamp and
    /// the alternative as it came inside field 1, it decodes to the same
    /// value alone, at an offset and as a query result, and encodes again
    /// to the same octets.
    #[test]
    fn audit_log_datum_round_trips(opaque in opaque(2), sequence_number in 1u64..) {
        let record = audit(BACnetAuditLogDatum::Opaque(opaque.clone()));
        let octets = encoded(|buf| encode_audit_log_record(&record, buf));
        prop_assert_eq!(&octets, &record_wire(&wire(&opaque)));
        prop_assert_eq!(&decode_audit_log_record(&octets).unwrap(), &record);
        prop_assert_eq!(decode_audit_log_record_at(&octets, 0).unwrap(), (record.clone(), octets.len()));
        prop_assert_eq!(
            encoded(|buf| encode_audit_log_record(&decode_audit_log_record(&octets).unwrap(), buf)),
            octets
        );

        let result = BACnetAuditLogRecordResult { sequence_number, record };
        let octets = encoded(|buf| encode_audit_log_record_result(&result, buf));
        prop_assert_eq!(decode_audit_log_record_result_at(&octets, 0).unwrap(), (result, octets.len()));
    }
}

/// Records back to back, as a ReadRange item list carries them: an unknown
/// datum between two known ones doesn't stop the records after it.
#[test]
fn an_unknown_audit_datum_between_known_records_decodes_in_place() {
    let records = [
        BACnetAuditLogDatum::TimeChange(-2.5),
        BACnetAuditLogDatum::Opaque(constructed(3, &[0x09, 0x01])),
        BACnetAuditLogDatum::Opaque(primitive(200, &[0xCF])),
        BACnetAuditLogDatum::LogStatus(LogStatus::BUFFER_PURGED),
    ]
    .map(audit);
    let mut octets = BytesMut::new();
    for record in &records {
        encode_audit_log_record(record, &mut octets).unwrap();
    }
    let mut offset = 0;
    for record in &records {
        let (decoded, end) = decode_audit_log_record_at(&octets, offset).unwrap();
        assert_eq!(&decoded, record);
        offset = end;
    }
    assert_eq!(offset, octets.len());
}

#[test]
fn audit_log_records_refuse_a_malformed_alternative_or_a_known_tag_in_the_wrong_form() {
    let well_formed: &[&[u8]] = &[
        // [3] primitive, empty and with contents; [12] constructed; [254]
        // around a nested body.
        &[0x38],
        &[0x3A, 1, 2],
        &[0xCE, 0x09, 1, 0xCF],
        &[0xFE, 254, 0x0E, 0x0F, 0xFF, 254],
    ];
    for datum in well_formed {
        assert!(
            decode_audit_log_record(&record_wire(datum)).is_ok(),
            "{datum:02X?}"
        );
    }
    let malformed: &[&[u8]] = &[
        // A primitive whose length runs past the field.
        &[0x3A, 1],
        // Closed by another tag, or not at all.
        &[0xCE, 0xDF],
        &[0xCE, 0x09, 1],
        // A stray closing tag, a value running past the closing tag, and an
        // application REAL of three octets, inside.
        &[0xCE, 0x1F, 0xCF],
        &[0xCE, 0x44, 0, 0xCF],
        &[0xCE, 0x43, 0, 0, 0, 0xCF],
        // An application value where the CHOICE is due, and a second
        // alternative after the first.
        &[0x21, 1],
        &[0x38, 0x38],
        // Tag 255, which no tag may use, primitive or constructed.
        &[0xF8, 0xFF],
        &[0xFE, 0xFF, 0xFF, 0xFF],
        // Known tags in the wrong form: log status [0] and time change [2]
        // are primitive, the notification [1] constructed.
        &[0x0E, 0x0F],
        &[0x18],
        &[0x2E, 0x2F],
    ];
    for datum in malformed {
        let record = record_wire(datum);
        assert!(decode_audit_log_record(&record).is_err(), "{datum:02X?}");
        assert!(
            decode_audit_log_record_at(&record, 0).is_err(),
            "{datum:02X?}"
        );
    }
    let deep = [&[0x3E][..], &nested(1, MAX_CONTEXT_NESTING_DEPTH), &[0x3F]].concat();
    assert!(decode_audit_log_record(&record_wire(&deep)).is_err());
}

/// The encoder writes an opaque datum only under a tag the CHOICE doesn't
/// assign, around a well-formed body and as long as a tag takes, leaving
/// the buffer as it was when it refuses.
#[test]
fn the_audit_encoder_refuses_an_opaque_datum_it_could_not_read_back() {
    let refuses = |datum: BACnetAuditLogDatum| {
        let record = audit(datum);
        let mut buf = BytesMut::from(&[0xAA][..]);
        assert!(
            encode_audit_log_record(&record, &mut buf).is_err(),
            "{record:?}"
        );
        assert_eq!(&buf[..], &[0xAA]);
        let result = BACnetAuditLogRecordResult {
            sequence_number: 1,
            record,
        };
        assert!(encode_audit_log_record_result(&result, &mut buf).is_err());
        assert_eq!(&buf[..], &[0xAA]);
    };
    for form in [primitive, constructed] {
        for tag in [0, 1, 2, 255] {
            refuses(BACnetAuditLogDatum::Opaque(form(tag, &[])));
        }
    }
    refuses(BACnetAuditLogDatum::Opaque(constructed(3, &[0x1F])));
    // A primitive's contents may be anything, up to what a tag takes.
    let fine = audit(BACnetAuditLogDatum::Opaque(primitive(3, &[0x1F])));
    assert!(encode_audit_log_record(&fine, &mut BytesMut::new()).is_ok());
    refuses(BACnetAuditLogDatum::Opaque(primitive(
        3,
        &vec![0; (1 << 20) + 1],
    )));
}
