//! Values outside the log-datum datatypes are logged with the any-value
//! alternative, carrying the value's own encoding, not as NULL (#1236).

use super::*;
use crate::value_types::LargeAnalogValueObject;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_types::enums::EngineeringUnits;

#[test]
fn read_values_map_to_their_datum_or_an_any_value() {
    let date = Date {
        year: 124,
        month: 2,
        day: 29,
        day_of_week: 4,
    };
    for (value, expected) in [
        (PropertyValue::Real(42.5), LogDatum::RealValue(42.5)),
        (PropertyValue::Unsigned(100), LogDatum::UnsignedValue(100)),
        (PropertyValue::Signed(-12), LogDatum::SignedValue(-12)),
        (PropertyValue::Boolean(true), LogDatum::BooleanValue(true)),
        (PropertyValue::Enumerated(7), LogDatum::EnumValue(7)),
        (PropertyValue::Null, LogDatum::NullValue),
        (
            PropertyValue::BitString {
                unused_bits: 4,
                data: vec![0b1010_0000],
            },
            LogDatum::BitstringValue {
                unused_bits: 4,
                data: vec![0b1010_0000],
            },
        ),
        (
            PropertyValue::CharacterString("Hi".into()),
            LogDatum::AnyValue(vec![0x73, 0x00, b'H', b'i']),
        ),
        (
            PropertyValue::Double(1.0),
            LogDatum::AnyValue(vec![0x55, 0x08, 0x3F, 0xF0, 0, 0, 0, 0, 0, 0]),
        ),
        (
            PropertyValue::Date(date),
            LogDatum::AnyValue(vec![0xA4, 0x7C, 0x02, 0x1D, 0x04]),
        ),
        (
            PropertyValue::ObjectIdentifier(target()),
            LogDatum::AnyValue(vec![0xC4, 0x00, 0x80, 0x00, 0x01]),
        ),
        (
            PropertyValue::OctetString(vec![1, 2]),
            LogDatum::AnyValue(vec![0x62, 0x01, 0x02]),
        ),
        // A whole array, and a constructed value read as framed bytes.
        (
            PropertyValue::List(vec![PropertyValue::Unsigned(1), PropertyValue::Unsigned(2)]),
            LogDatum::AnyValue(vec![0x21, 0x01, 0x21, 0x02]),
        ),
        (
            PropertyValue::ApplicationData(vec![0x0E, 0x09, 0x01, 0x0F]),
            LogDatum::AnyValue(vec![0x0E, 0x09, 0x01, 0x0F]),
        ),
    ] {
        assert_eq!(
            LogDatum::try_from(property_value_to_log_value(&value, Record::Trend)).unwrap(),
            expected,
            "{value:?}"
        );
    }
}

/// An any-value holds at most `ANY_VALUE_MAX_OCTETS` of encoding; a longer
/// value logs PROPERTY / VALUE_TOO_LONG so its record stays pageable. A value
/// no record could carry logs SERVICES / OTHER, so the log never refuses the
/// poller's record.
#[test]
fn oversized_and_uncarriable_values_log_a_failure() {
    let unsigned = |count| PropertyValue::List(vec![PropertyValue::Unsigned(1); count]);
    let logged = |value: &PropertyValue| {
        LogDatum::try_from(property_value_to_log_value(value, Record::Trend)).unwrap()
    };
    // 128 two-octet elements fill the cap exactly; one more is too long.
    assert_eq!(ANY_VALUE_MAX_OCTETS, 256);
    assert_eq!(
        logged(&unsigned(128)),
        LogDatum::AnyValue([0x21, 0x01].repeat(128))
    );
    assert_eq!(
        logged(&unsigned(129)),
        failure(ErrorClass::PROPERTY, ErrorCode::VALUE_TOO_LONG)
    );
    for value in [
        // A stray closing tag, a constructed value cut short, and padding
        // no BIT STRING can have.
        PropertyValue::ApplicationData(vec![0x21, 0x01, 0x0F]),
        PropertyValue::ApplicationData(vec![0x0E, 0x21]),
        PropertyValue::BitString {
            unused_bits: 9,
            data: vec![0],
        },
    ] {
        assert_eq!(
            logged(&value),
            failure(ErrorClass::SERVICES, ErrorCode::OTHER),
            "{value:?}"
        );
    }
}

/// Each record kind probes a sampled value with its own record (#2474): a
/// Trend Log Multiple member sits a level deeper than a Trend Log datum, so
/// a value nested as deep as a Trend Log record leaves room for is logged
/// there, and the member logs SERVICES / OTHER; a level deeper, both do.
#[test]
fn each_record_kind_probes_with_its_own_record() {
    // A Trend Log datum takes two levels, field 1 and any-value [10], of
    // the 31 a ReadRange-ACK leaves a record; a member takes three.
    let nested = |n: usize| PropertyValue::ApplicationData([vec![0x1E; n], vec![0x1F; n]].concat());
    let other = LogValue::Failure {
        error_class: ErrorClass::SERVICES,
        error_code: ErrorCode::OTHER,
    };
    let fits = nested(29);
    let PropertyValue::ApplicationData(bytes) = &fits else {
        unreachable!()
    };
    assert_eq!(
        property_value_to_log_value(&fits, Record::Trend),
        LogValue::AnyValue(bytes.clone())
    );
    assert_eq!(property_value_to_log_value(&fits, Record::Multiple), other);
    let fits = nested(28);
    assert!(matches!(
        property_value_to_log_value(&fits, Record::Multiple),
        LogValue::AnyValue(_)
    ));
    assert_eq!(
        property_value_to_log_value(&nested(30), Record::Trend),
        other
    );
}

/// An object whose Present_Value is `n` empty constructed values, each
/// inside the last, standing in for a value a property could nest that
/// deep.
struct Nested {
    oid: ObjectIdentifier,
    depth: usize,
}

impl Nested {
    fn value(depth: usize) -> PropertyValue {
        PropertyValue::ApplicationData([vec![0x1E; depth], vec![0x1F; depth]].concat())
    }
}

impl BACnetObject for Nested {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "Nested"
    }

    fn read_property(&self, property: P, _: Option<u32>) -> Result<PropertyValue, Error> {
        match property {
            P::PRESENT_VALUE => Ok(Self::value(self.depth)),
            _ => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            )),
        }
    }

    fn write_property(
        &mut self,
        _: P,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }

    fn property_metadata(&self) -> std::borrow::Cow<'_, [PropertyMetadata]> {
        crate::test_rows::read_only(&[P::PRESENT_VALUE])
    }
}

/// The poller probes with the log's own record kind: a value nested as deep
/// as a Trend Log record leaves room for is logged as it is by a Trend Log,
/// and as SERVICES / OTHER by a Trend Log Multiple, whose member sits a
/// level deeper (#2474).
#[test]
fn the_poller_probes_with_the_logs_own_record() {
    let (mut db, oid, _, _) = fixture(u32::MAX);
    db.remove(&target()).unwrap();
    db.add(Box::new(Nested {
        oid: target(),
        depth: 29,
    }))
    .unwrap();
    db.poll_trend_logs();
    assert_eq!(count(&db, oid), 1);
    let PropertyValue::ApplicationData(bytes) = Nested::value(29) else {
        unreachable!()
    };
    assert_eq!(last_datum(&db, oid), LogDatum::AnyValue(bytes));

    db.remove(&oid).unwrap();
    let log = super::multiple_tests::multiple(
        u32::MAX,
        16,
        vec![super::multiple_tests::member(
            target(),
            P::PRESENT_VALUE,
            None,
            None,
        )],
    );
    let multiple = log.object_identifier();
    db.add(Box::new(log)).unwrap();
    db.poll_trend_logs();
    let records = super::multiple_tests::records(&db, multiple);
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].log_data,
        LogData::Values(vec![LogValue::Failure {
            error_class: ErrorClass::SERVICES,
            error_code: ErrorCode::OTHER,
        }])
    );
}

/// Polling an Object_Name longer than the cap logs the failure record,
/// which ReadRange then serves like any other.
#[test]
fn a_polled_value_too_long_to_page_logs_value_too_long() {
    let (mut db, oid, _, _) = fixture(u32::MAX);
    db.remove(&target()).unwrap();
    db.add(Box::new(
        AnalogValueObject::new(1, "x".repeat(300), EngineeringUnits::NO_UNITS).unwrap(),
    ))
    .unwrap();
    let mut object = trend(u32::MAX, 16);
    object
        .set_log_device_object_property(Some(BACnetDeviceObjectPropertyReference {
            object_identifier: target(),
            property_identifier: P::OBJECT_NAME.to_raw(),
            property_array_index: None,
            device_identifier: None,
        }))
        .unwrap();
    db.add(Box::new(object)).unwrap();
    db.poll_trend_logs();
    assert_eq!(count(&db, oid), 1);
    assert_eq!(
        last_datum(&db, oid),
        failure(ErrorClass::PROPERTY, ErrorCode::VALUE_TOO_LONG)
    );
}

/// The record ReadRange serves for the newest sample of `oid`.
fn last_served(db: &ObjectDatabase, oid: ObjectIdentifier) -> Vec<u8> {
    let records = db
        .get(&oid)
        .unwrap()
        .log_buffer()
        .and_then(|log| log.buffer_records())
        .unwrap();
    let mut bytes = BytesMut::new();
    records.encode_record(records.record_count() - 1, &mut bytes);
    bytes.to_vec()
}

/// A Trend Log polling a CharacterString and one polling a Double each log
/// the value's encoding inside any-value [10], which decodes back to the
/// value read.
#[test]
fn a_character_string_and_a_double_are_logged_and_read_back_exactly() {
    let (mut db, oid, _, _) = fixture(u32::MAX);
    let mut large = LargeAnalogValueObject::new(1, "LAV").unwrap();
    large.set_relinquish_default(2.5).unwrap();
    let large_oid = large.object_identifier();
    db.add(Box::new(large)).unwrap();
    // The fixture's frame: 2024-02-29 (Thursday) 12:00:00.37.
    let timestamp = [
        0x0E, 0xA4, 0x7C, 0x02, 0x1D, 0x04, 0xB4, 0x0C, 0x00, 0x00, 0x25, 0x0F,
    ];
    for (target, property, encoded, value) in [
        (
            target(),
            P::OBJECT_NAME,
            vec![0x73, 0x00, b'A', b'V'],
            PropertyValue::CharacterString("AV".into()),
        ),
        (
            large_oid,
            P::PRESENT_VALUE,
            vec![0x55, 0x08, 0x40, 0x04, 0, 0, 0, 0, 0, 0],
            PropertyValue::Double(2.5),
        ),
    ] {
        let mut object = trend(u32::MAX, 16);
        object
            .set_log_device_object_property(Some(BACnetDeviceObjectPropertyReference {
                object_identifier: target,
                property_identifier: property.to_raw(),
                property_array_index: None,
                device_identifier: None,
            }))
            .unwrap();
        db.add(Box::new(object)).unwrap();
        db.poll_trend_logs();

        let mut expected = timestamp.to_vec();
        expected.extend([0x1E, 0xAE]);
        expected.extend(&encoded);
        expected.extend([0xAF, 0x1F]);
        assert_eq!(last_served(&db, oid), expected, "{property:?}");
        assert_eq!(last_datum(&db, oid), LogDatum::AnyValue(encoded.clone()));
        assert_eq!(
            decode_application_value(&encoded, 0).unwrap(),
            (value, encoded.len())
        );
    }
}
