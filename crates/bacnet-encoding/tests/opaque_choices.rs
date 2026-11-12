//! A choice under a context tag its CHOICE doesn't assign (#2474, #2483):
//! event values and every log record CHOICE keep it as their `Opaque`
//! variant, primitive or constructed, write it back octet for octet, and
//! still refuse one that isn't well formed. The Audit Log datum and the
//! Event_Parameters tags Clause 21 deprecates and reserves are in the
//! submodules.
//!
//! The generated bodies are runs of complete tagged values: application
//! values, context primitives of any contents, and constructed values
//! nested a few deep, each under any tag the tag decoder takes (0 to 254).

// The BytesMut encoders and the owned decoders need alloc.
#![cfg(feature = "alloc")]

use bacnet_encoding::constructed::{
    decode_event_log_record, decode_event_notification, decode_event_notification_tolerant,
    decode_log_multiple_record, decode_log_record, decode_notification_parameters,
    encode_event_log_record, encode_event_notification, encode_log_multiple_record,
    encode_log_record, encode_notification_parameters,
};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_encoding::tags::{
    MAX_CONTEXT_NESTING_DEPTH, TagClass, encode_closing_tag, encode_opening_tag, encode_tag,
};
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, EventLogDatum,
    EventNotificationRequest, LogData, LogDatum, LogValue, NotificationParameters, OpaqueChoice,
};
use bacnet_types::enums::{
    ErrorClass, ErrorCode, EventState, EventType, NotifyType, ObjectType, RejectReason,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{
    BACnetTimeStamp, Date, ObjectIdentifier, PropertyValue, StatusFlags, Time,
};
use bytes::BytesMut;
use proptest::prelude::*;

#[path = "opaque_choices/audit_log.rs"]
mod audit_log;
#[path = "opaque_choices/event_parameters.rs"]
mod event_parameters;

// ---------------------------------------------------------------------------
// Well-formed bodies
// ---------------------------------------------------------------------------

/// One complete tagged value.
#[derive(Debug, Clone)]
enum Item {
    Application(PropertyValue),
    Primitive(u8, Vec<u8>),
    Constructed(u8, Vec<Item>),
}

fn item() -> impl Strategy<Value = Item> {
    let leaf = prop_oneof![
        any::<u64>().prop_map(|v| Item::Application(PropertyValue::Unsigned(v))),
        any::<i32>().prop_map(|v| Item::Application(PropertyValue::Signed(v))),
        any::<f32>().prop_map(|v| Item::Application(PropertyValue::Real(v))),
        any::<bool>().prop_map(|v| Item::Application(PropertyValue::Boolean(v))),
        Just(Item::Application(PropertyValue::Null)),
        "[ -~]{0,8}".prop_map(|v| Item::Application(PropertyValue::CharacterString(v))),
        prop::collection::vec(any::<u8>(), 0..6)
            .prop_map(|v| Item::Application(PropertyValue::OctetString(v))),
        (0u8..=254, prop::collection::vec(any::<u8>(), 0..6))
            .prop_map(|(tag, contents)| Item::Primitive(tag, contents)),
    ];
    leaf.prop_recursive(4, 24, 4, |inner| {
        (0u8..=254, prop::collection::vec(inner, 0..4))
            .prop_map(|(tag, items)| Item::Constructed(tag, items))
    })
}

fn write(buf: &mut BytesMut, items: &[Item]) {
    for item in items {
        match item {
            Item::Application(value) => encode_property_value(buf, value).unwrap(),
            Item::Primitive(tag, contents) => {
                encode_tag(buf, *tag, TagClass::Context, contents.len() as u32);
                buf.extend_from_slice(contents);
            }
            Item::Constructed(tag, items) => {
                encode_opening_tag(buf, *tag);
                write(buf, items);
                encode_closing_tag(buf, *tag);
            }
        }
    }
}

/// A constructed alternative's body: a run of complete tagged values.
fn body() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(item(), 0..5).prop_map(|items| {
        let mut buf = BytesMut::new();
        write(&mut buf, &items);
        buf.to_vec()
    })
}

/// An alternative under a tag `tags` gives: primitive with any contents,
/// or constructed around a body.
fn opaque_under(tags: impl Strategy<Value = u8>) -> impl Strategy<Value = OpaqueChoice> {
    (
        tags,
        any::<bool>(),
        body(),
        prop::collection::vec(any::<u8>(), 0..12),
    )
        .prop_map(|(tag, constructed, body, contents)| OpaqueChoice {
            tag,
            constructed,
            data: if constructed { body } else { contents },
        })
}

/// An alternative under a tag above `last`, the highest its CHOICE
/// assigns.
fn opaque(last: u8) -> impl Strategy<Value = OpaqueChoice> {
    opaque_under(last + 1..=254)
}

/// A tag event values keep opaque: 7 and 12, which Clause 21 deprecates and
/// reserves, or one past the last it assigns.
fn opaque_tag() -> impl Strategy<Value = u8> {
    prop_oneof![Just(7u8), Just(12u8), 23u8..=254]
}

/// Event values under a tag they keep opaque, in either form (#2483).
fn opaque_values() -> impl Strategy<Value = NotificationParameters> {
    opaque_under(opaque_tag()).prop_map(NotificationParameters::Opaque)
}

/// A constructed alternative under `tag` around `data`.
fn constructed(tag: u8, data: &[u8]) -> OpaqueChoice {
    OpaqueChoice {
        tag,
        constructed: true,
        data: data.to_vec(),
    }
}

/// A primitive alternative under `tag` holding `data`.
fn primitive(tag: u8, data: &[u8]) -> OpaqueChoice {
    OpaqueChoice {
        tag,
        constructed: false,
        data: data.to_vec(),
    }
}

/// The wire form of `opaque`, built here rather than by the codec.
fn wire(opaque: &OpaqueChoice) -> Vec<u8> {
    let mut buf = BytesMut::new();
    if opaque.constructed {
        encode_opening_tag(&mut buf, opaque.tag);
        buf.extend_from_slice(&opaque.data);
        encode_closing_tag(&mut buf, opaque.tag);
    } else {
        encode_tag(
            &mut buf,
            opaque.tag,
            TagClass::Context,
            opaque.data.len() as u32,
        );
        buf.extend_from_slice(&opaque.data);
    }
    buf.to_vec()
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn date() -> Date {
    Date {
        year: 126,
        month: 11,
        day: 11,
        day_of_week: 3,
    }
}

fn time() -> Time {
    Time {
        hour: 8,
        minute: 30,
        second: 0,
        hundredths: 0,
    }
}

/// The record timestamp field: context 0 around the application Date and
/// Time.
fn stamp() -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_opening_tag(&mut buf, 0);
    encode_property_value(&mut buf, &PropertyValue::Date(date())).unwrap();
    encode_property_value(&mut buf, &PropertyValue::Time(time())).unwrap();
    encode_closing_tag(&mut buf, 0);
    buf.to_vec()
}

/// `datum` wrapped as a record's field 1, after the timestamp.
fn record_wire(datum: &[u8]) -> Vec<u8> {
    let mut buf = BytesMut::from(&stamp()[..]);
    encode_opening_tag(&mut buf, 1);
    buf.extend_from_slice(datum);
    encode_closing_tag(&mut buf, 1);
    buf.to_vec()
}

fn notification(event_values: Option<NotificationParameters>) -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier: 7,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap(),
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(42),
        notification_class: 5,
        priority: 100,
        event_type: EventType::from_raw(30),
        message_text: Some("later revision".into()),
        notify_type: NotifyType::ALARM,
        ack_required: true,
        from_state: EventState::NORMAL,
        to_state: EventState::OFFNORMAL,
        event_values,
    }
}

fn encoded(encode: impl FnOnce(&mut BytesMut) -> Result<(), Error>) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode(&mut buf).unwrap();
    buf.to_vec()
}

// ---------------------------------------------------------------------------
// Round trips over generated alternatives
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        // Failures shrink in the run's output; none is saved as a file.
        failure_persistence: None,
        ..ProptestConfig::with_cases(256)
    })]

    /// Event values under an unassigned tag, primitive or constructed
    /// (#2483), alone and in a request: the octets the codec writes are the
    /// alternative as it came, they decode to the same value, alone and
    /// closed by the event values' `[12]`, and the rest of the request
    /// decodes as it was.
    #[test]
    fn event_values_keep_an_unassigned_alternative(values in opaque_values()) {
        let octets = encoded(|buf| encode_notification_parameters(&values, buf));
        let NotificationParameters::Opaque(alone) = &values else { unreachable!() };
        prop_assert_eq!(&octets, &wire(alone));
        prop_assert_eq!(decode_notification_parameters(&octets, 0).unwrap(), values.clone());
        let enclosed = [&octets[..], &[0xCF]].concat();
        prop_assert_eq!(decode_notification_parameters(&enclosed, 0).unwrap(), values.clone());

        let request = notification(Some(values));
        let octets = encoded(|buf| encode_event_notification(&request, buf));
        let decoded = decode_event_notification(&octets).unwrap();
        prop_assert_eq!(&decoded, &request);
        prop_assert_eq!(&decode_event_notification_tolerant(&octets).unwrap(), &request);
        prop_assert_eq!(encoded(|buf| encode_event_notification(&decoded, buf)), octets);
    }

    /// A Trend Log record's datum under an unassigned tag, with or without
    /// the status flags after it.
    #[test]
    fn trend_log_datum_round_trips(opaque in opaque(10), flags in any::<bool>()) {
        let record = BACnetLogRecord {
            date: date(),
            time: time(),
            log_datum: LogDatum::Opaque(opaque.clone()),
            status_flags: flags.then_some(StatusFlags::FAULT),
        };
        let octets = encoded(|buf| encode_log_record(&record, buf));
        prop_assert!(octets.starts_with(&record_wire(&wire(&opaque))));
        let (decoded, end) = decode_log_record(&octets, 0).unwrap();
        prop_assert_eq!(end, octets.len());
        prop_assert_eq!(&decoded, &record);
        prop_assert_eq!(encoded(|buf| encode_log_record(&decoded, buf)), octets);
    }

    /// A Trend Log Multiple record's log data under an unassigned tag, and
    /// member values under one at any place among known values.
    #[test]
    fn trend_log_multiple_data_and_members_round_trip(
        data in opaque(2),
        members in prop::collection::vec(opaque(8), 1..4),
        known in prop::collection::vec(any::<u32>(), 0..4),
        seed in any::<u64>(),
    ) {
        let record = BACnetLogMultipleRecord {
            date: date(),
            time: time(),
            log_data: LogData::Opaque(data.clone()),
        };
        let octets = encoded(|buf| encode_log_multiple_record(&record, buf));
        prop_assert_eq!(&octets, &record_wire(&wire(&data)));
        prop_assert_eq!(decode_log_multiple_record(&octets, 0).unwrap(), (record, octets.len()));

        // Interleave the unknown members with known ones, by the seed.
        let mut values: Vec<LogValue> = known.iter().map(|&v| LogValue::UnsignedValue(v.into())).collect();
        for (i, member) in members.into_iter().enumerate() {
            let at = usize::try_from(seed.rotate_left(i as u32 * 7) % (values.len() as u64 + 1)).unwrap();
            values.insert(at, LogValue::Opaque(member));
        }
        let record = BACnetLogMultipleRecord {
            date: date(),
            time: time(),
            log_data: LogData::Values(values),
        };
        let octets = encoded(|buf| encode_log_multiple_record(&record, buf));
        let (decoded, end) = decode_log_multiple_record(&octets, 0).unwrap();
        prop_assert_eq!(end, octets.len());
        prop_assert_eq!(&decoded, &record);
        prop_assert_eq!(encoded(|buf| encode_log_multiple_record(&decoded, buf)), octets);
    }

    /// An Event Log record's datum under an unassigned tag, and a logged
    /// notification whose event values are under one.
    #[test]
    fn event_log_datum_round_trips(opaque in opaque(2), values in opaque_values()) {
        let record = BACnetEventLogRecord {
            date: date(),
            time: time(),
            log_datum: EventLogDatum::Opaque(opaque.clone()),
        };
        let octets = encoded(|buf| encode_event_log_record(&record, buf));
        prop_assert_eq!(&octets, &record_wire(&wire(&opaque)));
        prop_assert_eq!(decode_event_log_record(&octets, 0).unwrap(), (record, octets.len()));

        let record = BACnetEventLogRecord {
            date: date(),
            time: time(),
            log_datum: EventLogDatum::Notification(notification(Some(values))),
        };
        let octets = encoded(|buf| encode_event_log_record(&record, buf));
        let (decoded, end) = decode_event_log_record(&octets, 0).unwrap();
        prop_assert_eq!(end, octets.len());
        prop_assert_eq!(&decoded, &record);
        prop_assert_eq!(encoded(|buf| encode_event_log_record(&decoded, buf)), octets);
    }
}

/// `[207]`'s closing tag, `FF CF`, ends in the octet of a closing `[12]`:
/// the standalone decoder strips an enclosing `[12]` only after the
/// choice's own closing tag.
#[test]
fn a_closing_tag_ending_like_the_event_values_close_stays_in_the_choice() {
    let values = NotificationParameters::Opaque(constructed(207, &[0x09, 1]));
    let octets = encoded(|buf| encode_notification_parameters(&values, buf));
    assert_eq!(octets, [0xFE, 207, 0x09, 1, 0xFF, 207]);
    assert_eq!(decode_notification_parameters(&octets, 0).unwrap(), values);
    let enclosed = [&octets[..], &[0xCF]].concat();
    assert_eq!(
        decode_notification_parameters(&enclosed, 0).unwrap(),
        values
    );
    // Anything else after the choice is refused, a second close included.
    for tail in [&[0xCF, 0xCF][..], &[0x09, 1]] {
        let octets = [&octets[..], tail].concat();
        assert!(
            decode_notification_parameters(&octets, 0).is_err(),
            "{tail:02X?}"
        );
    }

    // A [12] choice ends in the octet of the enclosing close itself, and
    // its body may hold a [12] of its own: each closing tag pairs with its
    // own opening, alone, enclosed and inside a request.
    for data in [
        vec![],
        vec![0x09, 1],
        vec![0xCE, 0xCF],
        vec![0xCE, 0x09, 1, 0xCF],
    ] {
        let values = NotificationParameters::Opaque(constructed(12, &data));
        let octets = encoded(|buf| encode_notification_parameters(&values, buf));
        let enclosed = [&octets[..], &[0xCF]].concat();
        for octets in [&octets, &enclosed] {
            assert_eq!(
                decode_notification_parameters(octets, 0).unwrap(),
                values,
                "{octets:02X?}"
            );
        }
        let request = notification(Some(values.clone()));
        let octets = encoded(|buf| encode_event_notification(&request, buf));
        assert_eq!(decode_event_notification(&octets).unwrap(), request);
    }
    // A primitive choice whose last octet reads as a closing [12] (#2483):
    // its length, not that octet, says where it ends, alone, enclosed and
    // inside a request; a stray octet or a close too many after it fails.
    for data in [vec![], vec![0xCF], vec![0x01, 0xCF]] {
        let values = NotificationParameters::Opaque(primitive(30, &data));
        let alone = encoded(|buf| encode_notification_parameters(&values, buf));
        let enclosed = [&alone[..], &[0xCF]].concat();
        for octets in [&alone, &enclosed] {
            assert_eq!(
                decode_notification_parameters(octets, 0).unwrap(),
                values,
                "{octets:02X?}"
            );
        }
        for tail in [&[0xCF, 0xCF][..], &[0x00]] {
            let octets = [&alone[..], tail].concat();
            assert!(
                decode_notification_parameters(&octets, 0).is_err(),
                "{octets:02X?}"
            );
        }
        let request = notification(Some(values));
        let octets = encoded(|buf| encode_event_notification(&request, buf));
        assert_eq!(decode_event_notification(&octets).unwrap(), request);
    }
    // One stray octet after a choice is refused, not taken for the close of
    // the event values: after a [12] choice, and after a known one.
    let known = encoded(|buf| {
        encode_notification_parameters(
            &NotificationParameters::ChangeOfBitstring {
                referenced_bitstring: (0, vec![0xA0]),
                status_flags: StatusFlags::empty(),
            },
            buf,
        )
    });
    for octets in [vec![0xCE, 0xCF, 0x00], [&known[..], &[0x00]].concat()] {
        assert!(
            decode_notification_parameters(&octets, 0).is_err(),
            "{octets:02X?}"
        );
    }
    // One close too many after the inner [12] is refused.
    for octets in [
        &[0xCE, 0xCE, 0xCF, 0xCF, 0xCF, 0xCF][..],
        &[0xCE, 0xCF, 0xCF, 0xCF],
    ] {
        assert!(
            decode_notification_parameters(octets, 0).is_err(),
            "{octets:02X?}"
        );
    }
}

/// Records back to back, as a ReadRange item list carries them: an unknown
/// datum between two known ones doesn't stop the records after it.
#[test]
fn an_unknown_datum_between_known_records_decodes_in_place() {
    let records = [
        LogDatum::RealValue(21.5),
        LogDatum::Opaque(OpaqueChoice {
            tag: 11,
            constructed: false,
            data: vec![0x40, 0x35, 0, 0, 0, 0, 0, 0],
        }),
        LogDatum::LogStatus(LogStatus::LOG_INTERRUPTED),
    ]
    .map(|log_datum| BACnetLogRecord {
        date: date(),
        time: time(),
        log_datum,
        status_flags: None,
    });
    let mut octets = BytesMut::new();
    for record in &records {
        encode_log_record(record, &mut octets).unwrap();
    }
    let mut offset = 0;
    for record in &records {
        let (decoded, end) = decode_log_record(&octets, offset).unwrap();
        assert_eq!(&decoded, record);
        offset = end;
    }
    assert_eq!(offset, octets.len());
}

// ---------------------------------------------------------------------------
// The boundary: well formed but unknown, against malformed
// ---------------------------------------------------------------------------

/// `values` as a request's event values, the octets between `[12]`'s tags.
fn request_with(values: &[u8]) -> Vec<u8> {
    let template = encoded(|buf| {
        encode_event_notification(
            &notification(Some(NotificationParameters::Opaque(constructed(30, &[])))),
            buf,
        )
    });
    // The template ends `[12] [30] [/30] [/12]`: the empty alternative
    // takes four octets (each tag numbered above 14 takes two), and the
    // closing `[12]` one.
    let head = &template[..template.len() - 5];
    [head, values, &[0xCF]].concat()
}

/// `n` opening tags `[tag]`, then the closing ones.
fn nested(tag: u8, n: usize) -> Vec<u8> {
    let mut buf = BytesMut::new();
    for _ in 0..n {
        encode_opening_tag(&mut buf, tag);
    }
    for _ in 0..n {
        encode_closing_tag(&mut buf, tag);
    }
    buf.to_vec()
}

fn reject_reason(result: Result<impl std::fmt::Debug, Error>) -> RejectReason {
    result.unwrap_err().reject_reason().unwrap()
}

#[test]
fn event_values_refuse_a_malformed_or_reserved_alternative() {
    // The well-formed base cases: an empty body under [30], and one nested
    // as deep as the request's own tags leave room for.
    let empty = [0xFE, 30, 0xFF, 30];
    assert!(decode_event_notification(&request_with(&empty)).is_ok());
    let deep = [
        &[0xFE, 30][..],
        &nested(1, MAX_CONTEXT_NESTING_DEPTH - 3),
        &[0xFF, 30],
    ]
    .concat();
    assert!(decode_event_notification(&request_with(&deep)).is_ok());

    for (values, reason) in [
        // Nested past the tag decoder's limit.
        (
            [
                &[0xFE, 30][..],
                &nested(1, MAX_CONTEXT_NESTING_DEPTH),
                &[0xFF, 30],
            ]
            .concat(),
            RejectReason::INVALID_TAG,
        ),
        // Closed by another tag.
        (vec![0xFE, 30, 0xFF, 31], RejectReason::INVALID_TAG),
        // A stray closing tag inside.
        (vec![0xFE, 30, 0x1F, 0xFF, 30], RejectReason::INVALID_TAG),
        // A context primitive inside that runs past the closing tag.
        (
            vec![0xFE, 30, 0x1C, 1, 2, 0xFF, 30],
            RejectReason::INVALID_TAG,
        ),
        // An application REAL of three octets inside.
        (
            vec![0xFE, 30, 0x43, 0, 0, 0, 0xFF, 30],
            RejectReason::INVALID_DATA_ENCODING,
        ),
        // A primitive alternative whose contents run past the close of the
        // event values, and one under a tag that has a variant, or 20.
        (vec![0xFB, 30, 1], RejectReason::MISSING_REQUIRED_PARAMETER),
        (vec![0x29, 1], RejectReason::INVALID_TAG),
        (vec![0xF9, 20, 1], RejectReason::INVALID_TAG),
        // Two primitive alternatives where one is due.
        (vec![0xF8, 30, 0xF8, 30], RejectReason::TOO_MANY_ARGUMENTS),
        // Two alternatives where one is due.
        (
            vec![0xFE, 30, 0xFF, 30, 0xFE, 30, 0xFF, 30],
            RejectReason::TOO_MANY_ARGUMENTS,
        ),
        // Tag 20, left out to match the event type that reports no values.
        (vec![0xFE, 20, 0xFF, 20], RejectReason::INVALID_TAG),
    ] {
        let request = request_with(&values);
        let what = format!("{values:02X?}");
        assert!(decode_event_notification(&request).is_err(), "{what}");
        assert!(
            decode_event_notification_tolerant(&request).is_err(),
            "{what}"
        );
        // As event values alone too, closed by [12].
        let alone = [&values[..], &[0xCF]].concat();
        assert!(decode_notification_parameters(&alone, 0).is_err(), "{what}");
        assert_eq!(
            reject_reason(decode_event_notification(&request)),
            reason,
            "{what}"
        );
    }
    // Tag 255, which no tag may use, primitive or constructed.
    for values in [&[0xF8, 0xFF][..], &[0xFE, 0xFF, 0xFF, 0xFF]] {
        assert!(decode_event_notification(&request_with(values)).is_err());
        let alone = [values, &[0xCF]].concat();
        assert!(decode_notification_parameters(&alone, 0).is_err());
    }
    // Tags 7 and 12, which Clause 21 deprecates and reserves, are kept, and
    // a primitive under a tag it doesn't assign (#2483), with its form.
    for (values, kept) in [
        (&[0x7E, 0x7F][..], constructed(7, &[])),
        (&[0xCE, 0x09, 1, 0xCF], constructed(12, &[0x09, 1])),
        (&[0x79, 1], primitive(7, &[1])),
        (&[0xC8], primitive(12, &[])),
        (&[0xF9, 30, 1], primitive(30, &[1])),
    ] {
        let request = decode_event_notification(&request_with(values)).unwrap();
        assert_eq!(
            request.event_values,
            Some(NotificationParameters::Opaque(kept)),
            "{values:02X?}"
        );
    }
}

#[test]
fn log_records_refuse_a_malformed_alternative_or_a_known_tag_in_the_wrong_form() {
    // Each datum, wrapped as a Trend Log record, an Event Log record and a
    // Trend Log Multiple record's log data, then as a member value.
    let well_formed: &[&[u8]] = &[
        // [11] primitive, empty and with contents; [12] constructed.
        &[0xB8],
        &[0xBA, 1, 2],
        &[0xCE, 0x09, 1, 0xCF],
        // [254] around a nested body.
        &[0xFE, 254, 0x0E, 0x0F, 0xFF, 254],
    ];
    for datum in well_formed {
        let what = format!("{datum:02X?}");
        let record = record_wire(datum);
        assert!(decode_log_record(&record, 0).is_ok(), "{what}");
        assert!(decode_event_log_record(&record, 0).is_ok(), "{what}");
        assert!(decode_log_multiple_record(&record, 0).is_ok(), "{what}");
        let member = record_wire(&[&[0x1E][..], datum, &[0x1F]].concat());
        assert!(decode_log_multiple_record(&member, 0).is_ok(), "{what}");
    }

    let malformed: &[&[u8]] = &[
        // A primitive whose length runs past the field.
        &[0xBA, 1],
        // Closed by another tag, or not at all.
        &[0xCE, 0xDF],
        &[0xCE, 0x09, 1],
        // A stray closing tag, a value running past the closing tag, and an
        // application REAL of three octets, inside.
        &[0xCE, 0x1F, 0xCF],
        &[0xCE, 0x44, 0, 0xCF],
        &[0xCE, 0x43, 0, 0, 0, 0xCF],
        // An application value where the CHOICE is due.
        &[0x21, 1],
        // A second alternative after the first.
        &[0xB8, 0xB8],
        // Tag 255, which no tag may use, primitive or constructed.
        &[0xF8, 0xFF],
        &[0xFE, 0xFF, 0xFF, 0xFF],
    ];
    for datum in malformed {
        let what = format!("{datum:02X?}");
        let record = record_wire(datum);
        assert!(decode_log_record(&record, 0).is_err(), "{what}");
        assert!(decode_event_log_record(&record, 0).is_err(), "{what}");
        assert!(decode_log_multiple_record(&record, 0).is_err(), "{what}");
        // As member values, where two in a row are two members.
        if *datum != [0xB8, 0xB8] {
            let member = record_wire(&[&[0x1E][..], datum, &[0x1F]].concat());
            assert!(decode_log_multiple_record(&member, 0).is_err(), "{what}");
        }
    }
    let deep = [&[0xCE][..], &nested(1, MAX_CONTEXT_NESTING_DEPTH), &[0xCF]].concat();
    assert!(decode_log_record(&record_wire(&deep), 0).is_err());
    assert!(decode_event_log_record(&record_wire(&deep), 0).is_err());

    // Known tags in the wrong form stay refused: a Trend Log's failure [8]
    // and any-value [10] are constructed, its boolean [1] primitive.
    for datum in [&[0x88][..], &[0xA8], &[0x1E, 0x1F]] {
        assert!(
            decode_log_record(&record_wire(datum), 0).is_err(),
            "{datum:02X?}"
        );
    }
    // Log data [1] is constructed, log status [0] and time change [2]
    // primitive, for a Trend Log Multiple and an Event Log alike.
    for datum in [&[0x18][..], &[0x0E, 0x0F], &[0x2E, 0x2F]] {
        assert!(
            decode_log_multiple_record(&record_wire(datum), 0).is_err(),
            "{datum:02X?}"
        );
        assert!(
            decode_event_log_record(&record_wire(datum), 0).is_err(),
            "{datum:02X?}"
        );
    }
    // A member's failure [7] and any-value [8] are constructed, its
    // boolean [0] primitive.
    for member in [&[0x78][..], &[0x88], &[0x0E, 0x0F]] {
        let record = record_wire(&[&[0x1E][..], member, &[0x1F]].concat());
        assert!(
            decode_log_multiple_record(&record, 0).is_err(),
            "{member:02X?}"
        );
    }
}

/// The encoders write an opaque alternative only under a tag its CHOICE
/// doesn't assign, only around a well-formed body, only as long as a tag
/// takes, and only in a record no deeper than a ReadRange-ACK carries,
/// leaving the buffer as it was when they refuse.
#[test]
fn encoders_refuse_an_opaque_alternative_they_could_not_read_back() {
    let refuses = |encode: &dyn Fn(&mut BytesMut) -> Result<(), Error>| {
        let mut buf = BytesMut::from(&[0xAA][..]);
        assert!(encode(&mut buf).is_err());
        assert_eq!(&buf[..], &[0xAA]);
    };
    let choice = |tag, constructed, data: &[u8]| OpaqueChoice {
        tag,
        constructed,
        data: data.to_vec(),
    };
    for form in [primitive, constructed] {
        for tag in [0, 6, 13, 20, 22, 255] {
            let values = NotificationParameters::Opaque(form(tag, &[]));
            refuses(&|buf| encode_notification_parameters(&values, buf));
            let request = notification(Some(values.clone()));
            refuses(&|buf| encode_event_notification(&request, buf));
        }
    }
    let values = NotificationParameters::Opaque(constructed(30, &[0x1F]));
    refuses(&|buf| encode_notification_parameters(&values, buf));
    // A primitive's contents may be anything, up to what a tag takes.
    let values = NotificationParameters::Opaque(primitive(30, &[0x1F]));
    assert!(encode_notification_parameters(&values, &mut BytesMut::new()).is_ok());
    let values = NotificationParameters::Opaque(primitive(30, &vec![0; (1 << 20) + 1]));
    refuses(&|buf| encode_notification_parameters(&values, buf));

    let trend = |opaque| BACnetLogRecord {
        date: date(),
        time: time(),
        log_datum: LogDatum::Opaque(opaque),
        status_flags: None,
    };
    let multiple = |log_data| BACnetLogMultipleRecord {
        date: date(),
        time: time(),
        log_data,
    };
    let event = |opaque| BACnetEventLogRecord {
        date: date(),
        time: time(),
        log_datum: EventLogDatum::Opaque(opaque),
    };
    for constructed in [false, true] {
        for tag in [0, 10, 255] {
            let record = trend(choice(tag, constructed, &[]));
            refuses(&|buf| encode_log_record(&record, buf));
        }
        for tag in [0, 2, 255] {
            let record = multiple(LogData::Opaque(choice(tag, constructed, &[])));
            refuses(&|buf| encode_log_multiple_record(&record, buf));
            let record = event(choice(tag, constructed, &[]));
            refuses(&|buf| encode_event_log_record(&record, buf));
        }
        for tag in [0, 8, 255] {
            let member = LogValue::Opaque(choice(tag, constructed, &[]));
            let record = multiple(LogData::Values(vec![LogValue::NullValue, member]));
            refuses(&|buf| encode_log_multiple_record(&record, buf));
        }
    }
    // A constructed body that isn't complete tagged values.
    let record = trend(choice(11, true, &[0x1F]));
    refuses(&|buf| encode_log_record(&record, buf));
    let record = event(choice(3, true, &[0x0E]));
    refuses(&|buf| encode_event_log_record(&record, buf));
    let member = LogValue::Opaque(choice(9, true, &[0x44, 0]));
    let record = multiple(LogData::Values(vec![member]));
    refuses(&|buf| encode_log_multiple_record(&record, buf));
    // The same body is fine as a primitive's contents, but not contents
    // longer than the tag decoder takes.
    let record = trend(choice(11, false, &[0x1F]));
    assert!(encode_log_record(&record, &mut BytesMut::new()).is_ok());
    let record = trend(choice(11, false, &vec![0; 1 << 20]));
    assert!(encode_log_record(&record, &mut BytesMut::new()).is_ok());
    let record = trend(choice(11, false, &vec![0; (1 << 20) + 1]));
    refuses(&|buf| encode_log_record(&record, buf));
    // A body one level deeper than a ReadRange-ACK carries: the record's
    // field 1 and the alternative's own tag take two levels.
    let deep = nested(1, MAX_CONTEXT_NESTING_DEPTH - 2);
    let record = trend(choice(11, true, &deep));
    refuses(&|buf| encode_log_record(&record, buf));
    let record = event(choice(3, true, &deep));
    refuses(&|buf| encode_event_log_record(&record, buf));
    // A member's own levels are three: one level less of the same body.
    let member = LogValue::Opaque(choice(9, true, &deep[1..deep.len() - 1]));
    let record = multiple(LogData::Values(vec![member]));
    refuses(&|buf| encode_log_multiple_record(&record, buf));
}

/// A member value kept opaque has no Trend Log datum.
#[test]
fn an_opaque_member_value_has_no_trend_log_datum() {
    let opaque = OpaqueChoice {
        tag: 9,
        constructed: false,
        data: vec![1],
    };
    assert_eq!(
        LogDatum::try_from(LogValue::Opaque(opaque.clone())),
        Err(opaque)
    );
    assert_eq!(
        LogDatum::try_from(LogValue::Failure {
            error_class: ErrorClass::PROPERTY,
            error_code: ErrorCode::UNKNOWN_PROPERTY,
        }),
        Ok(LogDatum::Failure {
            error_class: ErrorClass::PROPERTY,
            error_code: ErrorCode::UNKNOWN_PROPERTY,
        })
    );
}
