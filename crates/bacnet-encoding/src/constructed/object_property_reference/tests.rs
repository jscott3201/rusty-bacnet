//! Golden + negative vectors for the reference codecs (Clause 21).

use super::*;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

fn ai_ref(instance: u32, property: u32) -> BACnetObjectPropertyReference {
    BACnetObjectPropertyReference::new(
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
        property,
    )
}

#[test]
fn golden_vector_unindexed() {
    // ANALOG_INPUT:5 → (0 << 22) | 5 = 0x00000005 under primitive context
    // tag [0]; present-value (85) as one-octet unsigned under [1].
    let mut buf = BytesMut::new();
    encode_object_property_reference(&mut buf, &ai_ref(5, 85));
    assert_eq!(buf.as_ref(), &[0x0C, 0x00, 0x00, 0x00, 0x05, 0x19, 0x55]);
    let decoded = decode_object_property_reference(&buf).unwrap();
    assert_eq!(decoded, ai_ref(5, 85));
}

#[test]
fn golden_vector_indexed() {
    let mut buf = BytesMut::new();
    encode_object_property_reference(
        &mut buf,
        &BACnetObjectPropertyReference::new_indexed(
            ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 3).unwrap(),
            87,
            2,
        ),
    );
    // ANALOG_OUTPUT:3 → (1 << 22) | 3 = 0x00400003; [2] index 2
    assert_eq!(
        buf.as_ref(),
        &[0x0C, 0x00, 0x40, 0x00, 0x03, 0x19, 0x57, 0x29, 0x02]
    );
    let decoded = decode_object_property_reference(&buf).unwrap();
    assert_eq!(decoded.property_array_index, Some(2));
}

#[test]
fn setpoint_reference_golden_vector() {
    // The BACnetSetpointReference [0] frame around the bare members.
    let mut buf = BytesMut::new();
    encode_setpoint_reference(&mut buf, &ai_ref(10, 85));
    assert_eq!(
        buf.as_ref(),
        &[0x0E, 0x0C, 0x00, 0x00, 0x00, 0x0A, 0x19, 0x55, 0x0F]
    );
    assert_eq!(
        decode_setpoint_reference(&buf).unwrap(),
        Some(ai_ref(10, 85))
    );
}

#[test]
fn setpoint_reference_golden_vector_indexed() {
    // The optional property-array-index rides inside the frame as member [2].
    let indexed = BACnetObjectPropertyReference::new_indexed(
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 10).unwrap(),
        85,
        7,
    );
    let mut buf = BytesMut::new();
    encode_setpoint_reference(&mut buf, &indexed);
    assert_eq!(
        buf.as_ref(),
        &[0x0E, 0x0C, 0x00, 0x80, 0x00, 0x0A, 0x19, 0x55, 0x29, 0x07, 0x0F]
    );
    assert_eq!(decode_setpoint_reference(&buf).unwrap(), Some(indexed));
}

#[test]
fn setpoint_reference_empty_frame_is_the_absent_alternative() {
    // 0x0E 0x0F: opening/closing tag 0 with no members — the production's
    // OPTIONAL member is absent, which Clause 12.17 defines as "no
    // reference" (fixed setpoint), NOT an encoding error.
    assert_eq!(decode_setpoint_reference(&[0x0E, 0x0F]).unwrap(), None);
}

#[test]
fn bare_decode_rejects_device_qualified_reference() {
    // [0] oid / [1] prop / [3] device: BACnetDeviceObjectPropertyReference
    // members are not part of this production.
    let mut buf = BytesMut::new();
    encode_object_property_reference(&mut buf, &ai_ref(5, 85));
    crate::primitives::encode_ctx_object_id(
        &mut buf,
        3,
        &ObjectIdentifier::new(ObjectType::DEVICE, 77).unwrap(),
    );
    assert!(decode_object_property_reference(&buf).is_err());
}

#[test]
fn bare_decode_rejects_unknown_trailing_context_tag() {
    // [4] is not in the production at all.
    let mut buf = BytesMut::new();
    encode_object_property_reference(&mut buf, &ai_ref(5, 85));
    crate::primitives::encode_ctx_unsigned(&mut buf, 4, 1);
    assert!(decode_object_property_reference(&buf).is_err());
}

#[test]
fn bare_decode_rejects_partial_and_empty() {
    // [0] object-identifier alone (property-identifier missing).
    let mut buf = BytesMut::new();
    crate::primitives::encode_ctx_object_id(
        &mut buf,
        0,
        &ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 5).unwrap(),
    );
    assert!(decode_object_property_reference(&buf).is_err());
    assert!(decode_object_property_reference(&[]).is_err());
}

#[test]
fn setpoint_decode_requires_the_frame_and_full_consumption() {
    // Bare members are NOT a BACnetSetpointReference.
    let mut buf = BytesMut::new();
    encode_object_property_reference(&mut buf, &ai_ref(5, 85));
    assert!(decode_setpoint_reference(&buf).is_err());
    // ... and the framed form is not a bare reference either.
    let mut wrapped = BytesMut::new();
    encode_setpoint_reference(&mut wrapped, &ai_ref(5, 85));
    assert!(decode_object_property_reference(&wrapped).is_err());
    // Unbalanced frame: opening [0] without its closing tag.
    let truncated = &wrapped[..wrapped.len() - 1];
    assert!(decode_setpoint_reference(truncated).is_err());
    // Trailing byte after the closing tag.
    let mut extra = wrapped.to_vec();
    extra.push(0x21);
    extra.push(0x01);
    assert!(decode_setpoint_reference(&extra).is_err());
}
