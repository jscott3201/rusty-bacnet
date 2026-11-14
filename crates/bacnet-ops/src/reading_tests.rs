//! What a read names, and how its raw rows become readings.

use bacnet_encoding::primitives::encode_property_value;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

use super::*;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn encoded(value: PropertyValue) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_property_value(&mut buf, &value).unwrap();
    buf.to_vec()
}

#[test]
fn a_property_reads_from_text_and_tuples() {
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    assert_eq!(
        "ai:1".into_property().unwrap(),
        PropertyRef::new(ai, P::PRESENT_VALUE)
    );
    assert_eq!(
        "analog-input:1  units".into_property().unwrap(),
        PropertyRef::new(ai, P::UNITS)
    );
    let ao = oid(ObjectType::ANALOG_OUTPUT, 2);
    assert_eq!(
        String::from("ao:2 priority-array[8]")
            .into_property()
            .unwrap(),
        PropertyRef::new(ao, P::PRIORITY_ARRAY).at(8)
    );
    assert_eq!(
        (ao, P::PRIORITY_ARRAY, 0).into_property().unwrap(),
        PropertyRef::new(ao, P::PRIORITY_ARRAY).at(0)
    );
    assert_eq!(
        (ai, P::OBJECT_NAME).into_property().unwrap(),
        PropertyRef::new(ai, P::OBJECT_NAME)
    );
}

#[test]
fn text_that_names_no_property_is_a_parse_error() {
    for (text, what) in [
        ("", "object"),
        ("nonsense:1 pv", "object"),
        ("ai:99999999 pv", "object"),
        ("ai:1 not-a-property", "property"),
        ("ai:1 pv extra", "property"),
    ] {
        let error = text.into_property().unwrap_err();
        assert!(
            matches!(&error, OpsError::Parse { what: w, input, .. } if *w == what && input == text),
            "{text:?}: {error:?}"
        );
        assert!(!error.to_string().is_empty());
    }
}

/// An object's Units and state texts name its values when the same read
/// holds them; another object's don't, nor do an element's, and a refused
/// property keeps its error.
#[test]
fn readings_name_values_with_what_the_read_held() {
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let ai2 = oid(ObjectType::ANALOG_INPUT, 2);
    let msv = oid(ObjectType::MULTI_STATE_VALUE, 1);
    let refused = PropertyError::Remote(RemoteError::Error {
        class: ErrorClass::PROPERTY,
        code: ErrorCode::UNKNOWN_PROPERTY,
    });
    let states = {
        let mut buf = BytesMut::new();
        for text in ["Off", "Heat"] {
            encode_property_value(&mut buf, &PropertyValue::CharacterString(text.into())).unwrap();
        }
        buf.to_vec()
    };
    let raw = vec![
        (
            ai,
            None,
            (P::PRESENT_VALUE, Ok(encoded(PropertyValue::Real(21.5)))),
        ),
        (
            ai,
            None,
            (P::UNITS, Ok(encoded(PropertyValue::Enumerated(62)))),
        ),
        (
            ai2,
            None,
            (P::PRESENT_VALUE, Ok(encoded(PropertyValue::Real(3.0)))),
        ),
        (
            msv,
            None,
            (P::PRESENT_VALUE, Ok(encoded(PropertyValue::Unsigned(2)))),
        ),
        (msv, None, (P::STATE_TEXT, Ok(states))),
        (msv, Some(1), (P::STATE_TEXT, Err(refused.clone()))),
    ];
    let read = readings(raw);
    let texts: Vec<_> = read
        .iter()
        .map(|r| r.value.as_ref().map(|v| v.text.as_str()))
        .collect();
    assert_eq!(texts[0], Ok("21.5 °C"));
    assert_eq!(texts[2], Ok("3"), "another object's units don't apply");
    assert_eq!(texts[3], Ok("2 (Heat)"));
    assert_eq!(read[5].value, Err(refused));
    assert_eq!(read[5].index, Some(1));
    assert!(read[3].value.as_ref().unwrap().may_hold_device_text());
    let readings = Readings {
        readings: read,
        via: Some(ReadVia::Rpm),
        stopped: None,
    };
    assert_eq!(
        readings.get(ai, P::UNITS).map(|r| r.property),
        Some(P::UNITS)
    );
    assert!(readings.get(ai2, P::UNITS).is_none());
}

/// The read types cross tasks: a front end's futures are `Send`.
#[test]
fn the_read_types_are_send_and_sync() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<PropertyRef>();
    send_sync::<Reading>();
    send_sync::<Readings>();
    send_sync::<Value>();
    send_sync::<OpsError>();
}
