//! BACnetNameValue (Clause 21, #1553): the name as a CharacterString under
//! primitive context tag 0 (`09`..`0D`, a UTF-8 charset octet first), then an
//! optional application-tagged primitive or a BACnetDateTime (application
//! Date `A4`, then Time `B4`). The vectors are worked by hand from Clause 20.2,
//! not produced by the codec.
use crate::constructed::{decode_name_value, encode_name_value};
use bacnet_types::constructed::{BACnetNameValue, TagValue};
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};
use bytes::BytesMut;

/// 2026-10-06, a Tuesday.
const DATE: Date = Date {
    year: 126,
    month: 10,
    day: 6,
    day_of_week: 2,
};

/// 12:30:00.00.
const TIME: Time = Time {
    hour: 12,
    minute: 30,
    second: 0,
    hundredths: 0,
};

fn valued(name: &str, value: PropertyValue) -> BACnetNameValue {
    BACnetNameValue::valued(name, TagValue::Primitive(value))
}

fn vectors() -> Vec<(BACnetNameValue, Vec<u8>)> {
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 30).unwrap();
    vec![
        // A semantic tag: eight content octets, so the length is extended.
        (
            BACnetNameValue::semantic("exhaust"),
            vec![0x0D, 0x08, 0x00, 0x65, 0x78, 0x68, 0x61, 0x75, 0x73, 0x74],
        ),
        (BACnetNameValue::semantic(""), vec![0x09, 0x00]),
        // REAL 1030.0 is 0x4480C000.
        (
            valued("fan", PropertyValue::Real(1030.0)),
            vec![0x0C, 0x00, 0x66, 0x61, 0x6E, 0x44, 0x44, 0x80, 0xC0, 0x00],
        ),
        (
            BACnetNameValue::valued(
                "due",
                TagValue::DateTime {
                    date: DATE,
                    time: TIME,
                },
            ),
            vec![
                0x0C, 0x00, 0x64, 0x75, 0x65, 0xA4, 0x7E, 0x0A, 0x06, 0x02, 0xB4, 0x0C, 0x1E, 0x00,
                0x00,
            ],
        ),
        // A Date with no Time after it is a Date.
        (
            valued("a", PropertyValue::Date(DATE)),
            vec![0x0A, 0x00, 0x61, 0xA4, 0x7E, 0x0A, 0x06, 0x02],
        ),
        (
            valued("a", PropertyValue::Unsigned(5)),
            vec![0x0A, 0x00, 0x61, 0x21, 0x05],
        ),
        (
            valued("a", PropertyValue::CharacterString("x".into())),
            vec![0x0A, 0x00, 0x61, 0x72, 0x00, 0x78],
        ),
        (
            valued("a", PropertyValue::Null),
            vec![0x0A, 0x00, 0x61, 0x00],
        ),
        (
            valued("a", PropertyValue::Boolean(true)),
            vec![0x0A, 0x00, 0x61, 0x11],
        ),
        (
            valued("a", PropertyValue::ObjectIdentifier(device)),
            vec![0x0A, 0x00, 0x61, 0xC4, 0x02, 0x00, 0x00, 0x1E],
        ),
    ]
}

#[test]
fn name_value_golden_vectors_round_trip() {
    for (value, octets) in vectors() {
        let mut buf = BytesMut::new();
        encode_name_value(&mut buf, &value).unwrap();
        assert_eq!(buf.as_ref(), octets.as_slice(), "{value:?}");
        assert_eq!(
            decode_name_value(&octets, 0).unwrap(),
            (value, octets.len())
        );
    }
}

#[test]
fn name_value_elements_decode_back_to_back() {
    let mut array = Vec::new();
    let values: Vec<_> = vectors()
        .into_iter()
        .map(|(value, octets)| {
            array.extend(octets);
            value
        })
        .collect();
    let mut offset = 0;
    let mut decoded = Vec::new();
    while offset < array.len() {
        let (value, end) = decode_name_value(&array, offset).unwrap();
        decoded.push(value);
        offset = end;
    }
    assert_eq!(decoded, values);
    // An element ends where the next one's context tag 0 begins.
    let semantic_then_more = [0x0A, 0x00, 0x61, 0x0A, 0x00, 0x62];
    assert_eq!(
        decode_name_value(&semantic_then_more, 0).unwrap(),
        (BACnetNameValue::semantic("a"), 3)
    );
}

#[test]
fn name_value_refuses_malformed_octets_without_panicking() {
    let malformed: [&[u8]; 11] = [
        &[],
        // The name as an application CharacterString, or a constructed [0].
        &[0x72, 0x00, 0x61],
        &[0x0E, 0x0F],
        // Another context tag.
        &[0x1A, 0x00, 0x61],
        // The name runs past the data, or isn't valid UTF-8.
        &[0x0D, 0x08, 0x00, 0x65],
        &[0x0A, 0x00, 0xFF],
        // An unsupported character set (UCS-4).
        &[0x0A, 0x03, 0x61],
        // A REAL cut short, and one with the wrong length.
        &[0x0A, 0x00, 0x61, 0x44, 0x44, 0x80],
        &[0x0A, 0x00, 0x61, 0x43, 0x44, 0x80, 0xC0],
        // Application tag 13 is reserved.
        &[0x0A, 0x00, 0x61, 0xD1, 0x00],
        // A BACnetDateTime whose Time runs past the data.
        &[0x0A, 0x00, 0x61, 0xA4, 0x7E, 0x0A, 0x06, 0x02, 0xB4, 0x0C],
    ];
    for octets in malformed {
        assert!(decode_name_value(octets, 0).is_err(), "{octets:02X?}");
    }
    // Every prefix of a valid element either fails or decodes a shorter
    // element; none panics.
    for (_, octets) in vectors() {
        for len in 0..octets.len() {
            if let Ok((_, end)) = decode_name_value(&octets[..len], 0) {
                assert!(end <= len);
            }
        }
    }
}

#[test]
fn name_value_encoding_refuses_a_constructed_value_and_leaves_the_buffer() {
    for value in [
        PropertyValue::List(vec![PropertyValue::Unsigned(1)]),
        PropertyValue::ApplicationData(vec![0x08]),
    ] {
        let mut buf = BytesMut::from(&[0xAA][..]);
        assert!(encode_name_value(&mut buf, &valued("a", value)).is_err());
        assert_eq!(buf.as_ref(), [0xAA]);
    }
}
