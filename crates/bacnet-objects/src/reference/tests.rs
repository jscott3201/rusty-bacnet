//! Shape matrix for `decode_reference_write`: the accepted local/framed
//! forms and the exact Clause 15.9.1.3 error pairing of everything else.

use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};
use bacnet_types::primitives::ObjectIdentifier;

fn ai_ref(instance: u32, property: u32) -> BACnetObjectPropertyReference {
    BACnetObjectPropertyReference::new(
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
        property,
    )
}

fn framed(r: &BACnetObjectPropertyReference) -> Vec<u8> {
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_object_property_reference(&mut buf, r);
    buf.to_vec()
}

fn framed_wrapped(r: &BACnetObjectPropertyReference) -> Vec<u8> {
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_setpoint_reference(&mut buf, r);
    buf.to_vec()
}

/// Split framed members at their tag boundaries the way the service decode
/// loop does (one `ApplicationData` per context tag).
fn framed_split(r: &BACnetObjectPropertyReference) -> PropertyValue {
    let bytes = framed(r);
    let mut values = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let (value, new_offset) =
            bacnet_encoding::primitives::decode_application_value(&bytes, offset).unwrap();
        values.push(value);
        offset = new_offset;
    }
    PropertyValue::List(values)
}

fn expect_protocol(
    result: Result<Option<BACnetObjectPropertyReference>, Error>,
    expected_code: ErrorCode,
    context: &str,
) {
    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(
                class,
                ErrorClass::PROPERTY.to_raw() as u32,
                "{context}: wrong error class"
            );
            assert_eq!(code, expected_code.to_raw() as u32, "{context}: wrong code");
        }
        other => panic!("{context}: expected PROPERTY/{expected_code:?}, got {other:?}"),
    }
}

#[test]
fn null_clears_in_both_frames() {
    assert_eq!(
        decode_reference_write(&PropertyValue::Null, ReferenceFrame::Bare).unwrap(),
        None
    );
    assert_eq!(
        decode_reference_write(&PropertyValue::Null, ReferenceFrame::Setpoint).unwrap(),
        None
    );
}

#[test]
fn legacy_local_list_form_round_trips_with_and_without_index() {
    let value = PropertyValue::List(vec![
        PropertyValue::ObjectIdentifier(ai_ref(5, 85).object_identifier),
        PropertyValue::Enumerated(85),
    ]);
    assert_eq!(
        decode_reference_write(&value, ReferenceFrame::Bare).unwrap(),
        Some(ai_ref(5, 85))
    );

    let indexed = PropertyValue::List(vec![
        PropertyValue::ObjectIdentifier(ai_ref(5, 85).object_identifier),
        PropertyValue::Enumerated(87),
        PropertyValue::Unsigned(3),
    ]);
    let decoded = decode_reference_write(&indexed, ReferenceFrame::Setpoint)
        .unwrap()
        .unwrap();
    assert_eq!(decoded.property_array_index, Some(3));
}

#[test]
fn legacy_list_shape_violations_are_invalid_data_type() {
    let oid = PropertyValue::ObjectIdentifier(ai_ref(5, 85).object_identifier);
    for (items, context) in [
        (vec![oid.clone()], "object id alone"),
        (
            vec![oid.clone(), PropertyValue::Real(85.0)],
            "property id neither Unsigned nor Enumerated",
        ),
        (
            vec![
                oid.clone(),
                PropertyValue::Unsigned(u64::from(u32::MAX) + 1),
            ],
            "property id Unsigned beyond u32 (>4-octet member)",
        ),
        (
            vec![
                oid.clone(),
                PropertyValue::Enumerated(85),
                PropertyValue::Real(1.0),
            ],
            "index not Unsigned",
        ),
        (
            vec![
                oid.clone(),
                PropertyValue::Enumerated(85),
                PropertyValue::Unsigned(u64::from(u32::MAX) + 1),
            ],
            "index beyond u32",
        ),
        (
            vec![
                oid.clone(),
                PropertyValue::Enumerated(85),
                PropertyValue::Unsigned(3),
                PropertyValue::Unsigned(4),
            ],
            "four members",
        ),
        (
            vec![PropertyValue::ObjectIdentifier(
                ai_ref(5, 85).object_identifier,
            )],
            "repeat guard",
        ),
    ] {
        expect_protocol(
            decode_reference_write(&PropertyValue::List(items), ReferenceFrame::Bare),
            ErrorCode::INVALID_DATA_TYPE,
            context,
        );
    }
}

#[test]
fn legacy_list_accepts_unsigned_or_enumerated_property_member() {
    // The Averaging flat form carries Unsigned, the Loop/Pulse flat form
    // Enumerated; both decode to the same reference.
    let oid = ai_ref(5, 85).object_identifier;
    for member in [PropertyValue::Unsigned(85), PropertyValue::Enumerated(85)] {
        let value = PropertyValue::List(vec![PropertyValue::ObjectIdentifier(oid), member]);
        assert_eq!(
            decode_reference_write(&value, ReferenceFrame::Bare).unwrap(),
            Some(ai_ref(5, 85))
        );
    }
}

#[test]
fn framed_form_decodes_from_one_or_split_application_data() {
    let reference =
        BACnetObjectPropertyReference::new_indexed(ai_ref(7, 88).object_identifier, 88, 12);
    // Whole frame in one element (an in-process framed write).
    assert_eq!(
        decode_reference_write(
            &PropertyValue::ApplicationData(framed(&reference)),
            ReferenceFrame::Bare
        )
        .unwrap(),
        Some(reference.clone())
    );
    // Split at tag boundaries, exactly as the service decode loop hands over.
    assert_eq!(
        decode_reference_write(&framed_split(&reference), ReferenceFrame::Bare).unwrap(),
        Some(reference)
    );
}

#[test]
fn framed_malformed_is_invalid_data_encoding() {
    let good = framed(&ai_ref(5, 85));
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (Vec::new(), "empty frame"),
        (good[..5].to_vec(), "object id only (partial members)"),
        (good[5..].to_vec(), "property id only (object id missing)"),
        (
            {
                let mut b = good.clone();
                b.extend_from_slice(&[0x29, 0x02]); // indexed → fine; then:
                b.extend_from_slice(&[0x3C, 0x00, 0x00, 0x00, 0x4D]); // + [3] device 77
                b
            },
            "device-qualified member [3]",
        ),
        (
            {
                let mut b = good.clone();
                b.extend_from_slice(&[0x49, 0x01]); // unknown context tag [4]
                b
            },
            "unknown trailing context tag [4]",
        ),
        (
            {
                let mut b = good.clone();
                b.extend_from_slice(&[0x21, 0x00]); // application tag after members
                b
            },
            "application tag trailing the members",
        ),
    ];
    for (bytes, context) in cases {
        expect_protocol(
            decode_reference_write(
                &PropertyValue::ApplicationData(bytes.clone()),
                ReferenceFrame::Bare,
            ),
            ErrorCode::INVALID_DATA_ENCODING,
            context,
        );
        // Same bytes split into per-element ApplicationData fail identically.
        let mut values = Vec::new();
        let mut offset = 0;
        while offset < bytes.len() {
            match bacnet_encoding::primitives::decode_application_value(&bytes, offset) {
                Ok((value, new_offset)) => {
                    values.push(value);
                    offset = new_offset;
                }
                Err(_) => break,
            }
        }
        if values.len() > 1
            && values
                .iter()
                .all(|v| matches!(v, PropertyValue::ApplicationData(_)))
        {
            expect_protocol(
                decode_reference_write(&PropertyValue::List(values), ReferenceFrame::Bare),
                ErrorCode::INVALID_DATA_ENCODING,
                context,
            );
        }
    }
}

#[test]
fn mixed_flat_and_framed_list_is_invalid_data_encoding() {
    let value = PropertyValue::List(vec![
        PropertyValue::ApplicationData(framed(&ai_ref(5, 85))[..5].to_vec()),
        PropertyValue::Enumerated(85),
    ]);
    expect_protocol(
        decode_reference_write(&value, ReferenceFrame::Bare),
        ErrorCode::INVALID_DATA_ENCODING,
        "mixed framed + flat members",
    );
}

#[test]
fn empty_setpoint_frame_clears_only_on_the_setpoint_arm() {
    // 0x0E 0x0F: the BACnetSetpointReference frame with its OPTIONAL member
    // absent — a syntactically valid encoding Clause 12.17 defines as "no
    // reference" (fixed setpoint). On the Setpoint arm it clears (None);
    // on the bare reference properties it is not a valid value.
    let empty_frame = PropertyValue::ApplicationData(vec![0x0E, 0x0F]);
    assert_eq!(
        decode_reference_write(&empty_frame, ReferenceFrame::Setpoint).unwrap(),
        None
    );
    expect_protocol(
        decode_reference_write(&empty_frame, ReferenceFrame::Bare),
        ErrorCode::INVALID_DATA_ENCODING,
        "empty setpoint frame on a bare-reference property",
    );
}

#[test]
fn wrong_value_datatypes_are_invalid_data_type() {
    for value in [
        PropertyValue::Unsigned(42),
        PropertyValue::Real(1.0),
        PropertyValue::List(vec![PropertyValue::Unsigned(1), PropertyValue::Unsigned(2)]),
    ] {
        expect_protocol(
            decode_reference_write(&value, ReferenceFrame::Bare),
            ErrorCode::INVALID_DATA_TYPE,
            "non-reference datatype",
        );
    }
}

#[test]
fn setpoint_frame_acceptance_is_scoped_to_the_setpoint_property() {
    let reference = ai_ref(10, 85);
    let wrapped = PropertyValue::ApplicationData(framed_wrapped(&reference));
    // The BACnetSetpointReference [0] frame decodes on Setpoint arm...
    assert_eq!(
        decode_reference_write(&wrapped, ReferenceFrame::Setpoint).unwrap(),
        Some(reference.clone())
    );
    // ... and is refused on the bare reference properties.
    expect_protocol(
        decode_reference_write(&wrapped, ReferenceFrame::Bare),
        ErrorCode::INVALID_DATA_ENCODING,
        "[0]-framed reference on a bare-reference property",
    );
    // The bare member sequence stays accepted on the Setpoint arm as well
    // (peers handling the reference generically), and an unbalanced frame is
    // refused either way.
    assert_eq!(
        decode_reference_write(
            &PropertyValue::ApplicationData(framed(&reference)),
            ReferenceFrame::Setpoint
        )
        .unwrap(),
        Some(reference.clone())
    );
    let unbalanced = framed_wrapped(&reference)[..framed_wrapped(&reference).len() - 1].to_vec();
    expect_protocol(
        decode_reference_write(
            &PropertyValue::ApplicationData(unbalanced),
            ReferenceFrame::Setpoint,
        ),
        ErrorCode::INVALID_DATA_ENCODING,
        "unbalanced setpoint frame",
    );
}
