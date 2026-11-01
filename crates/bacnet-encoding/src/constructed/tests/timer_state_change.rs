//! BACnetTimerStateChangeValue vectors (Clause 21): each alternative by hand,
//! and the malformed inputs the decoder refuses.
use crate::constructed::{decode_timer_state_change_value, encode_timer_state_change_value};
use bacnet_types::constructed::{BACnetLightingCommand, BACnetTimerStateChangeValue as V};
use bacnet_types::enums::LightingOperation;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, PropertyValue, Time};
use bytes::BytesMut;

fn vectors() -> Vec<(V, Vec<u8>)> {
    let date = Date {
        year: 126,
        month: 11,
        day: 1,
        day_of_week: 7,
    };
    let time = Time {
        hour: 8,
        minute: 30,
        second: 0,
        hundredths: 0,
    };
    vec![
        (V::NoValue, vec![0x08]),
        (V::Primitive(PropertyValue::Null), vec![0x00]),
        (V::Primitive(PropertyValue::Boolean(true)), vec![0x11]),
        (
            V::Primitive(PropertyValue::Unsigned(300)),
            vec![0x22, 0x01, 0x2C],
        ),
        (V::Primitive(PropertyValue::Signed(-2)), vec![0x31, 0xFE]),
        (
            V::Primitive(PropertyValue::Real(1.5)),
            vec![0x44, 0x3F, 0xC0, 0x00, 0x00],
        ),
        (V::Primitive(PropertyValue::Enumerated(1)), vec![0x91, 0x01]),
        (
            V::Primitive(PropertyValue::CharacterString("on".into())),
            vec![0x73, 0x00, b'o', b'n'],
        ),
        // A BACnetScale-like value of another datatype: [1] around its octets.
        (
            V::Constructed(vec![0x09, 0x05]),
            vec![0x1E, 0x09, 0x05, 0x1F],
        ),
        (
            V::DateTime(date, time),
            vec![0x2E, 0xA4, 126, 11, 1, 7, 0xB4, 8, 30, 0, 0, 0x2F],
        ),
        (
            V::LightingCommand(BACnetLightingCommand {
                priority: Some(8),
                ..BACnetLightingCommand::new(LightingOperation::STOP)
            }),
            vec![0x3E, 0x09, 0x0A, 0x59, 0x08, 0x3F],
        ),
    ]
}

#[test]
fn each_alternative_matches_its_vector_and_round_trips() {
    for (value, wire) in vectors() {
        let mut buf = BytesMut::new();
        encode_timer_state_change_value(&mut buf, &value).unwrap();
        assert_eq!(&buf[..], &wire[..], "{value:?}");
        // Followed by another element: the decoder stops after the first.
        let mut two = wire.clone();
        two.push(0x08);
        assert_eq!(
            decode_timer_state_change_value(&two, 0).unwrap(),
            (value, wire.len())
        );
    }
}

#[test]
fn encode_refuses_what_has_no_encoding_and_leaves_the_buffer() {
    for value in [
        V::Primitive(PropertyValue::List(vec![])),
        V::Primitive(PropertyValue::ApplicationData(vec![0x08])),
        V::Constructed(vec![]),
    ] {
        let mut buf = BytesMut::from(&[0xAA][..]);
        assert!(
            matches!(
                encode_timer_state_change_value(&mut buf, &value),
                Err(Error::Encode(_))
            ),
            "{value:?}"
        );
        assert_eq!(&buf[..], &[0xAA]);
    }
}

#[test]
fn decode_refuses_malformed_choices() {
    for (wire, what) in [
        (&[][..], "empty"),
        (&[0x09, 0x00][..], "no-value with contents"),
        (&[0x1E, 0x1F][..], "empty constructed-value"),
        (&[0x1E, 0x09, 0x05][..], "constructed-value left open"),
        (
            &[0x2E, 0xA4, 126, 11, 1, 7, 0x2F][..],
            "datetime without a time",
        ),
        (
            &[0x2E, 0xB4, 8, 30, 0, 0, 0xA4, 126, 11, 1, 7, 0x2F][..],
            "datetime out of order",
        ),
        (
            &[0x3E, 0x19, 0x01, 0x3F][..],
            "lighting command without operation",
        ),
        (
            &[0x3E, 0x09, 0x0A, 0x2F][..],
            "lighting command closed by tag 2",
        ),
        (&[0x4E, 0x00, 0x4F][..], "unknown context tag 4"),
        (&[0x19, 0x01][..], "primitive context tag 1"),
        (&[0x1F][..], "a closing tag"),
        (&[0x22, 0x01][..], "unsigned cut short"),
    ] {
        assert!(
            decode_timer_state_change_value(wire, 0).is_err(),
            "{what} decoded"
        );
    }
}
