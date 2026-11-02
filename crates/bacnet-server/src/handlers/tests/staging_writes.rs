use super::*;
use bacnet_encoding::constructed::{encode_device_object_reference, encode_stage_limit_value};
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};

#[test]
fn staging_structured_decoder_groups_whole_and_indexed_values() {
    let stages = [
        BACnetStageLimitValue {
            limit: 10.0,
            values: vec![false],
            deadband: 1.0,
        },
        BACnetStageLimitValue {
            limit: 20.0,
            values: vec![true],
            deadband: 1.0,
        },
    ];
    let mut bytes = BytesMut::new();
    for stage in &stages {
        encode_stage_limit_value(&mut bytes, stage);
    }
    let PropertyValue::List(values) = decode(PropertyIdentifier::STAGES, None, &bytes).unwrap()
    else {
        panic!("whole Stages must decode to a list");
    };
    assert_eq!(values.len(), 2);
    assert!(matches!(values[0], PropertyValue::ApplicationData(_)));

    let mut one = BytesMut::new();
    encode_stage_limit_value(&mut one, &stages[0]);
    assert!(matches!(
        decode(PropertyIdentifier::STAGES, Some(1), &one).unwrap(),
        PropertyValue::ApplicationData(_)
    ));
    assert!(decode(PropertyIdentifier::STAGES, Some(1), &bytes).is_err());
}

#[test]
fn staging_structured_decoder_rejects_trailing_malformed_data() {
    let stage = BACnetStageLimitValue {
        limit: 10.0,
        values: vec![false],
        deadband: 1.0,
    };
    let mut bytes = BytesMut::new();
    encode_stage_limit_value(&mut bytes, &stage);
    bytes.extend_from_slice(&[0x44, 0x00]);
    assert!(decode(PropertyIdentifier::STAGES, None, &bytes).is_err());
}

#[test]
fn staging_targets_reach_the_object_as_raw_reference_bytes() {
    // The object decodes them with the shared device-reference helpers, so
    // trailing malformed data is its refusal to make, not the decoder's
    // (#1313).
    let reference = BACnetDeviceObjectReference {
        device_identifier: None,
        object_identifier: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1).unwrap(),
    };
    let mut bytes = BytesMut::new();
    encode_device_object_reference(&mut bytes, &reference);
    bytes.extend_from_slice(&[0x19]);
    for index in [None, Some(1)] {
        assert_eq!(
            decode(PropertyIdentifier::TARGET_REFERENCES, index, &bytes).unwrap(),
            PropertyValue::ApplicationData(bytes.to_vec())
        );
    }
    let mut size = BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut size, 2);
    assert_eq!(
        decode(PropertyIdentifier::TARGET_REFERENCES, Some(0), &size).unwrap(),
        PropertyValue::Unsigned(2)
    );
}

#[test]
fn staging_array_index_zero_keeps_unsigned_write_semantics() {
    let mut bytes = BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut bytes, 2);
    assert_eq!(
        decode(PropertyIdentifier::STAGES, Some(0), &bytes).unwrap(),
        PropertyValue::Unsigned(2)
    );
}

/// A Staging write's value decoded as the server decodes it: by the
/// property's standard value type on a Staging object.
fn decode(
    property: PropertyIdentifier,
    index: Option<u32>,
    bytes: &[u8],
) -> Result<PropertyValue, bacnet_types::error::Error> {
    let value_type =
        bacnet_objects::property_metadata::standard_value_type(ObjectType::STAGING, property);
    decode_write_property_value(property, value_type, index, bytes)
}
