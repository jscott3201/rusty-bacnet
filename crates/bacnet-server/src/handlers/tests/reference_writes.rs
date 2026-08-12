//! Wire-level integration for the structured reference properties (#182):
//! the Loop references (Clause 12.17) and Pulse Converter Input_Reference
//! (Clause 12.23) are `BACnetObjectPropertyReference`, the Averaging
//! `Object_Property_Reference` (Clause 12.5) is the device-qualifying
//! sibling production — each decoded STRICTLY from the context-tagged
//! members the multi-element service decode hands over, with device
//! members [3] refused (local-device-only posture for the in-tree models).

use super::*;
use bacnet_objects::accumulator::PulseConverterObject;
use bacnet_objects::averaging::AveragingObject;
use bacnet_objects::loop_obj::LoopObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::BACnetObjectPropertyReference;

fn encode_value(value: PropertyValue) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_property_value(&mut buf, &value).unwrap();
    buf.to_vec()
}

fn write_raw(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    property_value: Vec<u8>,
) -> Result<(), Error> {
    let request = WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value,
        priority: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    handle_write_property(db, &buf).map(|_| ())
}

/// Read a property over the wire and loop-decode the flattened result the
/// same way the write path decodes (single element → scalar, else `List`).
fn read_prop(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    let request = ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    let mut ack_buf = BytesMut::new();
    handle_read_property(db, &buf, &mut ack_buf).unwrap();
    let raw = ReadPropertyACK::decode(&ack_buf).unwrap().property_value;
    let mut values = Vec::new();
    let mut offset = 0;
    while offset < raw.len() {
        let (value, new_offset) =
            bacnet_encoding::primitives::decode_application_value(&raw, offset).unwrap();
        values.push(value);
        offset = new_offset;
    }
    match values.len() {
        1 => values.pop().unwrap(),
        _ => PropertyValue::List(values),
    }
}

fn framed_reference(r: &BACnetObjectPropertyReference) -> Vec<u8> {
    let mut buf = BytesMut::new();
    bacnet_encoding::constructed::encode_object_property_reference(&mut buf, r);
    buf.to_vec()
}

fn reference_target() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap()
}

fn add_loop(db: &mut ObjectDatabase, instance: u32) -> ObjectIdentifier {
    let lo = LoopObject::new(instance, format!("LOOP-{instance}"), 62).unwrap();
    let oid = lo.object_identifier();
    db.add(Box::new(lo)).unwrap();
    oid
}

/// A refused write carries exactly PROPERTY / `expected_code` and leaves
/// `property` reading back as `expected`.
fn assert_refused(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    raw_value: Vec<u8>,
    expected_code: ErrorCode,
    expected: PropertyValue,
    context: &str,
) {
    match write_raw(db, oid, property, raw_value).expect_err(context) {
        Error::Protocol { class, code } => {
            assert_eq!(
                class,
                ErrorClass::PROPERTY.to_raw() as u32,
                "{context}: wrong error class"
            );
            assert_eq!(
                code,
                expected_code.to_raw() as u32,
                "{context}: wrong error code"
            );
        }
        other => panic!("{context}: expected PROPERTY/{expected_code:?}, got {other:?}"),
    }
    assert_eq!(
        read_prop(db, oid, property),
        expected,
        "{context}: refused write must leave the property unchanged"
    );
}

#[test]
fn loop_references_framed_writes_over_the_wire() {
    let mut db = ObjectDatabase::new();
    let oid = add_loop(&mut db, 1);
    let target = reference_target();
    let present_value = PropertyIdentifier::PRESENT_VALUE.to_raw();

    // Indexed CONTROLLED_VARIABLE_REFERENCE: bare member sequence.
    write_raw(
        &mut db,
        oid,
        PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE,
        framed_reference(&BACnetObjectPropertyReference::new_indexed(
            target,
            present_value,
            3,
        )),
    )
    .unwrap();
    assert_eq!(
        read_prop(&db, oid, PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE),
        PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(target),
            PropertyValue::Enumerated(present_value),
            PropertyValue::Unsigned(3),
        ])
    );

    // MANIPULATED_VARIABLE_REFERENCE: bare members, no index → 3 context-tag
    // elements minus [2] = 2 ApplicationData elements through the decode loop.
    write_raw(
        &mut db,
        oid,
        PropertyIdentifier::MANIPULATED_VARIABLE_REFERENCE,
        framed_reference(&BACnetObjectPropertyReference::new(target, present_value)),
    )
    .unwrap();
    assert_eq!(
        read_prop(&db, oid, PropertyIdentifier::MANIPULATED_VARIABLE_REFERENCE),
        PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(target),
            PropertyValue::Enumerated(present_value),
        ])
    );

    // SETPOINT_REFERENCE: the BACnetSetpointReference [0] frame — one
    // ApplicationData element through the decode loop.
    let mut wrapped = BytesMut::new();
    bacnet_encoding::constructed::encode_setpoint_reference(
        &mut wrapped,
        &BACnetObjectPropertyReference::new(target, present_value),
    );
    write_raw(
        &mut db,
        oid,
        PropertyIdentifier::SETPOINT_REFERENCE,
        wrapped.to_vec(),
    )
    .unwrap();
    assert_eq!(
        read_prop(&db, oid, PropertyIdentifier::SETPOINT_REFERENCE),
        PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(target),
            PropertyValue::Enumerated(present_value),
        ])
    );
}

#[test]
fn reference_write_flattened_local_form_also_lands_over_the_wire() {
    let mut db = ObjectDatabase::new();
    let oid = add_loop(&mut db, 1);
    let target = reference_target();
    let present_value = PropertyIdentifier::PRESENT_VALUE.to_raw();

    // A peer (or this stack's own read-build-write-back tooling) writing the
    // flattened application-tagged form: the decode loop builds the legacy
    // local List the arm already understood.
    let bytes = encode_value(PropertyValue::List(vec![
        PropertyValue::ObjectIdentifier(target),
        PropertyValue::Enumerated(present_value),
    ]));
    write_raw(
        &mut db,
        oid,
        PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE,
        bytes,
    )
    .unwrap();
    assert_eq!(
        read_prop(&db, oid, PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE),
        PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(target),
            PropertyValue::Enumerated(present_value),
        ])
    );

    // Null clears.
    write_raw(
        &mut db,
        oid,
        PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE,
        encode_value(PropertyValue::Null),
    )
    .unwrap();
    assert_eq!(
        read_prop(&db, oid, PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE),
        PropertyValue::Null
    );
}

#[test]
fn reference_write_rejections_over_the_wire_preserve_state() {
    let mut db = ObjectDatabase::new();
    let oid = add_loop(&mut db, 1);
    let target = reference_target();
    let present_value = PropertyIdentifier::PRESENT_VALUE.to_raw();
    let r = BACnetObjectPropertyReference::new(target, present_value);
    let baseline = PropertyValue::List(vec![
        PropertyValue::ObjectIdentifier(target),
        PropertyValue::Enumerated(present_value),
    ]);

    write_raw(
        &mut db,
        oid,
        PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE,
        framed_reference(&r),
    )
    .unwrap();

    // Device-qualified ([3]): not part of the BACnetObjectPropertyReference
    // production — INVALID_DATA_ENCODING.
    let mut framed = BytesMut::new();
    framed.extend_from_slice(&framed_reference(&r));
    bacnet_encoding::primitives::encode_ctx_object_id(
        &mut framed,
        3,
        &ObjectIdentifier::new(ObjectType::DEVICE, 77).unwrap(),
    );
    assert_refused(
        &mut db,
        oid,
        PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE,
        framed.to_vec(),
        ErrorCode::INVALID_DATA_ENCODING,
        baseline.clone(),
        "device-qualified reference",
    );

    // Unknown trailing context tag [4]: rejected, baseline preserved.
    let mut bytes = framed_reference(&r);
    bytes.extend_from_slice(&[0x49, 0x01]);
    assert_refused(
        &mut db,
        oid,
        PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE,
        bytes,
        ErrorCode::INVALID_DATA_ENCODING,
        baseline.clone(),
        "unknown trailing context tag",
    );

    // Wrong datatype for the property: a two-element Unsigned list is not a
    // reference — INVALID_DATA_TYPE.
    let bytes = encode_value(PropertyValue::List(vec![
        PropertyValue::Unsigned(1),
        PropertyValue::Unsigned(2),
    ]));
    assert_refused(
        &mut db,
        oid,
        PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE,
        bytes,
        ErrorCode::INVALID_DATA_TYPE,
        baseline,
        "wrong-typed value",
    );
}

#[test]
fn pulse_converter_input_reference_over_the_wire() {
    let mut db = ObjectDatabase::new();
    let pc = PulseConverterObject::new(1, "PC-1", 62).unwrap();
    let oid = pc.object_identifier();
    db.add(Box::new(pc)).unwrap();

    let target = ObjectIdentifier::new(ObjectType::ACCUMULATOR, 1).unwrap();
    let present_value = PropertyIdentifier::PRESENT_VALUE.to_raw();
    write_raw(
        &mut db,
        oid,
        PropertyIdentifier::INPUT_REFERENCE,
        framed_reference(&BACnetObjectPropertyReference::new_indexed(
            target,
            present_value,
            4,
        )),
    )
    .unwrap();
    assert_eq!(
        read_prop(&db, oid, PropertyIdentifier::INPUT_REFERENCE),
        PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(target),
            PropertyValue::Enumerated(present_value),
            PropertyValue::Unsigned(4),
        ])
    );
}

#[test]
fn setpoint_reference_empty_frame_and_null_both_clear_over_the_wire() {
    let mut db = ObjectDatabase::new();
    let oid = add_loop(&mut db, 3);
    let target = reference_target();
    let present_value = PropertyIdentifier::PRESENT_VALUE.to_raw();

    let set = |db: &mut ObjectDatabase| {
        let mut wrapped = BytesMut::new();
        bacnet_encoding::constructed::encode_setpoint_reference(
            &mut wrapped,
            &BACnetObjectPropertyReference::new(target, present_value),
        );
        write_raw(
            db,
            oid,
            PropertyIdentifier::SETPOINT_REFERENCE,
            wrapped.to_vec(),
        )
        .unwrap();
    };

    // Both clear paths are conformant and accepted: the empty
    // BACnetSetpointReference frame (OPTIONAL member absent — Clause 12.17
    // says the setpoint is then fixed in the Setpoint property) and Null.
    for (label, clearing_bytes) in [
        ("empty BACnetSetpointReference frame", vec![0x0E, 0x0F]),
        ("application-tagged Null", encode_value(PropertyValue::Null)),
    ] {
        set(&mut db);
        assert_ne!(
            read_prop(&db, oid, PropertyIdentifier::SETPOINT_REFERENCE),
            PropertyValue::Null
        );
        write_raw(
            &mut db,
            oid,
            PropertyIdentifier::SETPOINT_REFERENCE,
            clearing_bytes,
        )
        .unwrap_or_else(|e| panic!("{label}: clear must be accepted: {e:?}"));
        assert_eq!(
            read_prop(&db, oid, PropertyIdentifier::SETPOINT_REFERENCE),
            PropertyValue::Null,
            "{label}: reference must be cleared"
        );
    }
}

#[test]
fn averaging_object_property_reference_over_the_wire() {
    let mut db = ObjectDatabase::new();
    let avg = AveragingObject::new(1, "AVG-1").unwrap();
    let oid = avg.object_identifier();
    db.add(Box::new(avg)).unwrap();

    let target = reference_target();
    let present_value = PropertyIdentifier::PRESENT_VALUE.to_raw();
    let baseline = PropertyValue::List(vec![
        PropertyValue::ObjectIdentifier(target),
        PropertyValue::Unsigned(present_value as u64),
    ]);

    // Framed local reference lands; the Averaging read keeps its historical
    // Unsigned member emission.
    write_raw(
        &mut db,
        oid,
        PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
        framed_reference(&BACnetObjectPropertyReference::new(target, present_value)),
    )
    .unwrap();
    assert_eq!(
        read_prop(&db, oid, PropertyIdentifier::OBJECT_PROPERTY_REFERENCE),
        baseline
    );

    // Device-qualified [3] write is refused (remote sampling is the
    // standard's OPTIONAL branch, unmodeled) with the reference preserved.
    let mut framed = BytesMut::new();
    framed.extend_from_slice(&framed_reference(&BACnetObjectPropertyReference::new(
        target,
        present_value,
    )));
    bacnet_encoding::primitives::encode_ctx_object_id(
        &mut framed,
        3,
        &ObjectIdentifier::new(ObjectType::DEVICE, 42).unwrap(),
    );
    assert_refused(
        &mut db,
        oid,
        PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
        framed.to_vec(),
        ErrorCode::INVALID_DATA_ENCODING,
        baseline.clone(),
        "device-qualified Object_Property_Reference",
    );

    // And the pre-fix silent-drop shapes refuse over the wire: 4 members.
    let bytes = encode_value(PropertyValue::List(vec![
        PropertyValue::ObjectIdentifier(target),
        PropertyValue::Unsigned(present_value as u64),
        PropertyValue::Unsigned(2),
        PropertyValue::Unsigned(9),
    ]));
    assert_refused(
        &mut db,
        oid,
        PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
        bytes,
        ErrorCode::INVALID_DATA_TYPE,
        baseline,
        "4-member flat reference",
    );
}

#[test]
fn wpm_reference_write_commits_in_order_and_rolls_back_on_failure() {
    use bacnet_services::common::BACnetPropertyValue;
    use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};

    let mut db = ObjectDatabase::new();
    let oid = add_loop(&mut db, 2);
    let target = reference_target();
    let present_value = PropertyIdentifier::PRESENT_VALUE.to_raw();

    let wpm = |db: &mut ObjectDatabase, props: Vec<BACnetPropertyValue>| {
        let request = WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: oid,
                list_of_properties: props,
            }],
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);
        handle_write_property_multiple(db, &buf)
    };

    // Whole request applies atomically on success.
    wpm(
        &mut db,
        vec![
            BACnetPropertyValue {
                property_identifier: PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE,
                property_array_index: None,
                value: framed_reference(&BACnetObjectPropertyReference::new(target, present_value)),
                priority: None,
            },
            BACnetPropertyValue {
                property_identifier: PropertyIdentifier::SETPOINT,
                property_array_index: None,
                value: encode_value(PropertyValue::Real(21.5)),
                priority: None,
            },
        ],
    )
    .unwrap();
    assert_eq!(
        read_prop(&db, oid, PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE),
        PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(target),
            PropertyValue::Enumerated(present_value),
        ])
    );
    assert_eq!(
        read_prop(&db, oid, PropertyIdentifier::SETPOINT),
        PropertyValue::Real(21.5)
    );

    // Failing second property rolls the whole request back: the reference
    // returns to Null, not to a half-committed new value.
    let err = wpm(
        &mut db,
        vec![
            BACnetPropertyValue {
                property_identifier: PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE,
                property_array_index: None,
                value: framed_reference(&BACnetObjectPropertyReference::new(
                    ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 3).unwrap(),
                    present_value,
                )),
                priority: None,
            },
            BACnetPropertyValue {
                property_identifier: PropertyIdentifier::SETPOINT,
                property_array_index: None,
                value: encode_value(PropertyValue::Unsigned(42)), // wrong type: fails
                priority: None,
            },
        ],
    )
    .unwrap_err();
    match err {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
            assert_eq!(code, ErrorCode::INVALID_DATA_TYPE.to_raw() as u32);
        }
        other => panic!("expected PROPERTY/INVALID_DATA_TYPE, got {other:?}"),
    }
    assert_eq!(
        read_prop(&db, oid, PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE),
        PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(target),
            PropertyValue::Enumerated(present_value),
        ]),
        "rolled back to the pre-request reference"
    );
    assert_eq!(
        read_prop(&db, oid, PropertyIdentifier::SETPOINT),
        PropertyValue::Real(21.5),
        "rolled back to the pre-request setpoint"
    );
}
