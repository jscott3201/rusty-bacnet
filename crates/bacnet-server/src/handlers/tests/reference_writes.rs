//! Wire-level integration for the structured reference properties (#182,
//! #1312). The Loop references (Clause 12.17) and Pulse Converter
//! Input_Reference (Clause 12.23) read and take writes as their Clause 21
//! encodings: `BACnetObjectPropertyReference`, and `BACnetSetpointReference`
//! for Setpoint_Reference. The Averaging `Object_Property_Reference` (Clause
//! 12.5) is the device-qualifying sibling production. A device member [3] is
//! refused: INVALID_DATA_ENCODING where the production has no [3], and
//! OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED on the Averaging (#1153).

use super::*;
use bacnet_objects::accumulator::PulseConverterObject;
use bacnet_objects::averaging::AveragingObject;
use bacnet_objects::loop_obj::LoopObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{
    BACnetObjectPropertyReference, PropertyReference, ReadAccessSpecification,
};
use bacnet_types::enums::EngineeringUnits;

const CVR: PropertyIdentifier = PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE;
const MVR: PropertyIdentifier = PropertyIdentifier::MANIPULATED_VARIABLE_REFERENCE;
const SR: PropertyIdentifier = PropertyIdentifier::SETPOINT_REFERENCE;
const INPUT: PropertyIdentifier = PropertyIdentifier::INPUT_REFERENCE;

/// `[0]` analog-input 7, `[1]` present-value.
const AI_7_PV: [u8; 7] = [0x0C, 0x00, 0x00, 0x00, 0x07, 0x19, 0x55];
/// The flat application-tagged list the Loop and Pulse Converter used to
/// serve for the same reference: object identifier, then Enumerated.
const AI_7_PV_FLAT: [u8; 7] = [0xC4, 0x00, 0x00, 0x00, 0x07, 0x91, 0x55];
/// The unset forms (#1417): `[0]` analog-input, analog-output or accumulator
/// 4194303, `[1]` present-value.
const UNSET_AI: [u8; 7] = [0x0C, 0x00, 0x3F, 0xFF, 0xFF, 0x19, 0x55];
const UNSET_AO: [u8; 7] = [0x0C, 0x00, 0x7F, 0xFF, 0xFF, 0x19, 0x55];
const UNSET_ACCUMULATOR: [u8; 7] = [0x0C, 0x05, 0xFF, 0xFF, 0xFF, 0x19, 0x55];

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
    request.encode(&mut buf).unwrap();
    handle_write_property(db, &buf).map(|_| ())
}

/// The value bytes ReadProperty serves for `property`, after checking that
/// ReadPropertyMultiple serves the same ones.
fn read_raw(db: &ObjectDatabase, oid: ObjectIdentifier, property: PropertyIdentifier) -> Vec<u8> {
    let request = ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    let mut ack_buf = BytesMut::new();
    handle_read_property(db, &buf, &mut ack_buf).unwrap();
    let value = ReadPropertyACK::decode(&ack_buf).unwrap().property_value;

    let mut request = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: vec![PropertyReference {
                property_identifier: property,
                property_array_index: None,
            }],
        }],
    }
    .encode(&mut request)
    .unwrap();
    let mut ack = BytesMut::new();
    handle_read_property_multiple(db, &request, &mut ack).unwrap();
    let ack = ReadPropertyMultipleACK::decode(&ack).unwrap();
    assert_eq!(
        ack.list_of_read_access_results[0].list_of_results[0].property_value,
        Some(value.clone()),
        "{property:?}: RPM serves other bytes than RP"
    );
    value
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
    let lo = LoopObject::new(
        instance,
        format!("LOOP-{instance}"),
        EngineeringUnits::DEGREES_CELSIUS,
    )
    .unwrap();
    let oid = lo.object_identifier();
    db.add(Box::new(lo)).unwrap();
    oid
}

fn add_pulse_converter(db: &mut ObjectDatabase, instance: u32) -> ObjectIdentifier {
    let pc = PulseConverterObject::new(
        instance,
        format!("PC-{instance}"),
        EngineeringUnits::DEGREES_CELSIUS,
    )
    .unwrap();
    let oid = pc.object_identifier();
    db.add(Box::new(pc)).unwrap();
    oid
}

/// A refused write carries exactly PROPERTY / `expected_code` and leaves
/// `property` serving the bytes it served before.
fn assert_refused(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    raw_value: Vec<u8>,
    expected_code: ErrorCode,
    context: &str,
) {
    let before = read_raw(db, oid, property);
    match write_raw(db, oid, property, raw_value).expect_err(context) {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::PROPERTY, "{context}: wrong error class");
            assert_eq!(code, expected_code, "{context}: wrong error code");
        }
        other => panic!("{context}: expected PROPERTY/{expected_code:?}, got {other:?}"),
    }
    assert_eq!(
        read_raw(db, oid, property),
        before,
        "{context}: refused write must leave the property unchanged"
    );
}

#[test]
fn loop_references_read_and_write_in_their_clause_21_encodings() {
    let mut db = ObjectDatabase::new();
    let oid = add_loop(&mut db, 1);

    // Unset: the reserved instance for the two plain references (#1417),
    // and for Setpoint_Reference the sequence without its optional member,
    // which has no octets.
    assert_eq!(read_raw(&db, oid, CVR), UNSET_AI);
    assert_eq!(read_raw(&db, oid, MVR), UNSET_AO);
    assert_eq!(read_raw(&db, oid, SR), [0u8; 0]);

    let cases: [(PropertyIdentifier, &[u8]); 3] = [
        // [0] analog-input 7, [1] present-value, [2] index 3.
        (CVR, &[0x0C, 0x00, 0x00, 0x00, 0x07, 0x19, 0x55, 0x29, 0x03]),
        // [0] analog-output 3, [1] present-value.
        (MVR, &[0x0C, 0x00, 0x40, 0x00, 0x03, 0x19, 0x55]),
        // Opening tag 0, [0] analog-value 10, [1] present-value, closing tag 0.
        (SR, &[0x0E, 0x0C, 0x00, 0x80, 0x00, 0x0A, 0x19, 0x55, 0x0F]),
    ];
    for (property, bytes) in cases {
        write_raw(&mut db, oid, property, bytes.to_vec())
            .unwrap_or_else(|e| panic!("{property:?}: {e:?}"));
        assert_eq!(read_raw(&db, oid, property), bytes, "{property:?}");
    }
}

#[test]
fn pulse_converter_input_reference_reads_and_writes_in_its_clause_21_encoding() {
    let mut db = ObjectDatabase::new();
    let oid = add_pulse_converter(&mut db, 1);
    assert_eq!(read_raw(&db, oid, INPUT), UNSET_ACCUMULATOR);

    // [0] accumulator 1, [1] present-value, [2] index 4.
    let bytes = [0x0C, 0x05, 0xC0, 0x00, 0x01, 0x19, 0x55, 0x29, 0x04];
    write_raw(&mut db, oid, INPUT, bytes.to_vec()).unwrap();
    assert_eq!(read_raw(&db, oid, INPUT), bytes);
}

#[test]
fn reference_writes_clear_with_the_unset_values_they_read_as() {
    let mut db = ObjectDatabase::new();
    let lo = add_loop(&mut db, 1);
    let pc = add_pulse_converter(&mut db, 1);
    let framed_setpoint = [&[0x0E][..], &AI_7_PV, &[0x0F]].concat();
    for (oid, property, set, clear) in [
        (lo, CVR, AI_7_PV.to_vec(), UNSET_AI.to_vec()),
        (lo, MVR, AI_7_PV.to_vec(), UNSET_AO.to_vec()),
        (lo, SR, framed_setpoint, Vec::new()),
        (pc, INPUT, AI_7_PV.to_vec(), UNSET_ACCUMULATOR.to_vec()),
    ] {
        write_raw(&mut db, oid, property, set.clone()).unwrap();
        assert_eq!(read_raw(&db, oid, property), set);
        // A NULL is no reference and none of these is commandable, so it
        // succeeds and changes nothing (Clause 15.9.2, #1417).
        write_raw(&mut db, oid, property, vec![0x00])
            .unwrap_or_else(|e| panic!("{property:?}: NULL refused: {e:?}"));
        assert_eq!(
            read_raw(&db, oid, property),
            set,
            "{property:?}: NULL changed it"
        );
        write_raw(&mut db, oid, property, clear.clone())
            .unwrap_or_else(|e| panic!("{property:?}: clearing refused: {e:?}"));
        assert_eq!(read_raw(&db, oid, property), clear, "{property:?}");
        // The unset value written back is accepted and changes nothing.
        write_raw(&mut db, oid, property, clear.clone()).unwrap();
        assert_eq!(read_raw(&db, oid, property), clear, "{property:?}");
    }
}

#[test]
fn flat_reference_writes_are_refused_and_change_nothing() {
    let mut db = ObjectDatabase::new();
    let lo = add_loop(&mut db, 1);
    let pc = add_pulse_converter(&mut db, 1);
    // [0] analog-output 3, [1] present-value, set first so the refusal has
    // something to preserve.
    let held = vec![0x0C, 0x00, 0x40, 0x00, 0x03, 0x19, 0x55];
    let indexed_flat = [&AI_7_PV_FLAT[..], &[0x21, 0x03]].concat();
    for (oid, property, value) in [
        (lo, CVR, held.clone()),
        (lo, MVR, held.clone()),
        (lo, SR, [&[0x0E][..], &held, &[0x0F]].concat()),
        (pc, INPUT, held.clone()),
    ] {
        write_raw(&mut db, oid, property, value).unwrap();
        for flat in [AI_7_PV_FLAT.to_vec(), indexed_flat.clone()] {
            assert_refused(
                &mut db,
                oid,
                property,
                flat,
                ErrorCode::INVALID_DATA_TYPE,
                &format!("flat write to {property:?}"),
            );
        }
    }
}

#[test]
fn reference_write_rejections_over_the_wire_preserve_state() {
    let mut db = ObjectDatabase::new();
    let oid = add_loop(&mut db, 1);
    let r = BACnetObjectPropertyReference::new(
        reference_target(),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    );
    write_raw(&mut db, oid, CVR, framed_reference(&r)).unwrap();
    let mut wrapped = BytesMut::new();
    bacnet_encoding::constructed::encode_setpoint_reference(&mut wrapped, &r);
    write_raw(&mut db, oid, SR, wrapped.to_vec()).unwrap();

    // Device-qualified ([3]): not part of the BACnetObjectPropertyReference
    // production — INVALID_DATA_ENCODING.
    let mut framed = BytesMut::new();
    framed.extend_from_slice(&framed_reference(&r));
    bacnet_encoding::primitives::encode_ctx_object_id(
        &mut framed,
        3,
        &ObjectIdentifier::new(ObjectType::DEVICE, 77).unwrap(),
    );
    let cases: Vec<(PropertyIdentifier, Vec<u8>, ErrorCode, &str)> = vec![
        (
            CVR,
            framed.to_vec(),
            ErrorCode::INVALID_DATA_ENCODING,
            "device-qualified reference",
        ),
        (
            CVR,
            [framed_reference(&r), vec![0x49, 0x01]].concat(),
            ErrorCode::INVALID_DATA_ENCODING,
            "unknown trailing context tag",
        ),
        (
            CVR,
            Vec::new(),
            ErrorCode::INVALID_DATA_ENCODING,
            "empty value on a plain reference",
        ),
        (
            CVR,
            encode_value(PropertyValue::List(vec![
                PropertyValue::Unsigned(1),
                PropertyValue::Unsigned(2),
            ])),
            ErrorCode::INVALID_DATA_TYPE,
            "two Unsigneds",
        ),
        (
            CVR,
            wrapped.to_vec(),
            ErrorCode::INVALID_DATA_TYPE,
            "setpoint frame on a plain reference",
        ),
        // BACnetSetpointReference: the bare members are another datatype; a
        // frame holding no reference, or more than one, is malformed.
        (
            SR,
            framed_reference(&r),
            ErrorCode::INVALID_DATA_TYPE,
            "bare members on Setpoint_Reference",
        ),
        (
            SR,
            vec![0x0E, 0x0F],
            ErrorCode::INVALID_DATA_ENCODING,
            "empty setpoint frame",
        ),
        (
            SR,
            [
                &wrapped[..wrapped.len() - 1],
                &framed_reference(&r),
                &[0x0F],
            ]
            .concat(),
            ErrorCode::INVALID_DATA_ENCODING,
            "two references in one frame",
        ),
    ];
    for (property, bytes, code, context) in cases {
        assert_refused(&mut db, oid, property, bytes, code, context);
    }
    // NULL isn't a BACnetSetpointReference and the property isn't
    // commandable, so a NULL succeeds and leaves it as it is (#1396).
    let before = read_raw(&db, oid, SR);
    write_raw(&mut db, oid, SR, vec![0x00]).unwrap();
    assert_eq!(read_raw(&db, oid, SR), before);
}

#[test]
fn averaging_object_property_reference_over_the_wire() {
    let mut db = ObjectDatabase::new();
    let avg = AveragingObject::new(1, "AVG-1").unwrap();
    let oid = avg.object_identifier();
    db.add(Box::new(avg)).unwrap();

    let target = reference_target();
    let present_value = PropertyIdentifier::PRESENT_VALUE.to_raw();
    let opr = PropertyIdentifier::OBJECT_PROPERTY_REFERENCE;

    // Framed local reference lands, and a read serves the same Clause 21
    // bytes: [0] analog-input 7, [1] present-value, no Device (#1182).
    write_raw(
        &mut db,
        oid,
        opr,
        framed_reference(&BACnetObjectPropertyReference::new(target, present_value)),
    )
    .unwrap();
    assert_eq!(read_raw(&db, oid, opr), AI_7_PV);

    // The flat application-tagged form reads used to serve is another
    // datatype, and a [3] naming no Device is out of range (#1182).
    assert_refused(
        &mut db,
        oid,
        opr,
        encode_value(PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(target),
            PropertyValue::Unsigned(present_value as u64),
        ])),
        ErrorCode::INVALID_DATA_TYPE,
        "flat reference",
    );
    // Octets that don't open with the object identifier's primitive tag 0
    // aren't this datatype either: the members out of order, or wrapped in a
    // [0] frame (#1312, as device_reference.rs answers for #1311). Octets
    // that open with it and then break stay an encoding error.
    for (bytes, code, context) in [
        (
            [&AI_7_PV[5..], &AI_7_PV[..5]].concat(),
            ErrorCode::INVALID_DATA_TYPE,
            "[1] before [0]",
        ),
        (
            [&[0x0E][..], &AI_7_PV, &[0x0F]].concat(),
            ErrorCode::INVALID_DATA_TYPE,
            "reference wrapped in a [0] frame",
        ),
        (
            vec![0x0C, 0x00, 0x00, 0x00, 0x07, 0x91, 0x55],
            ErrorCode::INVALID_DATA_ENCODING,
            "[0] then an application-tagged property",
        ),
    ] {
        assert_refused(&mut db, oid, opr, bytes, code, context);
    }
    assert_refused(
        &mut db,
        oid,
        opr,
        [&AI_7_PV[..], &[0x3C, 0x00, 0x00, 0x00, 0x2A]].concat(),
        ErrorCode::VALUE_OUT_OF_RANGE,
        "[3] naming analog-input 42",
    );

    // Device-qualified [3] write is refused (remote sampling is the
    // standard's OPTIONAL branch, unmodeled) with the reference preserved.
    // The database has no Device, so Device 42 is never this device; the
    // encoding is valid for this production, so the refusal names the
    // missing remote support (#1153).
    let mut framed = BytesMut::new();
    framed.extend_from_slice(&AI_7_PV);
    bacnet_encoding::primitives::encode_ctx_object_id(
        &mut framed,
        3,
        &ObjectIdentifier::new(ObjectType::DEVICE, 42).unwrap(),
    );
    assert_refused(
        &mut db,
        oid,
        opr,
        framed.to_vec(),
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
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
        opr,
        bytes,
        ErrorCode::INVALID_DATA_TYPE,
        "4-member flat reference",
    );

    // A NULL is no reference and the property isn't commandable: it
    // succeeds and changes nothing (Clause 15.9.2, #1417).
    write_raw(&mut db, oid, opr, vec![0x00]).unwrap();
    assert_eq!(read_raw(&db, oid, opr), AI_7_PV);
    // The unset form clears it, and is what the property then serves; so
    // does a reference naming Device 4194303, which names no device.
    write_raw(&mut db, oid, opr, UNSET_AI.to_vec()).unwrap();
    assert_eq!(read_raw(&db, oid, opr), UNSET_AI);
    write_raw(&mut db, oid, opr, AI_7_PV.to_vec()).unwrap();
    write_raw(
        &mut db,
        oid,
        opr,
        [&AI_7_PV[..], &[0x3C, 0x02, 0x3F, 0xFF, 0xFF]].concat(),
    )
    .unwrap();
    assert_eq!(read_raw(&db, oid, opr), UNSET_AI);
}

#[test]
fn event_enrollment_fault_parameters_take_the_context_tagged_none_and_ignore_a_null() {
    use bacnet_objects::event_enrollment::EventEnrollmentObject;
    use bacnet_types::enums::EventType;

    const FP: PropertyIdentifier = PropertyIdentifier::FAULT_PARAMETERS;
    let mut db = ObjectDatabase::new();
    let ee = EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_BITSTRING).unwrap();
    let oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();
    // Unset: the context-tagged `none` choice, [0] with no octets (#1417).
    assert_eq!(read_raw(&db, oid, FP), [0x08]);
    // fault-out-of-range [6]: min-normal [0] { Double 1.0 }, max-normal [1]
    // { Double 2.0 }.
    let out_of_range = [
        0x6E, 0x0E, 0x55, 0x08, 0x3F, 0xF0, 0, 0, 0, 0, 0, 0, 0x0F, 0x1E, 0x55, 0x08, 0x40, 0, 0,
        0, 0, 0, 0, 0, 0x1F, 0x6F,
    ];
    write_raw(&mut db, oid, FP, out_of_range.to_vec()).unwrap();
    assert_eq!(read_raw(&db, oid, FP), out_of_range);
    // The CHOICE's `none` is context-tagged, so an application NULL is no
    // member of it: the write succeeds and changes nothing (Clause 15.9.2).
    write_raw(&mut db, oid, FP, vec![0x00]).unwrap();
    assert_eq!(read_raw(&db, oid, FP), out_of_range);
    // The context-tagged `none` clears it.
    write_raw(&mut db, oid, FP, vec![0x08]).unwrap();
    assert_eq!(read_raw(&db, oid, FP), [0x08]);
}

#[test]
fn wpm_reference_write_commits_in_order_and_keeps_prefix_on_failure() {
    use bacnet_services::common::BACnetPropertyValue;
    use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};

    let mut db = ObjectDatabase::new();
    let oid = add_loop(&mut db, 2);
    let present_value = PropertyIdentifier::PRESENT_VALUE.to_raw();

    let wpm = |db: &mut ObjectDatabase, props: Vec<BACnetPropertyValue>| {
        let request = WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: oid,
                list_of_properties: props,
            }],
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf).unwrap();
        handle_write_property_multiple(db, &buf)
    };

    // The complete request applies in order on success.
    wpm(
        &mut db,
        vec![
            BACnetPropertyValue {
                property_identifier: CVR,
                property_array_index: None,
                value: AI_7_PV.to_vec(),
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
    assert_eq!(read_raw(&db, oid, CVR), AI_7_PV);
    assert_eq!(
        read_raw(&db, oid, PropertyIdentifier::SETPOINT),
        encode_value(PropertyValue::Real(21.5))
    );

    // A failing second property leaves the first reference write committed.
    let analog_output_3 = framed_reference(&BACnetObjectPropertyReference::new(
        ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 3).unwrap(),
        present_value,
    ));
    let err = wpm(
        &mut db,
        vec![
            BACnetPropertyValue {
                property_identifier: CVR,
                property_array_index: None,
                value: analog_output_3.clone(),
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
            assert_eq!(class, ErrorClass::PROPERTY);
            assert_eq!(code, ErrorCode::INVALID_DATA_TYPE);
        }
        other => panic!("expected PROPERTY/INVALID_DATA_TYPE, got {other:?}"),
    }
    assert_eq!(
        read_raw(&db, oid, CVR),
        analog_output_3,
        "the successful reference prefix stays committed"
    );
    assert_eq!(
        read_raw(&db, oid, PropertyIdentifier::SETPOINT),
        encode_value(PropertyValue::Real(21.5)),
        "the failed setpoint attempt is mutation-free"
    );

    // The old flat form fails the WPM at its own attempt, before anything.
    let err = wpm(
        &mut db,
        vec![BACnetPropertyValue {
            property_identifier: MVR,
            property_array_index: None,
            value: AI_7_PV_FLAT.to_vec(),
            priority: None,
        }],
    )
    .unwrap_err();
    assert!(
        matches!(err, Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::INVALID_DATA_TYPE),
        "{err:?}"
    );
    assert_eq!(read_raw(&db, oid, MVR), UNSET_AO);
}
