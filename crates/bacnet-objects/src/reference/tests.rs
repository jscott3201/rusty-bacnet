//! The read values and the write decode of the reference properties: the
//! served bytes, the values that clear, and the error each refused value
//! carries (#182, #1312, #1395).

use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};
use bacnet_types::primitives::ObjectIdentifier;

const FRAMES: [ReferenceFrame; 2] = [ReferenceFrame::Bare, ReferenceFrame::Setpoint];

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

/// Split framed members at their tag boundaries the way the server's generic
/// value decode does (one `ApplicationData` per context tag).
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
            assert_eq!(class, ErrorClass::PROPERTY, "{context}: wrong error class");
            assert_eq!(code, expected_code, "{context}: wrong code");
        }
        other => panic!("{context}: expected PROPERTY/{expected_code:?}, got {other:?}"),
    }
}

#[test]
fn object_property_reference_reads_as_its_members_or_the_unset_form() {
    // [0] analog-input 5, [1] present-value (85).
    assert_eq!(
        object_property_reference_value(Some(&ai_ref(5, 85)), ObjectType::ANALOG_INPUT),
        PropertyValue::ApplicationData(vec![0x0C, 0x00, 0x00, 0x00, 0x05, 0x19, 0x55])
    );
    // [0] analog-output 3, [1] relinquish-default (104), [2] index 2.
    let indexed = BACnetObjectPropertyReference::new_indexed(
        ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 3).unwrap(),
        104,
        2,
    );
    assert_eq!(
        object_property_reference_value(Some(&indexed), ObjectType::ANALOG_INPUT),
        PropertyValue::ApplicationData(vec![0x0C, 0x00, 0x40, 0x00, 0x03, 0x19, 0x68, 0x29, 0x02])
    );
    // Unset (#1417): [0] the named type at instance 4194303, [1]
    // present-value (85).
    assert_eq!(
        object_property_reference_value(None, ObjectType::ANALOG_INPUT),
        PropertyValue::ApplicationData(vec![0x0C, 0x00, 0x3F, 0xFF, 0xFF, 0x19, 0x55])
    );
    assert_eq!(
        object_property_reference_value(None, ObjectType::ACCUMULATOR),
        PropertyValue::ApplicationData(vec![0x0C, 0x05, 0xFF, 0xFF, 0xFF, 0x19, 0x55])
    );
}

#[test]
fn setpoint_reference_reads_framed_or_empty() {
    // Opening tag 0, the members of analog-value 10 present-value, closing
    // tag 0.
    let reference = BACnetObjectPropertyReference::new(
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 10).unwrap(),
        85,
    );
    assert_eq!(
        setpoint_reference_value(Some(&reference)),
        PropertyValue::ApplicationData(vec![0x0E, 0x0C, 0x00, 0x80, 0x00, 0x0A, 0x19, 0x55, 0x0F])
    );
    // No reference: the optional member is left out, so no octets at all.
    assert_eq!(
        setpoint_reference_value(None),
        PropertyValue::ApplicationData(Vec::new())
    );
}

#[test]
fn read_values_write_back_unchanged() {
    let indexed =
        BACnetObjectPropertyReference::new_indexed(ai_ref(7, 88).object_identifier, 88, 12);
    for reference in [None, Some(ai_ref(5, 85)), Some(indexed)] {
        let bare = object_property_reference_value(reference.as_ref(), ObjectType::ANALOG_OUTPUT);
        assert_eq!(
            decode_reference_write(&bare, ReferenceFrame::Bare).unwrap(),
            reference
        );
        let setpoint = setpoint_reference_value(reference.as_ref());
        assert_eq!(
            decode_reference_write(&setpoint, ReferenceFrame::Setpoint).unwrap(),
            reference
        );
    }
}

#[test]
fn null_is_another_datatype_on_every_reference() {
    // Neither production has an application NULL member (#1417), so the
    // server turns this refusal into the Clause 15.9.2 no-op.
    for frame in [ReferenceFrame::Bare, ReferenceFrame::Setpoint] {
        expect_protocol(
            decode_reference_write(&PropertyValue::Null, frame),
            ErrorCode::INVALID_DATA_TYPE,
            &format!("Null on {frame:?}"),
        );
    }
}

#[test]
fn a_bare_reference_to_the_reserved_instance_clears() {
    // [0] analog-input 4194303, [1] present-value, and the same object with
    // another property and an index: both unset (#1417).
    for octets in [
        vec![0x0C, 0x00, 0x3F, 0xFF, 0xFF, 0x19, 0x55],
        vec![0x0C, 0x05, 0xFF, 0xFF, 0xFF, 0x19, 0x57, 0x29, 0x02],
    ] {
        let value = PropertyValue::ApplicationData(octets);
        assert_eq!(
            decode_reference_write(&value, ReferenceFrame::Bare).unwrap(),
            None,
            "{value:?}"
        );
    }
    // One instance short of it is an ordinary reference.
    let value = PropertyValue::ApplicationData(vec![0x0C, 0x00, 0x3F, 0xFF, 0xFE, 0x19, 0x55]);
    assert!(decode_reference_write(&value, ReferenceFrame::Bare)
        .unwrap()
        .is_some());
}

#[test]
fn empty_value_clears_only_setpoint_reference() {
    // No octets, as raw octets or as a list of chunks that join into none
    // (#1395): the setpoint frame's empty value, and no reference at all for
    // the bare members.
    for empty in [
        PropertyValue::ApplicationData(Vec::new()),
        PropertyValue::List(Vec::new()),
        PropertyValue::List(vec![PropertyValue::ApplicationData(Vec::new())]),
    ] {
        assert_eq!(
            decode_reference_write(&empty, ReferenceFrame::Setpoint).unwrap(),
            None,
            "{empty:?}"
        );
        expect_protocol(
            decode_reference_write(&empty, ReferenceFrame::Bare),
            ErrorCode::INVALID_DATA_ENCODING,
            &format!("{empty:?} on a bare reference"),
        );
    }
}

#[test]
fn framed_form_decodes_from_one_or_split_application_data() {
    let reference =
        BACnetObjectPropertyReference::new_indexed(ai_ref(7, 88).object_identifier, 88, 12);
    let frame = ReferenceFrame::Bare;
    // Whole members in one element (a WriteProperty's octets).
    assert_eq!(
        decode_reference_write(&PropertyValue::ApplicationData(framed(&reference)), frame).unwrap(),
        Some(reference.clone())
    );
    // Split at tag boundaries, as the generic value decode hands over.
    assert_eq!(
        decode_reference_write(&framed_split(&reference), frame).unwrap(),
        Some(reference.clone())
    );
    assert_eq!(
        decode_reference_write(
            &PropertyValue::ApplicationData(framed_wrapped(&reference)),
            ReferenceFrame::Setpoint
        )
        .unwrap(),
        Some(reference)
    );
}

#[test]
fn flat_list_is_another_datatype_in_every_frame() {
    let oid = ai_ref(5, 85).object_identifier;
    let flat_forms = [
        vec![
            PropertyValue::ObjectIdentifier(oid),
            PropertyValue::Enumerated(85),
        ],
        vec![
            PropertyValue::ObjectIdentifier(oid),
            PropertyValue::Unsigned(85),
            PropertyValue::Unsigned(3),
        ],
    ];
    for frame in FRAMES {
        for items in &flat_forms {
            expect_protocol(
                decode_reference_write(&PropertyValue::List(items.clone()), frame),
                ErrorCode::INVALID_DATA_TYPE,
                &format!("flat list {items:?} under {frame:?}"),
            );
        }
        // The same list as the application-tagged octets a client sends:
        // object identifier (0xC4) then Enumerated (0x91).
        expect_protocol(
            decode_reference_write(
                &PropertyValue::ApplicationData(vec![0xC4, 0x00, 0x00, 0x00, 0x05, 0x91, 0x55]),
                frame,
            ),
            ErrorCode::INVALID_DATA_TYPE,
            &format!("flat octets under {frame:?}"),
        );
    }
}

#[test]
fn opening_of_another_datatype_is_invalid_data_type() {
    let reference = ai_ref(10, 85);
    // The setpoint frame on a bare reference, and the bare members on
    // Setpoint_Reference, each open as the other datatype does.
    expect_protocol(
        decode_reference_write(
            &PropertyValue::ApplicationData(framed_wrapped(&reference)),
            ReferenceFrame::Bare,
        ),
        ErrorCode::INVALID_DATA_TYPE,
        "[0]-framed reference on a bare-reference property",
    );
    expect_protocol(
        decode_reference_write(
            &PropertyValue::ApplicationData(framed(&reference)),
            ReferenceFrame::Setpoint,
        ),
        ErrorCode::INVALID_DATA_TYPE,
        "bare members on Setpoint_Reference",
    );
    // An application Null in octets, and a context tag other than 0.
    for frame in FRAMES {
        for bytes in [vec![0x00], vec![0x19, 0x55]] {
            expect_protocol(
                decode_reference_write(&PropertyValue::ApplicationData(bytes.clone()), frame),
                ErrorCode::INVALID_DATA_TYPE,
                &format!("{bytes:02X?} under {frame:?}"),
            );
        }
    }
}

#[test]
fn framed_malformed_is_invalid_data_encoding() {
    let good = framed(&ai_ref(5, 85));
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (good[..5].to_vec(), "object id only (partial members)"),
        (
            [good.clone(), vec![0x29, 0x02, 0x3C, 0x00, 0x00, 0x00, 0x4D]].concat(),
            "device-qualified member [3]",
        ),
        (
            [good.clone(), vec![0x49, 0x01]].concat(),
            "unknown trailing context tag [4]",
        ),
        (
            [good.clone(), vec![0x21, 0x00]].concat(),
            "application tag trailing the members",
        ),
        ([good.clone(), good.clone()].concat(), "two references"),
        (vec![0x0C, 0x00], "truncated object identifier"),
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
        // The same members inside the setpoint frame fail the same way.
        let mut wrapped = vec![0x0E];
        wrapped.extend_from_slice(&bytes);
        wrapped.push(0x0F);
        expect_protocol(
            decode_reference_write(
                &PropertyValue::ApplicationData(wrapped),
                ReferenceFrame::Setpoint,
            ),
            ErrorCode::INVALID_DATA_ENCODING,
            &format!("{context}, framed"),
        );
    }
}

#[test]
fn setpoint_frame_must_hold_one_whole_reference() {
    let wrapped = framed_wrapped(&ai_ref(10, 85));
    for (bytes, context) in [
        (vec![0x0E, 0x0F], "empty frame"),
        (wrapped[..wrapped.len() - 1].to_vec(), "unbalanced frame"),
        (
            [wrapped.clone(), vec![0x21, 0x01]].concat(),
            "trailing octets",
        ),
        ([wrapped.clone(), wrapped.clone()].concat(), "two frames"),
    ] {
        expect_protocol(
            decode_reference_write(
                &PropertyValue::ApplicationData(bytes),
                ReferenceFrame::Setpoint,
            ),
            ErrorCode::INVALID_DATA_ENCODING,
            context,
        );
    }
}

#[test]
fn list_mixing_chunks_and_decoded_values_is_invalid_data_type() {
    // A chunk then a decoded member: the list isn't octets to join, so the
    // value is another datatype, as for the device references (#1395).
    let value = PropertyValue::List(vec![
        PropertyValue::ApplicationData(framed(&ai_ref(5, 85))[..5].to_vec()),
        PropertyValue::Enumerated(85),
    ]);
    for frame in FRAMES {
        expect_protocol(
            decode_reference_write(&value, frame),
            ErrorCode::INVALID_DATA_TYPE,
            &format!("mixed framed + flat members under {frame:?}"),
        );
    }
}

#[test]
fn wrong_value_datatypes_are_invalid_data_type() {
    for frame in FRAMES {
        for value in [
            PropertyValue::Unsigned(42),
            PropertyValue::Real(1.0),
            PropertyValue::ObjectIdentifier(ai_ref(5, 85).object_identifier),
            PropertyValue::List(vec![PropertyValue::Unsigned(1), PropertyValue::Unsigned(2)]),
        ] {
            expect_protocol(
                decode_reference_write(&value, frame),
                ErrorCode::INVALID_DATA_TYPE,
                &format!("{value:?} under {frame:?}"),
            );
        }
    }
}
