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
fn setpoint_reference_empty_value_is_the_absent_alternative() {
    // Without its optional member the sequence encodes as nothing at all
    // (Clause 20.2.16): the value that holds no reference.
    assert_eq!(decode_setpoint_reference(&[]).unwrap(), None);
}

#[test]
fn setpoint_reference_empty_frame_is_refused() {
    // 0x0E 0x0F: the [0] member is present but names no object or property,
    // so it is an incomplete reference, not the absent member (#1312).
    assert!(decode_setpoint_reference(&[0x0E, 0x0F]).is_err());
    // A closing tag alone, or an opening tag with no close, frames nothing.
    assert!(decode_setpoint_reference(&[0x0F]).is_err());
    assert!(decode_setpoint_reference(&[0x0E]).is_err());
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

#[test]
fn offset_codecs_stop_after_the_reference() {
    // Two references back to back, then a stray Unsigned: each decode
    // starts where the last stopped and leaves what follows alone (#1414).
    let indexed = BACnetObjectPropertyReference::new_indexed(
        ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 3).unwrap(),
        87,
        2,
    );
    let mut bare = BytesMut::new();
    encode_object_property_reference(&mut bare, &ai_ref(5, 85));
    let first = bare.len();
    encode_object_property_reference(&mut bare, &indexed);
    let second = bare.len();
    bare.extend_from_slice(&[0x21, 0x01]);
    assert_eq!(
        decode_object_property_reference_at(&bare, 0).unwrap(),
        (ai_ref(5, 85), first)
    );
    assert_eq!(
        decode_object_property_reference_at(&bare, first).unwrap(),
        (indexed.clone(), second)
    );
    let mut framed = BytesMut::new();
    encode_setpoint_reference(&mut framed, &ai_ref(10, 85));
    let first = framed.len();
    encode_setpoint_reference(&mut framed, &indexed);
    framed.extend_from_slice(&[0x21, 0x01]);
    assert_eq!(
        decode_setpoint_reference_at(&framed, 0).unwrap(),
        (ai_ref(10, 85), first)
    );
    assert_eq!(
        decode_setpoint_reference_at(&framed, first).unwrap(),
        (indexed, framed.len() - 2)
    );
}

#[test]
fn offset_codecs_refuse_a_device_member_and_a_crowded_frame() {
    // A [3] device right after the members is not part of the production.
    let mut bare = BytesMut::new();
    encode_object_property_reference(&mut bare, &ai_ref(5, 85));
    let device_at = bare.len();
    crate::primitives::encode_ctx_object_id(
        &mut bare,
        3,
        &ObjectIdentifier::new(ObjectType::DEVICE, 77).unwrap(),
    );
    match decode_object_property_reference_at(&bare, 0) {
        Err(error) if error.decode_fault().is_some() => {
            assert_eq!(error.decode_fault().map(|(offset, _)| offset), Some(device_at))
        }
        other => panic!("expected a decoding error, got {other:?}"),
    }
    // Inside the setpoint frame the members must fill it.
    let crowded = [
        0x0E, 0x0C, 0x00, 0x00, 0x00, 0x0A, 0x19, 0x55, 0x49, 0x01, 0x0F,
    ];
    assert!(decode_setpoint_reference_at(&crowded, 0).is_err());
    // The frame must be there; the value without one is a whole-payload
    // matter.
    assert!(decode_setpoint_reference_at(&[], 0).is_err());
    assert!(decode_setpoint_reference_at(&bare[..device_at], 0).is_err());
}

#[test]
fn members_cut_short_are_a_short_buffer() {
    let mut bare = BytesMut::new();
    encode_object_property_reference(&mut bare, &ai_ref(5, 85));
    crate::constructed::tests::assert_members_cut_short("bare", &bare, |data| {
        decode_object_property_reference_at(data, 0)
    });
    let mut framed = BytesMut::new();
    encode_setpoint_reference(&mut framed, &ai_ref(5, 85));
    let inside = crate::constructed::tests::assert_members_cut_short("setpoint", &framed, |data| {
        decode_setpoint_reference_at(data, 0)
    });
    assert!(inside > 0);
}
