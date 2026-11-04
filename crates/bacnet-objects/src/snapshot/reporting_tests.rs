//! The reporting settings kind of the written-configuration format (#2331):
//! the octets it writes and what it refuses. That the file backend stores
//! exactly what the public `encode` returns is in `tests.rs`, with the
//! other kinds.

use bacnet_encoding::constructed::encode_object_property_reference;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{NotifyType, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use super::{SnapshotErrorKind, write_config};
use crate::write_config::{ReportingWriteConfig, TimerWriteConfig, WriteConfigSnapshot};

fn oid(kind: ObjectType) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, 7).unwrap()
}

fn inhibit_reference() -> BACnetObjectPropertyReference {
    BACnetObjectPropertyReference::new(
        oid(ObjectType::BINARY_VALUE),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )
}

/// Every setting written.
fn every_setting() -> ReportingWriteConfig {
    ReportingWriteConfig {
        notification_class: Some(9),
        event_enable: Some(EventTransitionBits::TO_FAULT | EventTransitionBits::TO_NORMAL),
        notify_type: Some(NotifyType::ACK_NOTIFICATION),
        event_detection_enable: Some(false),
        event_message_texts_config: Some(["off".into(), String::new(), "back".into()]),
        event_algorithm_inhibit_ref: Some(inhibit_reference()),
        event_algorithm_inhibit: Some(true),
    }
}

/// The body after the header and the kind octet.
fn body(octets: &[u8]) -> &[u8] {
    &octets[13..]
}

/// Every object type that keeps these settings round-trips them, and the
/// body is laid out field by field as the format documents.
#[test]
fn the_reporting_kind_round_trips_in_the_documented_layout() {
    for kind in [
        ObjectType::ANALOG_INPUT,
        ObjectType::ANALOG_OUTPUT,
        ObjectType::ANALOG_VALUE,
        ObjectType::BINARY_INPUT,
        ObjectType::BINARY_OUTPUT,
        ObjectType::BINARY_VALUE,
        ObjectType::MULTI_STATE_INPUT,
        ObjectType::MULTI_STATE_OUTPUT,
        ObjectType::MULTI_STATE_VALUE,
    ] {
        assert!(write_config::keeps_reporting(kind), "{kind:?}");
        let snapshot = WriteConfigSnapshot::Reporting(every_setting());
        let octets = write_config::encode(oid(kind), &snapshot).unwrap();
        assert_eq!(octets[12], write_config::KIND_REPORTING);
        assert_eq!(write_config::decode(oid(kind), &octets), Ok(snapshot));
    }
    let octets = write_config::encode(
        oid(ObjectType::ANALOG_INPUT),
        &WriteConfigSnapshot::Reporting(every_setting()),
    )
    .unwrap();
    let mut reference = BytesMut::new();
    encode_object_property_reference(&mut reference, &inhibit_reference());
    let mut expected = vec![1, 0, 0, 0, 9, 1, 0b110, 1, 0, 0, 0, 2, 1, 0, 1];
    expected.extend_from_slice(&[0, 0, 0, 3]);
    expected.extend_from_slice(b"off");
    expected.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 4]);
    expected.extend_from_slice(b"back");
    expected.extend_from_slice(&[1, 0, 0, 0, reference.len() as u8]);
    expected.extend_from_slice(&reference);
    expected.extend_from_slice(&[1, 1]);
    assert_eq!(body(&octets), expected);
    // Nothing written is seven absent parts.
    let empty = write_config::encode(
        oid(ObjectType::BINARY_INPUT),
        &WriteConfigSnapshot::Reporting(ReportingWriteConfig::default()),
    )
    .unwrap();
    assert_eq!(body(&empty), [0; 7]);
}

/// The kind belongs to the object types that keep these settings, and those
/// keep no other kind.
#[test]
fn the_reporting_kind_and_the_object_type_must_match() {
    let reporting = WriteConfigSnapshot::Reporting(every_setting());
    for kind in [ObjectType::TIMER, ObjectType::CHANNEL, ObjectType::DEVICE] {
        let error = write_config::encode(oid(kind), &reporting).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange, "{kind:?}");
    }
    let timer = WriteConfigSnapshot::Timer(TimerWriteConfig::default());
    let error = write_config::encode(oid(ObjectType::ANALOG_INPUT), &timer).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange);
    // A reporting file relabelled for a Timer, and a Timer's kind in an
    // Analog Input's file, name another object type.
    let mut octets = write_config::encode(oid(ObjectType::ANALOG_INPUT), &reporting).unwrap();
    octets[8..12].copy_from_slice(&oid(ObjectType::TIMER).encode());
    let error = write_config::decode(oid(ObjectType::TIMER), &octets).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::OtherObject);
    let mut octets = write_config::encode(oid(ObjectType::ANALOG_INPUT), &reporting).unwrap();
    octets[12] = write_config::KIND_TIMER;
    let error = write_config::decode(oid(ObjectType::ANALOG_INPUT), &octets).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::OtherObject);
}

/// A file cut anywhere is refused, and so is one with octets past its end.
#[test]
fn every_cut_of_a_reporting_file_is_refused() {
    let object = oid(ObjectType::MULTI_STATE_VALUE);
    let octets =
        write_config::encode(object, &WriteConfigSnapshot::Reporting(every_setting())).unwrap();
    for cut in 0..octets.len() {
        assert!(
            write_config::decode(object, &octets[..cut]).is_err(),
            "a file cut at {cut} loaded"
        );
    }
    let mut longer = octets.clone();
    longer.push(0);
    let error = write_config::decode(object, &longer).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::Damaged);
}

/// A value no write leaves is damage when read, and never encodes.
#[test]
fn a_value_no_write_leaves_is_refused_both_ways() {
    let object = oid(ObjectType::ANALOG_VALUE);
    let octets =
        write_config::encode(object, &WriteConfigSnapshot::Reporting(every_setting())).unwrap();
    // Offsets into the body, which starts at octet 13.
    let cases: [(&str, usize, &[u8]); 6] = [
        ("a presence octet of 2", 0, &[2]),
        ("an unknown Event_Enable bit", 6, &[0b1000]),
        ("an unknown Notify_Type", 8, &[0, 0, 0, 3]),
        ("a non-Boolean detection flag", 13, &[2]),
        ("a message text that isn't UTF-8", 19, &[0xFF]),
        ("a message text past its cap", 15, &[0, 1, 0, 0]),
    ];
    for (what, offset, value) in cases {
        let mut patched = octets.clone();
        patched[13 + offset..13 + offset + value.len()].copy_from_slice(value);
        let error = write_config::decode(object, &patched).unwrap_err();
        assert!(error.is_damage(), "{what}: {error:?}");
    }
    let unset = BACnetObjectPropertyReference::new(
        ObjectIdentifier::new(ObjectType::BINARY_VALUE, 4_194_303).unwrap(),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    );
    for config in [
        ReportingWriteConfig {
            event_enable: Some(EventTransitionBits::from_bits_retain(0x08)),
            ..every_setting()
        },
        ReportingWriteConfig {
            notify_type: Some(NotifyType::from_raw(7)),
            ..every_setting()
        },
        ReportingWriteConfig {
            event_message_texts_config: Some([
                "x".repeat(16 * 1024 + 1),
                String::new(),
                String::new(),
            ]),
            ..every_setting()
        },
        ReportingWriteConfig {
            event_algorithm_inhibit_ref: Some(unset),
            ..every_setting()
        },
    ] {
        let error = write_config::encode(object, &WriteConfigSnapshot::Reporting(config.clone()))
            .unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange, "{config:?}");
    }
}
