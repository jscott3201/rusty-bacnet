//! [`Value`]: both forms from one formatter, and the accessors over each
//! primitive and a constructed value.

use bacnet_encoding::primitives::encode_property_value;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier as P};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};
use bytes::BytesMut;

use super::*;
use crate::format::{Floats, StateTexts, ValueFormat};

fn encoded(values: &[PropertyValue]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    for value in values {
        encode_property_value(&mut buf, value).unwrap();
    }
    buf.to_vec()
}

/// The value of `property` of an `object_type` object holding `values`, as
/// the Device reads write it, with `texts` and `units` as the read had
/// them.
fn read(
    object_type: ObjectType,
    property: P,
    values: &[PropertyValue],
    texts: &StateTexts,
    units: Option<EngineeringUnits>,
) -> Value {
    let format = ValueFormat::new(object_type, texts, Floats::Exact).whole_lists();
    Value::new(&format, property, None, encoded(values), units)
}

fn plain(object_type: ObjectType, property: P, value: PropertyValue) -> Value {
    read(
        object_type,
        property,
        &[value],
        &StateTexts::default(),
        None,
    )
}

/// Each primitive answers the accessors that fit it and no others; the
/// octets and the text are the ones sent and written.
#[test]
fn each_primitive_answers_its_accessors() {
    let ai = ObjectType::ANALOG_INPUT;
    let null = plain(ai, P::PRESENT_VALUE, PropertyValue::Null);
    assert!(null.is_null());
    assert_eq!((null.as_f64(), null.as_bool()), (None, None));

    let flag = plain(ai, P::OUT_OF_SERVICE, PropertyValue::Boolean(true));
    assert_eq!(flag.as_bool(), Some(true));
    assert_eq!(
        (flag.as_f64(), flag.as_u64(), flag.is_null()),
        (None, None, false)
    );

    let unsigned = plain(ai, P::COV_INCREMENT, PropertyValue::Unsigned(u64::MAX));
    assert_eq!(unsigned.as_u64(), Some(u64::MAX));
    assert_eq!(unsigned.as_i64(), None, "past i64");
    assert_eq!(unsigned.as_f64(), Some(u64::MAX as f64));
    let small = plain(ai, P::COV_INCREMENT, PropertyValue::Unsigned(7));
    assert_eq!(small.as_i64(), Some(7));

    let signed = plain(ai, P::PRESENT_VALUE, PropertyValue::Signed(-3));
    assert_eq!(signed.as_i64(), Some(-3));
    assert_eq!(signed.as_u64(), None, "negative");
    assert_eq!(signed.as_f64(), Some(-3.0));
    let positive = plain(ai, P::PRESENT_VALUE, PropertyValue::Signed(3));
    assert_eq!(positive.as_u64(), Some(3));

    // A REAL as the decimal it stands for, not its binary widening.
    let real = plain(ai, P::PRESENT_VALUE, PropertyValue::Real(0.1));
    assert_eq!(real.as_f64(), Some(0.1));
    assert_eq!(real.text, "0.1");
    assert_eq!(real.as_u64(), None);
    let double = plain(ai, P::PRESENT_VALUE, PropertyValue::Double(2.5e300));
    assert_eq!(double.as_f64(), Some(2.5e300));
    let nan = plain(ai, P::PRESENT_VALUE, PropertyValue::Real(f32::NAN));
    assert!(nan.as_f64().is_some_and(f64::is_nan));

    let name = plain(
        ai,
        P::OBJECT_NAME,
        PropertyValue::CharacterString("Zone \"A\"".into()),
    );
    assert_eq!(name.as_str(), Some("Zone \"A\""));
    assert!(name.may_hold_device_text());
    assert_eq!(name.as_f64(), None);

    let octets = plain(ai, P::PRESENT_VALUE, PropertyValue::OctetString(vec![1, 2]));
    assert_eq!(
        octets.data,
        Datum::Octets {
            octets: "0102".into()
        }
    );
    assert_eq!(octets.as_str(), None);

    let state = plain(ai, P::EVENT_STATE, PropertyValue::Enumerated(2));
    assert_eq!(
        state.data,
        Datum::Enumerated {
            value: 2,
            name: Some("offnormal".into())
        }
    );
    assert_eq!((state.as_u64(), state.as_bool()), (None, None));

    let flags = plain(
        ai,
        P::STATUS_FLAGS,
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0b0100_0000],
        },
    );
    assert!(matches!(&flags.data, Datum::Bits { bits, .. } if bits == "0100"));
    assert_eq!(flags.as_bool(), None);

    let object = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let id = plain(
        ai,
        P::OBJECT_IDENTIFIER,
        PropertyValue::ObjectIdentifier(object),
    );
    assert!(matches!(&id.data, Datum::Object { instance: 1, .. }));
    assert_eq!(
        id.decode().unwrap(),
        PropertyValue::ObjectIdentifier(object)
    );

    for value in [
        PropertyValue::Date(Date {
            year: 126,
            month: 11,
            day: 14,
            day_of_week: 6,
        }),
        PropertyValue::Time(Time {
            hour: 7,
            minute: 0,
            second: 0,
            hundredths: 0,
        }),
    ] {
        let when = plain(ai, P::PRESENT_VALUE, value.clone());
        assert_eq!(when.decode().unwrap(), value);
        assert_eq!((when.as_f64(), when.as_str()), (None, None));
    }
}

/// A binary object's state reads as a truth value, named by its own text
/// when the read had it, and then its text holds device text.
#[test]
fn a_binary_state_is_a_bool_named_by_its_text() {
    let bv = ObjectType::BINARY_VALUE;
    let active = plain(bv, P::PRESENT_VALUE, PropertyValue::Enumerated(1));
    assert_eq!(active.as_bool(), Some(true));
    assert_eq!(active.text, "active");
    assert!(!active.may_hold_device_text());
    let inactive = plain(bv, P::PRESENT_VALUE, PropertyValue::Enumerated(0));
    assert_eq!(inactive.as_bool(), Some(false));
    // Not a binary object's state: an enumeration, no truth value.
    let polarity = plain(bv, P::POLARITY, PropertyValue::Enumerated(1));
    assert_eq!(polarity.as_bool(), None);

    let texts = StateTexts {
        active: Some("Running".into()),
        inactive: None,
        states: Vec::new(),
    };
    let named = read(
        bv,
        P::PRESENT_VALUE,
        &[PropertyValue::Enumerated(1)],
        &texts,
        None,
    );
    assert_eq!(named.text, "active (Running)");
    assert_eq!(named.as_bool(), Some(true));
    assert!(named.may_hold_device_text());
    // The same texts don't touch a property that holds no state.
    let polarity = read(
        bv,
        P::POLARITY,
        &[PropertyValue::Enumerated(1)],
        &texts,
        None,
    );
    assert!(!polarity.may_hold_device_text());
}

/// A multi-state value is its number, named by its State_Text.
#[test]
fn a_multistate_value_is_its_number_named() {
    let texts = StateTexts {
        states: vec!["Off".into(), "Heat".into()],
        ..StateTexts::default()
    };
    let value = read(
        ObjectType::MULTI_STATE_VALUE,
        P::PRESENT_VALUE,
        &[PropertyValue::Unsigned(2)],
        &texts,
        None,
    );
    assert_eq!(value.text, "2 (Heat)");
    assert_eq!(value.as_u64(), Some(2));
    assert_eq!(value.data, Datum::Unsigned(2));
}

/// A whole property's number takes its units; an array element doesn't.
#[test]
fn units_follow_a_whole_number() {
    let texts = StateTexts::default();
    let pv = read(
        ObjectType::ANALOG_INPUT,
        P::PRESENT_VALUE,
        &[PropertyValue::Real(21.5)],
        &texts,
        Some(EngineeringUnits::DEGREES_CELSIUS),
    );
    assert_eq!(pv.text, "21.5 °C");
    assert_eq!(pv.as_f64(), Some(21.5));
    let format = ValueFormat::new(ObjectType::ANALOG_OUTPUT, &texts, Floats::Exact);
    let slot = Value::new(
        &format,
        P::PRIORITY_ARRAY,
        Some(8),
        encoded(&[PropertyValue::Real(50.0)]),
        Some(EngineeringUnits::PERCENT),
    );
    assert_eq!(slot.text, "50");
}

/// A constructed value: a priority array whole is a list of its slots,
/// which no scalar accessor answers, and decodes as a list.
#[test]
fn a_constructed_value_is_a_list() {
    let mut slots = vec![PropertyValue::Null; 16];
    slots[7] = PropertyValue::Real(72.5);
    let array = read(
        ObjectType::ANALOG_OUTPUT,
        P::PRIORITY_ARRAY,
        &slots,
        &StateTexts::default(),
        None,
    );
    let Datum::List(items) = &array.data else {
        panic!("{array:?}");
    };
    assert_eq!(items.len(), 16);
    assert_eq!(items[7], Datum::Real(72.5));
    assert_eq!(
        (array.as_f64(), array.is_null(), array.as_str()),
        (None, false, None)
    );
    assert_eq!(array.decode().unwrap(), PropertyValue::List(slots));
    assert!(array.text.contains("72.5"), "{}", array.text);
    assert_eq!(array.to_string(), array.text);
}

/// Each application datatype as octets, the text and the typed data the
/// formatter gives it on an Analog Input's Present_Value (#2618 pins these
/// before the tag-tree decoder takes over the generic view).
fn application_cases() -> Vec<(Vec<u8>, &'static str, serde_json::Value)> {
    use serde_json::json;
    let mut object = vec![0xC4];
    object.extend_from_slice(&((ObjectType::ANALOG_INPUT.to_raw() << 22) | 7).to_be_bytes());
    vec![
        (vec![0x00], "null", json!(null)),
        (vec![0x10], "false", json!(false)),
        (vec![0x11], "true", json!(true)),
        (vec![0x21, 0x48], "72", json!(72)),
        (vec![0x31, 0xB8], "-72", json!(-72)),
        (vec![0x44, 0x42, 0x91, 0, 0], "72.5", json!(72.5)),
        (
            vec![0x55, 0x08, 0x40, 0x52, 0x20, 0, 0, 0, 0, 0],
            "72.5",
            json!(72.5),
        ),
        (
            vec![0x63, 0x12, 0x34, 0xFF],
            "0x1234ff",
            json!({"octets": "1234ff"}),
        ),
        (vec![0x60], "(no octets)", json!({"octets": ""})),
        (vec![0x73, 0x00, b'h', b'i'], "\"hi\"", json!("hi")),
        (vec![0x82, 0x05, 0xE0], "bits(111)", json!({"bits": "111"})),
        (vec![0x91, 0x03], "3", json!({"value": 3})),
        (
            vec![0xA4, 126, 10, 3, 6],
            "2026-10-03 Sat",
            json!({"date": "2026-10-03 Sat"}),
        ),
        (vec![0xB4, 12, 30, 0, 0], "12:30", json!({"time": "12:30"})),
        (
            object,
            "analog-input 7",
            json!({"type": "analog-input", "type_number": 0, "instance": 7}),
        ),
    ]
}

#[test]
fn application_values_keep_their_text_and_data() {
    let texts = StateTexts::default();
    let format = ValueFormat::new(ObjectType::ANALOG_INPUT, &texts, Floats::Exact);
    for (octets, text, data) in application_cases() {
        let value = Value::new(&format, P::PRESENT_VALUE, None, octets.clone(), None);
        assert_eq!(value.text, text, "{octets:02x?}");
        assert_eq!(serde_json::to_value(&value.data).unwrap(), data);
        assert!(!value.may_hold_device_text() || text.starts_with('"'));
        // Two of them: a list of both, in wire order.
        let pair = [&[0x21, 0x01][..], &octets].concat();
        let value = Value::new(&format, P::PRESENT_VALUE, None, pair, None);
        assert_eq!(value.text, format!("[1, {text}]"));
        assert_eq!(
            serde_json::to_value(&value.data).unwrap(),
            serde_json::json!([1, data])
        );
    }
}

/// Octets that aren't application values (an Unsigned whose octet is
/// missing): raw data, and a decoding error.
#[test]
fn octets_that_dont_decode_are_raw() {
    let texts = StateTexts::default();
    let format = ValueFormat::new(ObjectType::ANALOG_INPUT, &texts, Floats::Exact);
    let value = Value::new(&format, P::PRESENT_VALUE, None, vec![0x21], None);
    assert_eq!(value.data, Datum::Raw { raw: "21".into() });
    assert_eq!(value.raw, [0x21]);
    assert!(value.decode().is_err());
    // A context-tagged part decodes as its octets.
    let tagged = Value::new(&format, P::PRESENT_VALUE, None, vec![0x0E, 0x0F], None);
    assert_eq!(
        tagged.decode().unwrap(),
        PropertyValue::ApplicationData(vec![0x0E, 0x0F])
    );
}
