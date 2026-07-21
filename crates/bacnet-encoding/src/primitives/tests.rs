use super::*;
use bacnet_types::enums::ObjectType;

fn encode_to_vec<F: FnOnce(&mut BytesMut)>(f: F) -> Vec<u8> {
    let mut buf = BytesMut::new();
    f(&mut buf);
    buf.to_vec()
}

// --- Raw unsigned ---

#[test]
fn unsigned_encode_decode_1byte() {
    let mut buf = BytesMut::new();
    encode_unsigned(&mut buf, 42);
    assert_eq!(&buf[..], &[42]);
    assert_eq!(decode_unsigned(&buf).unwrap(), 42);
}

#[test]
fn unsigned_encode_decode_2bytes() {
    let mut buf = BytesMut::new();
    encode_unsigned(&mut buf, 0x1234);
    assert_eq!(&buf[..], &[0x12, 0x34]);
    assert_eq!(decode_unsigned(&buf).unwrap(), 0x1234);
}

#[test]
fn unsigned_encode_decode_3bytes() {
    let mut buf = BytesMut::new();
    encode_unsigned(&mut buf, 0x12_3456);
    assert_eq!(&buf[..], &[0x12, 0x34, 0x56]);
    assert_eq!(decode_unsigned(&buf).unwrap(), 0x12_3456);
}

#[test]
fn unsigned_encode_decode_4bytes() {
    let mut buf = BytesMut::new();
    encode_unsigned(&mut buf, 0xDEAD_BEEF);
    assert_eq!(&buf[..], &[0xDE, 0xAD, 0xBE, 0xEF]);
    assert_eq!(decode_unsigned(&buf).unwrap(), 0xDEAD_BEEF);
}

#[test]
fn unsigned_zero() {
    let mut buf = BytesMut::new();
    encode_unsigned(&mut buf, 0);
    assert_eq!(&buf[..], &[0]);
    assert_eq!(decode_unsigned(&buf).unwrap(), 0);
}

#[test]
fn unsigned_max_u64() {
    let mut buf = BytesMut::new();
    encode_unsigned(&mut buf, u64::MAX);
    assert_eq!(buf.len(), 8);
    assert_eq!(decode_unsigned(&buf).unwrap(), u64::MAX);
}

// --- Raw signed ---

#[test]
fn signed_encode_decode_positive() {
    let mut buf = BytesMut::new();
    encode_signed(&mut buf, 42);
    assert_eq!(&buf[..], &[42]);
    assert_eq!(decode_signed(&buf).unwrap(), 42);
}

#[test]
fn signed_encode_decode_negative() {
    let mut buf = BytesMut::new();
    encode_signed(&mut buf, -1);
    assert_eq!(&buf[..], &[0xFF]);
    assert_eq!(decode_signed(&buf).unwrap(), -1);
}

#[test]
fn signed_encode_decode_neg128() {
    let mut buf = BytesMut::new();
    encode_signed(&mut buf, -128);
    assert_eq!(&buf[..], &[0x80]);
    assert_eq!(decode_signed(&buf).unwrap(), -128);
}

#[test]
fn signed_encode_decode_neg129() {
    let mut buf = BytesMut::new();
    encode_signed(&mut buf, -129);
    assert_eq!(&buf[..], &[0xFF, 0x7F]);
    assert_eq!(decode_signed(&buf).unwrap(), -129);
}

#[test]
fn signed_encode_decode_min() {
    let mut buf = BytesMut::new();
    encode_signed(&mut buf, i32::MIN);
    assert_eq!(buf.len(), 4);
    assert_eq!(decode_signed(&buf).unwrap(), i32::MIN);
}

#[test]
fn signed_encode_decode_max() {
    let mut buf = BytesMut::new();
    encode_signed(&mut buf, i32::MAX);
    assert_eq!(buf.len(), 4);
    assert_eq!(decode_signed(&buf).unwrap(), i32::MAX);
}

// --- Real / Double ---

#[test]
fn real_round_trip() {
    let mut buf = BytesMut::new();
    encode_real(&mut buf, 72.5);
    assert_eq!(decode_real(&buf).unwrap(), 72.5);
}

#[test]
fn double_round_trip() {
    let mut buf = BytesMut::new();
    encode_double(&mut buf, core::f64::consts::PI);
    assert_eq!(decode_double(&buf).unwrap(), core::f64::consts::PI);
}

#[test]
fn fixed_width_application_values_reject_overlong_tags() {
    for (tag, len) in [
        (app_tag::REAL, 5),
        (app_tag::DOUBLE, 9),
        (app_tag::DATE, 5),
        (app_tag::TIME, 5),
        (app_tag::OBJECT_IDENTIFIER, 5),
    ] {
        let mut bytes = BytesMut::new();
        tags::encode_tag(&mut bytes, tag, TagClass::Application, len);
        bytes.extend_from_slice(&vec![0; len as usize]);
        assert!(
            decode_application_value(&bytes, 0).is_err(),
            "tag {tag} with length {len} should be rejected"
        );
    }
}

// --- Character string ---

#[test]
fn character_string_round_trip() {
    let mut buf = BytesMut::new();
    encode_character_string(&mut buf, "hello");
    let decoded = decode_character_string(&buf).unwrap();
    assert_eq!(decoded, "hello");
}

#[test]
fn character_string_empty() {
    let mut buf = BytesMut::new();
    encode_character_string(&mut buf, "");
    let decoded = decode_character_string(&buf).unwrap();
    assert_eq!(decoded, "");
}

// --- Bit string ---

#[test]
fn bit_string_round_trip() {
    let mut buf = BytesMut::new();
    encode_bit_string(&mut buf, 3, &[0b1010_0000]);
    let (unused, data) = decode_bit_string(&buf).unwrap();
    assert_eq!(unused, 3);
    assert_eq!(data, vec![0b1010_0000]);
}

#[test]
fn bit_string_invalid_unused() {
    assert!(decode_bit_string(&[8]).is_err());
}

// --- Application-tagged encode/decode round trips ---

#[test]
fn app_null_round_trip() {
    let bytes = encode_to_vec(encode_app_null);
    let (val, offset) = decode_application_value(&bytes, 0).unwrap();
    assert_eq!(val, PropertyValue::Null);
    assert_eq!(offset, bytes.len());
}

#[test]
fn app_boolean_true_round_trip() {
    let bytes = encode_to_vec(|buf| encode_app_boolean(buf, true));
    let (val, _) = decode_application_value(&bytes, 0).unwrap();
    assert_eq!(val, PropertyValue::Boolean(true));
}

#[test]
fn app_boolean_false_round_trip() {
    let bytes = encode_to_vec(|buf| encode_app_boolean(buf, false));
    let (val, _) = decode_application_value(&bytes, 0).unwrap();
    assert_eq!(val, PropertyValue::Boolean(false));
}

#[test]
fn app_unsigned_round_trip() {
    for &v in &[0u64, 1, 255, 256, 65535, 65536, 0xFFFF_FFFF, u64::MAX] {
        let bytes = encode_to_vec(|buf| encode_app_unsigned(buf, v));
        let (val, end) = decode_application_value(&bytes, 0).unwrap();
        assert_eq!(val, PropertyValue::Unsigned(v), "failed for {v}");
        assert_eq!(end, bytes.len());
    }
}

#[test]
fn app_signed_round_trip() {
    for &v in &[0i32, 1, -1, 127, -128, 128, -129, i32::MIN, i32::MAX] {
        let bytes = encode_to_vec(|buf| encode_app_signed(buf, v));
        let (val, end) = decode_application_value(&bytes, 0).unwrap();
        assert_eq!(val, PropertyValue::Signed(v), "failed for {v}");
        assert_eq!(end, bytes.len());
    }
}

#[test]
fn app_real_round_trip() {
    let bytes = encode_to_vec(|buf| encode_app_real(buf, 72.5));
    let (val, end) = decode_application_value(&bytes, 0).unwrap();
    assert_eq!(val, PropertyValue::Real(72.5));
    assert_eq!(end, bytes.len());
}

#[test]
fn app_double_round_trip() {
    let bytes = encode_to_vec(|buf| encode_app_double(buf, core::f64::consts::PI));
    let (val, end) = decode_application_value(&bytes, 0).unwrap();
    assert_eq!(val, PropertyValue::Double(core::f64::consts::PI));
    assert_eq!(end, bytes.len());
}

#[test]
fn app_octet_string_round_trip() {
    let data = vec![0xDE, 0xAD, 0xBE, 0xEF];
    let bytes = encode_to_vec(|buf| encode_app_octet_string(buf, &data));
    let (val, end) = decode_application_value(&bytes, 0).unwrap();
    assert_eq!(val, PropertyValue::OctetString(data));
    assert_eq!(end, bytes.len());
}

#[test]
fn app_character_string_round_trip() {
    let bytes = encode_to_vec(|buf| encode_app_character_string(buf, "BACnet").unwrap());
    let (val, end) = decode_application_value(&bytes, 0).unwrap();
    assert_eq!(val, PropertyValue::CharacterString("BACnet".into()));
    assert_eq!(end, bytes.len());
}

#[test]
fn app_enumerated_round_trip() {
    let bytes = encode_to_vec(|buf| encode_app_enumerated(buf, 8));
    let (val, end) = decode_application_value(&bytes, 0).unwrap();
    assert_eq!(val, PropertyValue::Enumerated(8));
    assert_eq!(end, bytes.len());
}

#[test]
fn app_date_round_trip() {
    let date = Date {
        year: 124,
        month: 6,
        day: 15,
        day_of_week: 6,
    };
    let bytes = encode_to_vec(|buf| encode_app_date(buf, &date));
    let (val, end) = decode_application_value(&bytes, 0).unwrap();
    assert_eq!(val, PropertyValue::Date(date));
    assert_eq!(end, bytes.len());
}

#[test]
fn app_time_round_trip() {
    let time = Time {
        hour: 14,
        minute: 30,
        second: 0,
        hundredths: 0,
    };
    let bytes = encode_to_vec(|buf| encode_app_time(buf, &time));
    let (val, end) = decode_application_value(&bytes, 0).unwrap();
    assert_eq!(val, PropertyValue::Time(time));
    assert_eq!(end, bytes.len());
}

#[test]
fn app_object_id_round_trip() {
    let oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
    let bytes = encode_to_vec(|buf| encode_app_object_id(buf, &oid));
    let (val, end) = decode_application_value(&bytes, 0).unwrap();
    assert_eq!(val, PropertyValue::ObjectIdentifier(oid));
    assert_eq!(end, bytes.len());
}

#[test]
fn app_bit_string_round_trip() {
    let bytes = encode_to_vec(|buf| encode_app_bit_string(buf, 4, &[0xF0]));
    let (val, end) = decode_application_value(&bytes, 0).unwrap();
    assert_eq!(
        val,
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0xF0],
        }
    );
    assert_eq!(end, bytes.len());
}

// --- PropertyValue encode/decode round trip ---

#[test]
fn property_value_encode_decode_all_types() {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let values = vec![
        PropertyValue::Null,
        PropertyValue::Boolean(true),
        PropertyValue::Boolean(false),
        PropertyValue::Unsigned(12345),
        PropertyValue::Signed(-42),
        PropertyValue::Real(72.5),
        PropertyValue::Double(3.125),
        PropertyValue::OctetString(vec![1, 2, 3]),
        PropertyValue::CharacterString("test".into()),
        PropertyValue::BitString {
            unused_bits: 0,
            data: vec![0xFF],
        },
        PropertyValue::Enumerated(8),
        PropertyValue::Date(Date {
            year: 124,
            month: 1,
            day: 1,
            day_of_week: 1,
        }),
        PropertyValue::Time(Time {
            hour: 12,
            minute: 0,
            second: 0,
            hundredths: 0,
        }),
        PropertyValue::ObjectIdentifier(oid),
    ];

    for original in &values {
        let bytes = encode_to_vec(|buf| encode_property_value(buf, original).unwrap());
        let (decoded, end) = decode_application_value(&bytes, 0).unwrap();
        assert_eq!(&decoded, original, "round-trip failed for {original:?}");
        assert_eq!(end, bytes.len(), "offset mismatch for {original:?}");
    }
}

// --- Context-tagged encode ---

fn context_tagged(tag_number: u32, value: PropertyValue) -> PropertyValue {
    PropertyValue::ContextTagged {
        tag_number,
        value: Box::new(value),
    }
}

#[test]
fn property_value_context_tagged_encodes_all_primitive_contents() {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let cases = vec![
        (context_tagged(0, PropertyValue::Null), vec![0x08]),
        (
            context_tagged(1, PropertyValue::Boolean(true)),
            vec![0x19, 0x01],
        ),
        (
            context_tagged(2, PropertyValue::Unsigned(42)),
            vec![0x29, 0x2a],
        ),
        (
            context_tagged(3, PropertyValue::Signed(-1)),
            vec![0x39, 0xff],
        ),
        (
            context_tagged(4, PropertyValue::Real(1.0)),
            vec![0x4c, 0x3f, 0x80, 0x00, 0x00],
        ),
        (
            context_tagged(5, PropertyValue::Double(1.0)),
            vec![0x5d, 0x08, 0x3f, 0xf0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ),
        (
            context_tagged(6, PropertyValue::OctetString(vec![0xaa])),
            vec![0x69, 0xaa],
        ),
        (
            context_tagged(7, PropertyValue::CharacterString("A".into())),
            vec![0x7a, 0x00, 0x41],
        ),
        (
            context_tagged(
                8,
                PropertyValue::BitString {
                    unused_bits: 0,
                    data: vec![0xff],
                },
            ),
            vec![0x8a, 0x00, 0xff],
        ),
        (
            context_tagged(9, PropertyValue::Enumerated(1)),
            vec![0x99, 0x01],
        ),
        (
            context_tagged(
                10,
                PropertyValue::Date(Date {
                    year: 124,
                    month: 1,
                    day: 2,
                    day_of_week: 2,
                }),
            ),
            vec![0xac, 124, 1, 2, 2],
        ),
        (
            context_tagged(
                11,
                PropertyValue::Time(Time {
                    hour: 12,
                    minute: 34,
                    second: 56,
                    hundredths: 78,
                }),
            ),
            vec![0xbc, 12, 34, 56, 78],
        ),
        (
            context_tagged(12, PropertyValue::ObjectIdentifier(oid)),
            vec![0xcc, 0x00, 0x00, 0x00, 0x01],
        ),
    ];

    for (value, expected) in cases {
        let bytes = encode_to_vec(|buf| encode_property_value(buf, &value).unwrap());
        assert_eq!(bytes, expected, "wrong encoding for {value:?}");
    }
}

#[test]
fn property_value_context_tagged_list_uses_opening_and_closing_tags() {
    let value = context_tagged(
        20,
        PropertyValue::List(vec![
            PropertyValue::Unsigned(1),
            context_tagged(2, PropertyValue::Enumerated(3)),
        ]),
    );

    let bytes = encode_to_vec(|buf| encode_property_value(buf, &value).unwrap());
    assert_eq!(bytes, vec![0xfe, 20, 0x21, 0x01, 0x29, 0x03, 0xff, 20]);
}

#[test]
fn property_value_context_tagged_rejects_nested_context_tagged_values() {
    let value = context_tagged(20, context_tagged(2, PropertyValue::Unsigned(1)));
    let mut bytes = BytesMut::new();

    let error = encode_property_value(&mut bytes, &value).unwrap_err();

    assert!(
        matches!(error, Error::Encoding(_)),
        "unexpected error: {error}"
    );
    assert!(error.to_string().contains("nested context-tagged"));
    assert!(bytes.is_empty());
}

#[test]
fn property_value_context_tagged_supports_extended_tag_numbers() {
    let value = context_tagged(254, PropertyValue::Real(1.0));
    let bytes = encode_to_vec(|buf| encode_property_value(buf, &value).unwrap());
    assert_eq!(bytes, vec![0xfc, 254, 0x3f, 0x80, 0x00, 0x00]);
}

#[test]
fn property_value_context_tagged_rejects_unrepresentable_tag_numbers() {
    for tag_number in [255, 256, u32::MAX] {
        let value = context_tagged(tag_number, PropertyValue::Null);
        let mut bytes = BytesMut::new();
        let error = encode_property_value(&mut bytes, &value).unwrap_err();
        assert!(
            error.to_string().contains("context tag number"),
            "unexpected error for tag {tag_number}: {error}"
        );
        assert!(bytes.is_empty());
    }
}

#[test]
fn property_value_context_tagged_list_error_preserves_existing_buffer() {
    let value = context_tagged(
        20,
        PropertyValue::List(vec![
            PropertyValue::Unsigned(1),
            context_tagged(255, PropertyValue::Null),
        ]),
    );
    let sentinel = [0xDE, 0xAD, 0xBE, 0xEF];
    let mut bytes = BytesMut::from(&sentinel[..]);

    let error = encode_property_value(&mut bytes, &value).unwrap_err();

    assert!(error.to_string().contains("context tag number 255"));
    assert_eq!(&bytes[..], sentinel);
}

#[test]
fn ctx_unsigned_encoding() {
    let bytes = encode_to_vec(|buf| encode_ctx_unsigned(buf, 1, 42));
    // Context tag 1, length 1, value 42
    assert_eq!(bytes, vec![0x19, 42]);
}

#[test]
fn ctx_boolean_encoding() {
    let bytes = encode_to_vec(|buf| encode_ctx_boolean(buf, 0, true));
    // Context tag 0, length 1, value 1
    assert_eq!(bytes, vec![0x09, 0x01]);
}

#[test]
fn ctx_object_id_encoding() {
    let oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
    let bytes = encode_to_vec(|buf| encode_ctx_object_id(buf, 2, &oid));
    // Context tag 2, length 4: (2<<4) | (1<<3) | 4 = 0x2C
    let mut expected = vec![0x2C];
    expected.extend_from_slice(&oid.encode());
    assert_eq!(bytes, expected);
}

// --- BACnetTimeStamp encode/decode ---

#[test]
fn timestamp_sequence_number_round_trip() {
    let ts = BACnetTimeStamp::SequenceNumber(42);
    let mut buf = BytesMut::new();
    encode_timestamp(&mut buf, 3, &ts);
    let (decoded, end) = decode_timestamp(&buf, 0, 3).unwrap();
    assert_eq!(decoded, ts);
    assert_eq!(end, buf.len());
}

#[test]
fn timestamp_time_round_trip() {
    let ts = BACnetTimeStamp::Time(Time {
        hour: 14,
        minute: 30,
        second: 45,
        hundredths: 50,
    });
    let mut buf = BytesMut::new();
    encode_timestamp(&mut buf, 3, &ts);
    let (decoded, end) = decode_timestamp(&buf, 0, 3).unwrap();
    assert_eq!(decoded, ts);
    assert_eq!(end, buf.len());
}

#[test]
fn timestamp_datetime_round_trip() {
    let ts = BACnetTimeStamp::DateTime {
        date: Date {
            year: 126,
            month: 2,
            day: 28,
            day_of_week: 6,
        },
        time: Time {
            hour: 10,
            minute: 15,
            second: 0,
            hundredths: 0,
        },
    };
    let mut buf = BytesMut::new();
    encode_timestamp(&mut buf, 5, &ts);
    let (decoded, end) = decode_timestamp(&buf, 0, 5).unwrap();
    assert_eq!(decoded, ts);
    assert_eq!(end, buf.len());
}

#[test]
fn ucs2_decode_ascii() {
    // UCS-2 BE for "AB": 0x00 0x41 0x00 0x42
    let data = [charset::UCS2, 0x00, 0x41, 0x00, 0x42];
    let result = decode_character_string(&data).unwrap();
    assert_eq!(result, "AB");
}

#[test]
fn ucs2_decode_non_ascii() {
    // UCS-2 BE for "é" (U+00E9): 0x00 0xE9
    let data = [charset::UCS2, 0x00, 0xE9];
    let result = decode_character_string(&data).unwrap();
    assert_eq!(result, "é");
}

#[test]
fn unsupported_charset_errors() {
    for &cs in &[
        charset::IBM_MICROSOFT_DBCS,
        charset::JIS_X_0208,
        charset::UCS4,
    ] {
        let data = [cs, 0x41, 0x42];
        let result = decode_character_string(&data);
        assert!(result.is_err(), "charset {cs} should return an error");
    }
}
