//! Encode-then-decode round trips for the application values and the
//! constructed values the services carry, over generated values (#2180).
//!
//! Values are compared by their `Debug` text: a REAL or Double may be a
//! NaN, which equals nothing, while its text matches itself (and a negative
//! zero's differs from a positive one's).

// The BytesMut encoders and the owned decoders need alloc.
#![cfg(feature = "alloc")]

use std::fmt::Debug;

use bacnet_encoding::constructed::{
    decode_bacnet_property_value, decode_calendar_entry, decode_date_range,
    decode_object_property_reference_at, decode_read_access_specification, decode_recipient,
    decode_time_value, encode_bacnet_property_value, encode_calendar_entry, encode_date_range,
    encode_object_property_reference, encode_read_access_specification, encode_recipient,
    encode_time_value,
};
use bacnet_encoding::primitives::{
    ApplicationValueRef, charset, decode_application_value, decode_application_value_ref,
    decode_timestamp_choice, encode_property_value, encode_timestamp_choice,
    validate_application_value,
};
use bacnet_encoding::tags::{TagClass, app_tag, encode_tag};
use bacnet_types::constructed::{
    BACnetAddress, BACnetCalendarEntry, BACnetDateRange, BACnetObjectPropertyReference,
    BACnetPropertyValue, BACnetRecipient, BACnetTimeValue, BACnetWeekNDay, PropertyReference,
    ReadAccessSpecification,
};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, PropertyValue, Time};
use bytes::BytesMut;
use proptest::prelude::*;

/// `value` encoded, then decoded from offset 0: the decoder reads the whole
/// encoding back to the same value.
fn round_trip<T: Debug>(
    value: &T,
    encode: impl Fn(&mut BytesMut, &T) -> Result<(), Error>,
    decode: impl Fn(&[u8], usize) -> Result<(T, usize), Error>,
) -> Result<(), TestCaseError> {
    let mut buf = BytesMut::new();
    encode(&mut buf, value).map_err(|error| TestCaseError::fail(format!("{error:?}")))?;
    let (decoded, end) =
        decode(&buf, 0).map_err(|error| TestCaseError::fail(format!("{buf:02x?}: {error:?}")))?;
    prop_assert_eq!(end, buf.len(), "{:02x?} decodes only in part", &buf[..]);
    prop_assert_eq!(format!("{decoded:?}"), format!("{value:?}"));
    Ok(())
}

fn date() -> impl Strategy<Value = Date> {
    any::<[u8; 4]>().prop_map(|[year, month, day, day_of_week]| Date {
        year,
        month,
        day,
        day_of_week,
    })
}

fn time() -> impl Strategy<Value = Time> {
    any::<[u8; 4]>().prop_map(|[hour, minute, second, hundredths]| Time {
        hour,
        minute,
        second,
        hundredths,
    })
}

fn object_identifier() -> impl Strategy<Value = ObjectIdentifier> {
    (0u32..1024, 0..=ObjectIdentifier::MAX_INSTANCE).prop_map(|(object_type, instance)| {
        ObjectIdentifier::new(ObjectType::from_raw(object_type), instance)
            .expect("a type under 1,024 and an instance within 22 bits")
    })
}

/// An unsigned at the edges of each width from one octet to eight (0, each
/// width's largest value and the next one up), or any value.
fn unsigned() -> impl Strategy<Value = u64> {
    let edges: Vec<u64> = (1u32..=8)
        .flat_map(|octets| {
            let max = u64::MAX >> (64 - 8 * octets);
            [max, max.wrapping_add(1)]
        })
        .chain([0, 1])
        .collect();
    prop_oneof![
        prop::sample::select(edges),
        any::<u8>().prop_map(u64::from),
        any::<u16>().prop_map(u64::from),
        any::<u32>().prop_map(u64::from),
        any::<u64>(),
    ]
}

/// A Signed at the edges of each width from one octet to four, on both
/// sides of zero, or any value.
fn signed() -> impl Strategy<Value = i32> {
    prop_oneof![
        prop::sample::select(vec![
            0,
            -1,
            1,
            127,
            128,
            -128,
            -129,
            32_767,
            32_768,
            -32_768,
            -32_769,
            (1 << 23) - 1,
            1 << 23,
            -(1 << 23),
            -(1 << 23) - 1,
            i32::MAX,
            i32::MIN,
        ]),
        any::<i32>(),
    ]
}

/// An Enumerated at the width edges, or any value.
fn enumerated() -> impl Strategy<Value = u32> {
    prop_oneof![
        prop::sample::select(vec![
            0,
            0xFF,
            0x100,
            0xFFFF,
            0x1_0000,
            0xFF_FFFF,
            0x100_0000,
            u32::MAX
        ]),
        any::<u32>(),
    ]
}

/// A REAL with its special values drawn explicitly: NaNs (quiet,
/// signalling, negative), both infinities, both zeros, the subnormal range
/// and its edges, the extremes; or any bit pattern.
fn real() -> impl Strategy<Value = f32> {
    prop_oneof![
        prop::sample::select(vec![
            f32::NAN,
            -f32::NAN,
            f32::from_bits(0x7F80_0001),
            f32::INFINITY,
            f32::NEG_INFINITY,
            0.0,
            -0.0,
            f32::from_bits(1),
            f32::from_bits(0x007F_FFFF),
            f32::MIN_POSITIVE,
            f32::MAX,
            f32::MIN,
        ]),
        (1u32..0x0080_0000, any::<bool>())
            .prop_map(|(bits, negative)| f32::from_bits(bits | (u32::from(negative) << 31))),
        any::<u32>().prop_map(f32::from_bits),
    ]
}

/// A Double, drawn as [`real`] draws a REAL.
fn double() -> impl Strategy<Value = f64> {
    prop_oneof![
        prop::sample::select(vec![
            f64::NAN,
            -f64::NAN,
            f64::from_bits(0x7FF0_0000_0000_0001),
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
            -0.0,
            f64::from_bits(1),
            f64::from_bits(0x000F_FFFF_FFFF_FFFF),
            f64::MIN_POSITIVE,
            f64::MAX,
            f64::MIN,
        ]),
        (1u64..0x0010_0000_0000_0000, any::<bool>())
            .prop_map(|(bits, negative)| f64::from_bits(bits | (u64::from(negative) << 63))),
        any::<u64>().prop_map(f64::from_bits),
    ]
}

/// A length for strings and octet strings: empty, around the 253 and 254
/// octet edges where a tag's length moves to its extended forms, or any
/// length up to 600.
fn length() -> impl Strategy<Value = usize> {
    prop_oneof![
        Just(0usize),
        prop::sample::select(vec![1, 4, 5, 252, 253, 254, 255, 256, 300, 600]),
        0usize..600,
    ]
}

fn octet_string() -> impl Strategy<Value = Vec<u8>> {
    length().prop_flat_map(|length| prop::collection::vec(any::<u8>(), length))
}

/// A string of up to 600 characters: ASCII, or any characters, which may
/// take up to four octets each.
fn character_string() -> impl Strategy<Value = String> {
    length().prop_flat_map(|length| {
        prop_oneof![
            prop::collection::vec(0x20u8..0x7F, length)
                .prop_map(|octets| octets.into_iter().map(char::from).collect::<String>()),
            prop::collection::vec(any::<char>(), length / 4)
                .prop_map(|chars| chars.into_iter().collect::<String>()),
        ]
    })
}

/// A bit string: up to 7 unused bits, and at least one data octet when any
/// bit is unused.
fn bit_string() -> impl Strategy<Value = PropertyValue> {
    (0u8..8, octet_string()).prop_map(|(unused_bits, mut data)| {
        if unused_bits > 0 && data.is_empty() {
            data.push(0);
        }
        PropertyValue::BitString {
            unused_bits: if data.is_empty() { 0 } else { unused_bits },
            data,
        }
    })
}

/// Every application-tagged primitive the owned decoder returns.
fn application_value() -> impl Strategy<Value = PropertyValue> {
    prop_oneof![
        Just(PropertyValue::Null),
        any::<bool>().prop_map(PropertyValue::Boolean),
        unsigned().prop_map(PropertyValue::Unsigned),
        signed().prop_map(PropertyValue::Signed),
        real().prop_map(PropertyValue::Real),
        double().prop_map(PropertyValue::Double),
        octet_string().prop_map(PropertyValue::OctetString),
        character_string().prop_map(PropertyValue::CharacterString),
        bit_string(),
        enumerated().prop_map(PropertyValue::Enumerated),
        date().prop_map(PropertyValue::Date),
        time().prop_map(PropertyValue::Time),
        object_identifier().prop_map(PropertyValue::ObjectIdentifier),
    ]
}

/// The borrowed view's value as the owned one it should equal, by `Debug`
/// text; a CharacterString by its text, which the view has only for UTF-8.
fn viewed(value: &ApplicationValueRef<'_>) -> String {
    use ApplicationValueRef as V;
    let owned = match *value {
        V::Null => PropertyValue::Null,
        V::Boolean(value) => PropertyValue::Boolean(value),
        V::Unsigned(value) => PropertyValue::Unsigned(value),
        V::Signed(value) => PropertyValue::Signed(value),
        V::Real(value) => PropertyValue::Real(value),
        V::Double(value) => PropertyValue::Double(value),
        V::OctetString(octets) => PropertyValue::OctetString(octets.to_vec()),
        V::CharacterString(text) => {
            PropertyValue::CharacterString(text.as_str().expect("UTF-8").to_owned())
        }
        V::BitString { unused_bits, data } => PropertyValue::BitString {
            unused_bits,
            data: data.to_vec(),
        },
        V::Enumerated(value) => PropertyValue::Enumerated(value),
        V::Date(value) => PropertyValue::Date(value),
        V::Time(value) => PropertyValue::Time(value),
        V::ObjectIdentifier(value) => PropertyValue::ObjectIdentifier(value),
    };
    format!("{owned:?}")
}

/// An application-tagged CharacterString in `charset`, its contents
/// already encoded.
fn character_string_in(charset: u8, contents: &[u8]) -> BytesMut {
    let mut buf = BytesMut::new();
    let length = u32::try_from(contents.len() + 1).unwrap();
    encode_tag(
        &mut buf,
        app_tag::CHARACTER_STRING,
        TagClass::Application,
        length,
    );
    buf.extend_from_slice(&[charset]);
    buf.extend_from_slice(contents);
    buf
}

fn array_index() -> impl Strategy<Value = Option<u32>> {
    prop_oneof![Just(None), any::<u32>().prop_map(Some)]
}

fn property_identifier() -> impl Strategy<Value = PropertyIdentifier> {
    (0..=ObjectIdentifier::MAX_INSTANCE).prop_map(PropertyIdentifier::from_raw)
}

fn calendar_entry() -> impl Strategy<Value = BACnetCalendarEntry> {
    prop_oneof![
        date().prop_map(BACnetCalendarEntry::Date),
        (date(), date()).prop_map(|(start_date, end_date)| {
            BACnetCalendarEntry::DateRange(BACnetDateRange {
                start_date,
                end_date,
            })
        }),
        any::<[u8; 3]>().prop_map(|[month, week_of_month, day_of_week]| {
            BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
                month,
                week_of_month,
                day_of_week,
            })
        }),
    ]
}

fn timestamp() -> impl Strategy<Value = BACnetTimeStamp> {
    prop_oneof![
        time().prop_map(BACnetTimeStamp::Time),
        any::<u16>().prop_map(BACnetTimeStamp::SequenceNumber),
        (date(), time()).prop_map(|(date, time)| BACnetTimeStamp::DateTime { date, time }),
    ]
}

fn recipient() -> impl Strategy<Value = BACnetRecipient> {
    prop_oneof![
        object_identifier().prop_map(BACnetRecipient::Device),
        (any::<u16>(), prop::collection::vec(any::<u8>(), 0..=6)).prop_map(
            |(network_number, mac)| BACnetRecipient::Address(BACnetAddress {
                network_number,
                mac_address: mac.as_slice().into(),
            })
        ),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    /// An application value reads back through the owned decoder and, the
    /// same value at the same length, through the borrowed view; the
    /// validator takes the encoding whole, so the encoder writes the
    /// shortest form.
    #[test]
    fn application_values(value in application_value()) {
        round_trip(&value, encode_property_value, decode_application_value)?;
        let mut buf = BytesMut::new();
        encode_property_value(&mut buf, &value).unwrap();
        let (view, end) = decode_application_value_ref(&buf, 0).unwrap();
        prop_assert_eq!(end, buf.len());
        prop_assert_eq!(viewed(&view), format!("{value:?}"));
        prop_assert_eq!(validate_application_value(&buf, 0).ok(), Some(buf.len()));
    }

    /// A CharacterString in UCS-2 or ISO 8859-1, which the encoder never
    /// writes, decodes to the same text as its UTF-8 form.
    #[test]
    fn other_character_sets(text in prop::collection::vec(any::<char>(), 0..80)) {
        let ucs2: String = text
            .iter()
            .copied()
            .filter(|&c| u32::from(c) <= 0xFFFF)
            .collect();
        let contents: Vec<u8> = ucs2
            .chars()
            .flat_map(|c| (u32::from(c) as u16).to_be_bytes())
            .collect();
        let buf = character_string_in(charset::UCS2, &contents);
        let (value, end) = decode_application_value(&buf, 0).unwrap();
        prop_assert_eq!(end, buf.len());
        prop_assert_eq!(value, PropertyValue::CharacterString(ucs2));

        let latin: String = text.iter().copied().filter(|&c| u32::from(c) <= 0xFF).collect();
        let contents: Vec<u8> = latin.chars().map(|c| u32::from(c) as u8).collect();
        let buf = character_string_in(charset::ISO_8859_1, &contents);
        let (value, end) = decode_application_value(&buf, 0).unwrap();
        prop_assert_eq!(end, buf.len());
        prop_assert_eq!(value, PropertyValue::CharacterString(latin));
    }

    /// A run of values, as an array property's elements travel, reads back
    /// one after another.
    #[test]
    fn application_value_runs(values in prop::collection::vec(application_value(), 0..8)) {
        let mut buf = BytesMut::new();
        encode_property_value(&mut buf, &PropertyValue::List(values.clone())).unwrap();
        let mut offset = 0;
        let mut decoded = Vec::new();
        while offset < buf.len() {
            let (value, end) = decode_application_value(&buf, offset).unwrap();
            decoded.push(value);
            offset = end;
        }
        prop_assert_eq!(format!("{decoded:?}"), format!("{values:?}"));
    }

    #[test]
    fn timestamps(value in timestamp()) {
        round_trip(&value, encode_timestamp_choice, decode_timestamp_choice)?;
    }

    #[test]
    fn date_ranges(start_date in date(), end_date in date()) {
        let range = BACnetDateRange { start_date, end_date };
        round_trip(&range, |buf, value| { encode_date_range(buf, value); Ok(()) }, decode_date_range)?;
    }

    #[test]
    fn calendar_entries(entry in calendar_entry()) {
        round_trip(&entry, |buf, value| { encode_calendar_entry(buf, value); Ok(()) }, decode_calendar_entry)?;
    }

    #[test]
    fn time_values(time in time(), value in application_value()) {
        let time_value = BACnetTimeValue { time, value };
        round_trip(&time_value, encode_time_value, decode_time_value)?;
    }

    #[test]
    fn object_property_references(
        object_identifier in object_identifier(),
        property_identifier in 0..=ObjectIdentifier::MAX_INSTANCE,
        property_array_index in array_index(),
    ) {
        let reference = BACnetObjectPropertyReference {
            object_identifier,
            property_identifier,
            property_array_index,
        };
        round_trip(
            &reference,
            |buf, value| { encode_object_property_reference(buf, value); Ok(()) },
            decode_object_property_reference_at,
        )?;
    }

    /// The list element of WritePropertyMultiple, CreateObject and the COV
    /// notifications, holding any run of application values.
    #[test]
    fn property_values(
        property_identifier in property_identifier(),
        property_array_index in array_index(),
        values in prop::collection::vec(application_value(), 0..4),
        priority in prop_oneof![Just(None), (1u8..=16).prop_map(Some)],
    ) {
        let mut value = BytesMut::new();
        encode_property_value(&mut value, &PropertyValue::List(values)).unwrap();
        let property_value = BACnetPropertyValue {
            property_identifier,
            property_array_index,
            value: value.to_vec(),
            priority,
        };
        round_trip(
            &property_value,
            |buf, value| encode_bacnet_property_value(value, buf),
            decode_bacnet_property_value,
        )?;
    }

    /// ReadPropertyMultiple's request element.
    #[test]
    fn read_access_specifications(
        object_identifier in object_identifier(),
        references in prop::collection::vec((property_identifier(), array_index()), 1..6),
    ) {
        let specification = ReadAccessSpecification {
            object_identifier,
            list_of_property_references: references
                .into_iter()
                .map(|(property_identifier, property_array_index)| PropertyReference {
                    property_identifier,
                    property_array_index,
                })
                .collect(),
        };
        round_trip(
            &specification,
            |buf, value| { encode_read_access_specification(buf, value); Ok(()) },
            decode_read_access_specification,
        )?;
    }

    /// The recipient of a notification class's destinations and of COV and
    /// event subscriptions.
    #[test]
    fn recipients(value in recipient()) {
        round_trip(&value, encode_recipient, decode_recipient)?;
    }
}
