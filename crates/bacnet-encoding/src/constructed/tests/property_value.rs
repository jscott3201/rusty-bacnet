//! `BACnetPropertyValue` encode and decode.

use super::*;
use crate::tags::TagClass;
use bacnet_types::constructed::BACnetPropertyValue;
use bacnet_types::enums::PropertyIdentifier;
use bytes::BufMut;

fn encode_context_bytes(buf: &mut BytesMut, tag: u8, value: &[u8]) {
    tags::encode_tag(
        buf,
        tag,
        TagClass::Context,
        u32::try_from(value.len()).unwrap(),
    );
    buf.put_slice(value);
}

fn append_null_value(buf: &mut BytesMut) {
    tags::encode_opening_tag(buf, 2);
    primitives::encode_app_null(buf);
    tags::encode_closing_tag(buf, 2);
}

#[test]
fn bacnet_property_value_round_trip() {
    let pv = BACnetPropertyValue {
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        value: vec![0x44, 0x42, 0x90, 0x00, 0x00], // app-tagged Real 72.5
        priority: None,
    };
    let mut buf = BytesMut::new();
    encode_bacnet_property_value(&pv, &mut buf).unwrap();
    let (decoded, _) = decode_bacnet_property_value(&buf, 0).unwrap();
    assert_eq!(pv, decoded);
}

#[test]
fn bacnet_property_value_stops_at_the_next_concatenated_value() {
    let first = BACnetPropertyValue {
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        value: vec![0x10],
        priority: None,
    };
    let second = BACnetPropertyValue {
        property_identifier: PropertyIdentifier::STATUS_FLAGS,
        property_array_index: None,
        value: vec![0x00],
        priority: None,
    };
    let mut buf = BytesMut::new();
    encode_bacnet_property_value(&first, &mut buf).unwrap();
    let first_end = buf.len();
    encode_bacnet_property_value(&second, &mut buf).unwrap();

    let (decoded_first, next) = decode_bacnet_property_value(&buf, 0).unwrap();
    let (decoded_second, end) = decode_bacnet_property_value(&buf, next).unwrap();

    assert_eq!(decoded_first, first);
    assert_eq!(decoded_second, second);
    assert_eq!(next, first_end);
    assert_eq!(end, buf.len());
}

#[test]
fn bacnet_property_value_with_all_fields() {
    let pv = BACnetPropertyValue {
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: Some(5),
        value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
        priority: Some(8),
    };
    let mut buf = BytesMut::new();
    encode_bacnet_property_value(&pv, &mut buf).unwrap();
    let (decoded, _) = decode_bacnet_property_value(&buf, 0).unwrap();
    assert_eq!(pv, decoded);
}

#[test]
fn bacnet_property_value_preserves_legacy_event_parameters() {
    let pv = BACnetPropertyValue {
        property_identifier: PropertyIdentifier::EVENT_PARAMETERS,
        property_array_index: None,
        value: vec![0xfe, 0xff, 1, 0xff, 0xff, 0x2f, 2, 0xff, 0xff],
        priority: None,
    };
    // The inner `ff ff 2f` could end the value, so the encoder refuses it
    // (#2376) and leaves the buffer as it was.
    let mut buf = BytesMut::from(&[0xaa][..]);
    assert!(encode_bacnet_property_value(&pv, &mut buf).is_err());
    assert_eq!(buf.as_ref(), [0xaa]);

    // From an older peer it still decodes: `02` after that pair is no valid
    // next field, so the pair is payload.
    let mut wire = vec![0x09, 0x53, 0x2e];
    wire.extend_from_slice(&pv.value);
    wire.push(0x2f);
    let (decoded, consumed) = decode_bacnet_property_value(&wire, 0).unwrap();
    assert_eq!(decoded, pv);
    assert_eq!(consumed, wire.len());
}

#[test]
fn bacnet_property_value_rejects_unclosed_legacy_event_parameters() {
    let pv = BACnetPropertyValue {
        property_identifier: PropertyIdentifier::EVENT_PARAMETERS,
        property_array_index: None,
        value: vec![0xfe, 0xff, 1, 2],
        priority: None,
    };
    let mut buf = BytesMut::new();
    assert!(encode_bacnet_property_value(&pv, &mut buf).is_err());
    assert!(buf.is_empty());

    let wire = [0x09, 0x53, 0x2e, 0xfe, 0xff, 1, 2, 0x2f];
    assert!(decode_bacnet_property_value(&wire, 0).is_err());
}

#[test]
fn bacnet_property_value_priority_validation() {
    let pv = BACnetPropertyValue {
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        value: vec![0x10], // app boolean true
        priority: None,
    };
    let mut base = BytesMut::new();
    encode_bacnet_property_value(&pv, &mut base).unwrap();

    for priority in [0, 17, 257, 272, u64::MAX] {
        let mut encoded = base.clone();
        primitives::encode_ctx_unsigned(&mut encoded, 3, priority);
        let error = decode_bacnet_property_value(&encoded, 0).unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&format!("priority {priority} out of range 1-16")),
            "unexpected error for priority {priority}: {error}"
        );
    }

    let mut leading_zero = base;
    // Context tag 3 with a two-octet leading-zero encoding of numeric 1.
    leading_zero.extend_from_slice(&[0x3A, 0x00, 0x01]);
    let (decoded, consumed) = decode_bacnet_property_value(&leading_zero, 0).unwrap();
    assert_eq!(decoded.priority, Some(1));
    assert_eq!(consumed, leading_zero.len());
}

#[test]
fn shared_property_values_must_fit_u32() {
    let max_with_leading_zero = [0, 0xFF, 0xFF, 0xFF, 0xFF];

    let mut reference = BytesMut::new();
    encode_context_bytes(&mut reference, 0, &max_with_leading_zero);
    encode_context_bytes(&mut reference, 1, &max_with_leading_zero);
    let (decoded, consumed) = decode_property_reference(&reference, 0).unwrap();
    assert_eq!(decoded.property_identifier.to_raw(), u32::MAX);
    assert_eq!(decoded.property_array_index, Some(u32::MAX));
    assert_eq!(consumed, reference.len());

    let mut property_value = BytesMut::new();
    encode_context_bytes(&mut property_value, 0, &max_with_leading_zero);
    encode_context_bytes(&mut property_value, 1, &max_with_leading_zero);
    append_null_value(&mut property_value);
    let (decoded, consumed) = decode_bacnet_property_value(&property_value, 0).unwrap();
    assert_eq!(decoded.property_identifier.to_raw(), u32::MAX);
    assert_eq!(decoded.property_array_index, Some(u32::MAX));
    assert_eq!(consumed, property_value.len());

    for overflow in [u32::MAX as u64 + 1, u64::MAX] {
        let mut reference_property = BytesMut::new();
        primitives::encode_ctx_unsigned(&mut reference_property, 0, overflow);
        assert!(decode_property_reference(&reference_property, 0).is_err());

        let mut reference_index = BytesMut::new();
        primitives::encode_ctx_unsigned(&mut reference_index, 0, 1);
        primitives::encode_ctx_unsigned(&mut reference_index, 1, overflow);
        assert!(decode_property_reference(&reference_index, 0).is_err());

        let mut value_property = BytesMut::new();
        primitives::encode_ctx_unsigned(&mut value_property, 0, overflow);
        append_null_value(&mut value_property);
        assert!(decode_bacnet_property_value(&value_property, 0).is_err());

        let mut value_index = BytesMut::new();
        primitives::encode_ctx_unsigned(&mut value_index, 0, 1);
        primitives::encode_ctx_unsigned(&mut value_index, 1, overflow);
        append_null_value(&mut value_index);
        assert!(decode_bacnet_property_value(&value_index, 0).is_err());
    }
}

#[test]
fn shared_property_values_require_property_context_tag_zero() {
    let mut wrong_tag = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut wrong_tag, 1, 85);
    append_null_value(&mut wrong_tag);

    assert!(decode_property_reference(&wrong_tag, 0).is_err());
    assert!(decode_bacnet_property_value(&wrong_tag, 0).is_err());
}

// -----------------------------------------------------------------------
// Malformed-input decode error tests
// -----------------------------------------------------------------------

#[test]
fn test_decode_bacnet_property_value_empty_input() {
    assert!(decode_bacnet_property_value(&[], 0).is_err());
}

#[test]
fn test_decode_bacnet_property_value_truncated_1_byte() {
    let pv = BACnetPropertyValue {
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
        priority: None,
    };
    let mut buf = BytesMut::new();
    encode_bacnet_property_value(&pv, &mut buf).unwrap();
    assert!(decode_bacnet_property_value(&buf[..1], 0).is_err());
}

#[test]
fn test_decode_bacnet_property_value_truncated_2_bytes() {
    let pv = BACnetPropertyValue {
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
        priority: None,
    };
    let mut buf = BytesMut::new();
    encode_bacnet_property_value(&pv, &mut buf).unwrap();
    assert!(decode_bacnet_property_value(&buf[..2], 0).is_err());
}

#[test]
fn test_decode_bacnet_property_value_truncated_3_bytes() {
    let pv = BACnetPropertyValue {
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        value: vec![0x44, 0x42, 0x90, 0x00, 0x00],
        priority: None,
    };
    let mut buf = BytesMut::new();
    encode_bacnet_property_value(&pv, &mut buf).unwrap();
    assert!(decode_bacnet_property_value(&buf[..3], 0).is_err());
}

#[test]
fn test_decode_bacnet_property_value_invalid_tag() {
    assert!(decode_bacnet_property_value(&[0xFF, 0xFF, 0xFF], 0).is_err());
}

#[test]
fn test_decode_bacnet_property_value_oversized_length() {
    // Tag byte with extended length that exceeds data
    assert!(decode_bacnet_property_value(&[0x05, 0xFF], 0).is_err());
}
