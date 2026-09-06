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

#[test]
fn signed_rejects_redundant_sign_octets() {
    for encoded in [
        &[0x00, 0x01][..],
        &[0xFF, 0x80],
        &[0x00, 0x7F],
        &[0xFF, 0xFF],
    ] {
        assert!(
            decode_signed_canonical(encoded).is_err(),
            "accepted {encoded:02x?}"
        );
    }
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
fn app_enumerated_rejects_values_exceeding_u32() {
    for content in [
        &[0x01, 0x00, 0x00, 0x00, 0x00][..],
        &[0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02][..],
    ] {
        let mut bytes = BytesMut::new();
        tags::encode_tag(
            &mut bytes,
            app_tag::ENUMERATED,
            TagClass::Application,
            u32::try_from(content.len()).unwrap(),
        );
        bytes.extend_from_slice(content);

        assert!(decode_application_value(&bytes, 0).is_err());
    }
}

#[test]
fn app_enumerated_accepts_representable_values_with_leading_zeroes() {
    for content in [
        &[0x00, 0xFF, 0xFF, 0xFF, 0xFF][..],
        &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08][..],
    ] {
        let mut bytes = BytesMut::new();
        tags::encode_tag(
            &mut bytes,
            app_tag::ENUMERATED,
            TagClass::Application,
            u32::try_from(content.len()).unwrap(),
        );
        bytes.extend_from_slice(content);

        let (value, end) = decode_application_value(&bytes, 0).unwrap();
        assert_eq!(
            value,
            PropertyValue::Enumerated(u32::try_from(decode_unsigned(content).unwrap()).unwrap())
        );
        assert_eq!(end, bytes.len());
    }
}

/// Standard 135-2020 Clause 21 pins `record-access (0)` / `stream-access (1)`
/// for `BACnetFileAccessMethod`; on the wire the enumerated application tag
/// is 0x91 with the value octet after it (Clause 20.2.4). Guard against the
/// #273 swap regression at the encoder that puts the value on the wire.
#[test]
fn file_access_method_wire_values_match_clause_21() {
    use bacnet_types::enums::FileAccessMethod;

    let bytes =
        encode_to_vec(|buf| encode_app_enumerated(buf, FileAccessMethod::RECORD_ACCESS.to_raw()));
    assert_eq!(bytes, [0x91, 0x00], "record-access encodes to value 0");

    let bytes =
        encode_to_vec(|buf| encode_app_enumerated(buf, FileAccessMethod::STREAM_ACCESS.to_raw()));
    assert_eq!(bytes, [0x91, 0x01], "stream-access encodes to value 1");
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
    encode_timestamp(&mut buf, 3, &ts).unwrap();
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
    encode_timestamp(&mut buf, 3, &ts).unwrap();
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
    encode_timestamp(&mut buf, 5, &ts).unwrap();
    let (decoded, end) = decode_timestamp(&buf, 0, 5).unwrap();
    assert_eq!(decoded, ts);
    assert_eq!(end, buf.len());
}

// --- BACnetTimeStamp golden wire vectors (Clause 20.2.1.5 + Clause 21) ---

#[test]
fn timestamp_time_choice_golden_primitive_context_tag() {
    // time [0] tags the PRIMITIVE base type Time: a context-specific
    // PRIMITIVE tag 0 of length 4 holding the raw Time octets —
    // (0<<4)|(1<<3)|4 = 0x0C. An opening-tag-0 wrapper around an
    // application-tagged Time is NOT conformant.
    let ts = BACnetTimeStamp::Time(Time {
        hour: 14,
        minute: 30,
        second: 45,
        hundredths: 50,
    });
    let mut buf = BytesMut::new();
    encode_timestamp_choice(&mut buf, &ts).unwrap();
    assert_eq!(buf.as_ref(), &[0x0C, 14, 30, 45, 50]);
    // The wrapped form adds the enclosing field's opening/closing pair.
    let mut wrapped = BytesMut::new();
    encode_timestamp(&mut wrapped, 3, &ts).unwrap();
    assert_eq!(wrapped.as_ref(), &[0x3E, 0x0C, 14, 30, 45, 50, 0x3F]);
    // And the bare decoder reads both forms' inner content identically.
    let (decoded, end) = decode_timestamp_choice(&wrapped, 1).unwrap();
    assert_eq!(decoded, ts);
    assert_eq!(end, 6);
}

#[test]
fn timestamp_sequence_number_choice_golden() {
    // sequence-number [1]: context tag 1, minimal big-endian contents.
    let ts = BACnetTimeStamp::SequenceNumber(42);
    let mut buf = BytesMut::new();
    encode_timestamp_choice(&mut buf, &ts).unwrap();
    // (1<<4)|(1<<3)|1 = 0x19
    assert_eq!(buf.as_ref(), &[0x19, 42]);
    let (decoded, end) = decode_timestamp_choice(&buf, 0).unwrap();
    assert_eq!(decoded, ts);
    assert_eq!(end, buf.len());
}

#[test]
fn timestamp_datetime_choice_golden() {
    // datetime [2] tags the CONSTRUCTED BACnetDateTime: opening tag 2 /
    // application-tagged Date / application-tagged Time / closing tag 2.
    let ts = BACnetTimeStamp::DateTime {
        date: Date {
            year: 126, // 2026
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
    encode_timestamp_choice(&mut buf, &ts).unwrap();
    assert_eq!(
        buf.as_ref(),
        &[
            0x2E, // opening tag 2
            0xA4, 126, 2, 28, 6, // application Date (tag 10, len 4)
            0xB4, 10, 15, 0, 0,    // application Time (tag 11, len 4)
            0x2F, // closing tag 2
        ]
    );
    let (decoded, end) = decode_timestamp_choice(&buf, 0).unwrap();
    assert_eq!(decoded, ts);
    assert_eq!(end, buf.len());
}

#[test]
fn timestamp_sequence_number_boundaries() {
    // Clause 21: sequence-number is Unsigned (0..65535).
    for n in [0u16, u16::MAX] {
        let ts = BACnetTimeStamp::SequenceNumber(n);
        let mut buf = BytesMut::new();
        encode_timestamp_choice(&mut buf, &ts).unwrap();
        let (decoded, _) = decode_timestamp_choice(&buf, 0).unwrap();
        assert_eq!(decoded, ts);
        let mut wrapped = BytesMut::new();
        encode_timestamp(&mut wrapped, 3, &ts).unwrap();
        let (decoded, _) = decode_timestamp(&wrapped, 0, 3).unwrap();
        assert_eq!(decoded, ts);
    }
    // 65535 encodes in exactly two content octets, 0 in one.
    let mut buf = BytesMut::new();
    encode_timestamp_choice(&mut buf, &BACnetTimeStamp::SequenceNumber(65535)).unwrap();
    assert_eq!(buf.as_ref(), &[0x1A, 0xFF, 0xFF]);
}

#[test]
fn timestamp_sequence_number_over_65535_rejected_from_wire() {
    // The public enum carries u16, so local construction cannot exceed the
    // production's range. Hostile wire input still must be rejected.
    let data = [0x3E, 0x1B, 0x01, 0x00, 0x00, 0x3F];
    assert!(
        decode_timestamp(&data, 0, 3).is_err(),
        "decode of 65536 must fail"
    );
    assert!(decode_timestamp_choice(&data, 1).is_err());
}

#[test]
fn timestamp_decode_rejects_non_conformant_time_form() {
    // The non-canonical time [0] encoding (opening tag 0 / application Time /
    // closing tag 0) must be rejected: neither bare nor wrapped decode
    // accepts it.
    let non_conformant = [0x3E, 0x0E, 0xB4, 14, 30, 45, 50, 0x0F, 0x3F];
    assert!(decode_timestamp(&non_conformant, 0, 3).is_err());
    assert!(decode_timestamp_choice(&non_conformant, 1).is_err());
}

#[test]
fn timestamp_decode_wrong_outer_tag_rejected() {
    let ts = BACnetTimeStamp::SequenceNumber(42);
    let mut buf = BytesMut::new();
    encode_timestamp(&mut buf, 3, &ts).unwrap();
    assert!(
        decode_timestamp(&buf, 0, 4).is_err(),
        "outer tag [4] != [3]"
    );
}

#[test]
fn timestamp_decode_truncated_rejected() {
    let ts = BACnetTimeStamp::SequenceNumber(42);
    let mut buf = BytesMut::new();
    encode_timestamp(&mut buf, 3, &ts).unwrap();
    for cut in 1..buf.len() {
        assert!(
            decode_timestamp(&buf[..cut], 0, 3).is_err(),
            "truncated at {cut} bytes must fail"
        );
    }
}

#[test]
fn timestamp_decode_wrong_class_bit_rejected() {
    // Application-tagged content where the CHOICE's context tags belong:
    // app Unsigned (0x21) instead of ctx 1 (0x19) for sequence-number.
    let data = [0x3E, 0x21, 42, 0x3F];
    assert!(decode_timestamp(&data, 0, 3).is_err());
    assert!(decode_timestamp_choice(&data, 1).is_err());
    // Bare decode likewise refuses an application tag outright.
    assert!(decode_timestamp_choice(&[0x21, 42], 0).is_err());
}

// --- PropertyValue::ApplicationData (context-tagged content) ---

#[test]
fn application_data_decode_primitive_context_tag() {
    // A primitive context-tagged element is preserved verbatim, tag header
    // included, so the property-level framed codec can interpret it.
    // e.g. FaultNone: ctx 0, no contents.
    let data = [0x08];
    let (val, end) = decode_application_value(&data, 0).unwrap();
    assert_eq!(val, PropertyValue::ApplicationData(vec![0x08]));
    assert_eq!(end, 1);

    // ctx tag 0, len 4 (a Time content under [0]).
    let data = [0x0C, 14, 30, 45, 50];
    let (val, end) = decode_application_value(&data, 0).unwrap();
    assert_eq!(val, PropertyValue::ApplicationData(data.to_vec()));
    assert_eq!(end, data.len());
}

#[test]
fn application_data_decode_constructed_spans_closing_tag() {
    // opening [5], REAL, closing [5] — preserved whole.
    let data = [0x5E, 0x44, 0x42, 0x90, 0x00, 0x00, 0x5F];
    let (val, end) = decode_application_value(&data, 0).unwrap();
    assert_eq!(val, PropertyValue::ApplicationData(data.to_vec()));
    assert_eq!(end, data.len());
}

#[test]
fn application_data_decode_nested_constructed() {
    // Nested same-tag pairs balance: open 5, open 5, close 5, close 5.
    let data = [0x5E, 0x5E, 0x5F, 0x5F];
    let (val, end) = decode_application_value(&data, 0).unwrap();
    assert_eq!(val, PropertyValue::ApplicationData(data.to_vec()));
    assert_eq!(end, data.len());
    // Trailing application-tagged items decode independently after it.
    let data = [0x5E, 0x5F, 0x21, 0x2A];
    let (val, end) = decode_application_value(&data, 0).unwrap();
    assert_eq!(val, PropertyValue::ApplicationData(vec![0x5E, 0x5F]));
    assert_eq!(end, 2);
    let (val2, end2) = decode_application_value(&data, end).unwrap();
    assert_eq!(val2, PropertyValue::Unsigned(42));
    assert_eq!(end2, data.len());
}

#[test]
fn application_data_decode_unbalanced_constructed_rejected() {
    // opening without closing.
    assert!(decode_application_value(&[0x5E, 0x21, 0x01], 0).is_err());
    // closing tag alone.
    assert!(decode_application_value(&[0x5F], 0).is_err());
}

#[test]
fn application_data_encode_verbatim() {
    let bytes = vec![0x5E, 0x5F];
    let encoded = encode_to_vec(|buf| {
        encode_property_value(buf, &PropertyValue::ApplicationData(bytes.clone())).unwrap()
    });
    assert_eq!(encoded, bytes);
    // Inside a List it concatenates like any other value.
    let encoded = encode_to_vec(|buf| {
        encode_property_value(
            buf,
            &PropertyValue::List(vec![
                PropertyValue::Unsigned(1),
                PropertyValue::ApplicationData(bytes.clone()),
            ]),
        )
        .unwrap()
    });
    assert_eq!(encoded, vec![0x21, 0x01, 0x5E, 0x5F]);
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
