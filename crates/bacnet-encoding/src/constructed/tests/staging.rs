use super::*;
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};

#[test]
fn stage_limit_value_uses_three_application_fields_and_round_trips() {
    let value = BACnetStageLimitValue {
        limit: 12.5,
        values: vec![true, false, true, true, false, false, true, false, true],
        deadband: 0.75,
    };
    let mut encoded = BytesMut::new();
    encode_stage_limit_value(&mut encoded, &value);

    assert_eq!(encoded[0] >> 4, tags::app_tag::REAL);
    let (_, second) = primitives::decode_application_value(&encoded, 0).unwrap();
    assert_eq!(encoded[second] >> 4, tags::app_tag::BIT_STRING);
    let (_, third) = primitives::decode_application_value(&encoded, second).unwrap();
    assert_eq!(encoded[third] >> 4, tags::app_tag::REAL);

    let (decoded, end) = decode_stage_limit_value(&encoded, 0).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(end, encoded.len());
}

#[test]
fn stage_limit_value_rejects_wrong_order_truncation_and_nonzero_padding() {
    let mut wrong_order = BytesMut::new();
    primitives::encode_app_bit_string(&mut wrong_order, 7, &[0x80]);
    primitives::encode_app_real(&mut wrong_order, 1.0);
    primitives::encode_app_real(&mut wrong_order, 0.0);
    assert!(decode_stage_limit_value(&wrong_order, 0).is_err());

    let value = BACnetStageLimitValue {
        limit: 1.0,
        values: vec![true],
        deadband: 0.0,
    };
    let mut encoded = BytesMut::new();
    encode_stage_limit_value(&mut encoded, &value);
    assert!(decode_stage_limit_value(&encoded[..encoded.len() - 1], 0).is_err());

    let mut padding = BytesMut::new();
    primitives::encode_app_real(&mut padding, 1.0);
    primitives::encode_app_bit_string(&mut padding, 7, &[0x81]);
    primitives::encode_app_real(&mut padding, 0.0);
    assert!(decode_stage_limit_value(&padding, 0).is_err());
}

#[test]
fn device_object_reference_exact_local_and_remote_forms_round_trip() {
    let local = BACnetDeviceObjectReference {
        device_identifier: None,
        object_identifier: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 7).unwrap(),
    };
    let remote = BACnetDeviceObjectReference {
        device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 99).unwrap()),
        object_identifier: ObjectIdentifier::new(ObjectType::BINARY_VALUE, 8).unwrap(),
    };

    for (reference, first_tag) in [(local, 1_u8), (remote, 0_u8)] {
        let mut encoded = BytesMut::new();
        encode_device_object_reference(&mut encoded, &reference);
        let (tag, _) = tags::decode_tag(&encoded, 0).unwrap();
        assert!(tag.is_context(first_tag));
        let (decoded, end) = decode_device_object_reference(&encoded, 0).unwrap();
        assert_eq!(decoded, reference);
        assert_eq!(end, encoded.len());
    }
}

#[test]
fn device_object_reference_rejects_missing_object_and_trailing_malformed_sequence() {
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 99).unwrap();
    let mut missing_object = BytesMut::new();
    primitives::encode_ctx_object_id(&mut missing_object, 0, &device);
    assert!(decode_device_object_reference(&missing_object, 0).is_err());

    let local = BACnetDeviceObjectReference {
        device_identifier: None,
        object_identifier: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 7).unwrap(),
    };
    let mut encoded = BytesMut::new();
    encode_device_object_reference(&mut encoded, &local);
    encoded.extend_from_slice(&[0x19]);
    let (_, end) = decode_device_object_reference(&encoded, 0).unwrap();
    assert_ne!(end, encoded.len(), "callers must reject trailing bytes");
    assert!(decode_device_object_reference(&encoded, end).is_err());
}
