//! The Loop's three references read and take writes in their Clause 21
//! encodings (#1312): Controlled_Variable_Reference and
//! Manipulated_Variable_Reference as `BACnetObjectPropertyReference`,
//! Setpoint_Reference as `BACnetSetpointReference`.

use super::*;
use bacnet_types::error::RemoteError;

const CVR: PropertyIdentifier = PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE;
const MVR: PropertyIdentifier = PropertyIdentifier::MANIPULATED_VARIABLE_REFERENCE;
const SR: PropertyIdentifier = PropertyIdentifier::SETPOINT_REFERENCE;
const PRESENT_VALUE: u32 = 85;

/// `[0]` analog-input 7, `[1]` present-value.
const AI_7_PV: [u8; 7] = [0x0C, 0x00, 0x00, 0x00, 0x07, 0x19, 0x55];
/// `[0]` analog-output 3, `[1]` present-value, `[2]` index 4.
const AO_3_PV_4: [u8; 9] = [0x0C, 0x00, 0x40, 0x00, 0x03, 0x19, 0x55, 0x29, 0x04];
/// Opening tag 0, `[0]` analog-value 10, `[1]` present-value, closing tag 0.
const AV_10_PV_FRAMED: [u8; 9] = [0x0E, 0x0C, 0x00, 0x80, 0x00, 0x0A, 0x19, 0x55, 0x0F];
/// The unset Controlled_Variable_Reference (#1417): `[0]` analog-input
/// 4194303, `[1]` present-value.
const UNSET_CVR: [u8; 7] = [0x0C, 0x00, 0x3F, 0xFF, 0xFF, 0x19, 0x55];
/// The unset Manipulated_Variable_Reference: `[0]` analog-output 4194303,
/// `[1]` present-value.
const UNSET_MVR: [u8; 7] = [0x0C, 0x00, 0x7F, 0xFF, 0xFF, 0x19, 0x55];

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn data(bytes: &[u8]) -> PropertyValue {
    PropertyValue::ApplicationData(bytes.to_vec())
}

fn read(lo: &LoopObject, property: PropertyIdentifier) -> PropertyValue {
    lo.read_property(property, None).unwrap()
}

fn write(lo: &mut LoopObject, property: PropertyIdentifier, value: PropertyValue) {
    lo.write_property(property, None, value, None)
        .unwrap_or_else(|e| panic!("{property:?} write refused: {e:?}"));
}

/// A refused write carries PROPERTY / `expected` and leaves the property as
/// it read before.
fn assert_refused(
    lo: &mut LoopObject,
    property: PropertyIdentifier,
    value: PropertyValue,
    expected: ErrorCode,
) {
    let before = read(lo, property);
    let error = lo
        .write_property(property, None, value.clone(), None)
        .unwrap_err();
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::PROPERTY && code == expected),
        "{property:?} <- {value:?}: expected PROPERTY/{expected:?}, got {error:?}"
    );
    assert_eq!(read(lo, property), before, "{property:?} changed");
}

/// A Loop with all three references set by its setters.
fn configured_loop() -> LoopObject {
    let mut lo = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    lo.set_controlled_variable_reference(BACnetObjectPropertyReference::new(
        oid(ObjectType::ANALOG_INPUT, 7),
        PRESENT_VALUE,
    ));
    lo.set_manipulated_variable_reference(BACnetObjectPropertyReference::new_indexed(
        oid(ObjectType::ANALOG_OUTPUT, 3),
        PRESENT_VALUE,
        4,
    ));
    lo.set_setpoint_reference(BACnetObjectPropertyReference::new(
        oid(ObjectType::ANALOG_VALUE, 10),
        PRESENT_VALUE,
    ));
    lo
}

#[test]
fn loop_unset_references_read_the_reserved_instance_or_the_empty_setpoint_reference() {
    let lo = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    assert_eq!(read(&lo, CVR), data(&UNSET_CVR));
    assert_eq!(read(&lo, MVR), data(&UNSET_MVR));
    // A BACnetSetpointReference without its optional member encodes as
    // nothing at all.
    assert_eq!(read(&lo, SR), data(&[]));
}

#[test]
fn loop_set_references_read_as_their_clause_21_encodings() {
    let lo = configured_loop();
    assert_eq!(read(&lo, CVR), data(&AI_7_PV));
    assert_eq!(read(&lo, MVR), data(&AO_3_PV_4));
    assert_eq!(read(&lo, SR), data(&AV_10_PV_FRAMED));
}

#[test]
fn loop_references_in_property_list() {
    let lo = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let list = lo.property_list();
    for property in [CVR, MVR, SR] {
        assert!(list.contains(&property), "{property:?} missing");
    }
}

#[test]
fn loop_reference_writes_take_the_encodings_they_read_as() {
    let mut lo = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    for (property, bytes) in [
        (CVR, &AO_3_PV_4[..]),
        (MVR, &AI_7_PV[..]),
        (SR, &AV_10_PV_FRAMED[..]),
    ] {
        write(&mut lo, property, data(bytes));
        assert_eq!(read(&lo, property), data(bytes), "{property:?}");
    }
    // A value read from one Loop writes back to another unchanged.
    let source = configured_loop();
    for property in [CVR, MVR, SR] {
        write(&mut lo, property, read(&source, property));
        assert_eq!(read(&lo, property), read(&source, property));
    }
}

#[test]
fn loop_reference_writes_clear_with_the_unset_values_they_read_as() {
    let mut lo = configured_loop();
    write(&mut lo, CVR, data(&UNSET_CVR));
    write(&mut lo, MVR, data(&UNSET_MVR));
    write(&mut lo, SR, data(&[]));
    assert_eq!(read(&lo, CVR), data(&UNSET_CVR));
    assert_eq!(read(&lo, MVR), data(&UNSET_MVR));
    assert_eq!(read(&lo, SR), data(&[]));
    // Any reference to the reserved instance clears one, whatever else it
    // names (here analog-value 4194303's priority-array); the read gives the
    // property's own unset form back (#1417).
    let unset = LoopObject::new(2, "LOOP-2", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    for property in [CVR, MVR] {
        let mut lo = configured_loop();
        write(
            &mut lo,
            property,
            data(&[0x0C, 0x00, 0xBF, 0xFF, 0xFF, 0x19, 0x57]),
        );
        assert_eq!(read(&lo, property), read(&unset, property), "{property:?}");
    }
    // A setter given the unset form clears the reference too.
    let mut lo = configured_loop();
    lo.set_controlled_variable_reference(crate::reference::unset_reference(
        ObjectType::ANALOG_VALUE,
    ));
    assert_eq!(read(&lo, CVR), data(&UNSET_CVR));
}

#[test]
fn loop_null_reference_writes_are_refused_as_another_datatype() {
    // A NULL is no BACnetObjectPropertyReference or BACnetSetpointReference:
    // INVALID_DATA_TYPE, which the server answers as the Clause 15.9.2 no-op.
    let mut lo = configured_loop();
    for property in [CVR, MVR, SR] {
        assert_refused(
            &mut lo,
            property,
            PropertyValue::Null,
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
}

#[test]
fn loop_flat_reference_writes_are_refused_and_change_nothing() {
    let mut lo = configured_loop();
    let target = oid(ObjectType::ANALOG_INPUT, 9);
    for property in [CVR, MVR, SR] {
        for flat in [
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(target),
                PropertyValue::Enumerated(PRESENT_VALUE),
            ]),
            PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(target),
                PropertyValue::Enumerated(PRESENT_VALUE),
                PropertyValue::Unsigned(2),
            ]),
        ] {
            assert_refused(&mut lo, property, flat, ErrorCode::INVALID_DATA_TYPE);
        }
    }
}

#[test]
fn loop_reference_writes_of_other_datatypes_or_bad_octets_change_nothing() {
    let mut lo = configured_loop();
    // Another datatype: Null and the bare members on Setpoint_Reference, the
    // setpoint frame on a bare reference, a scalar anywhere.
    assert_refused(
        &mut lo,
        SR,
        PropertyValue::Null,
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_refused(&mut lo, SR, data(&AI_7_PV), ErrorCode::INVALID_DATA_TYPE);
    assert_refused(
        &mut lo,
        CVR,
        data(&AV_10_PV_FRAMED),
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_refused(
        &mut lo,
        MVR,
        PropertyValue::Unsigned(42),
        ErrorCode::INVALID_DATA_TYPE,
    );
    // Octets that open right but don't decode: a device member [3] (not
    // part of BACnetObjectPropertyReference), a frame holding no
    // reference, a reference cut short.
    let with_device = [&AI_7_PV[..], &[0x3C, 0x02, 0x00, 0x00, 0x4D]].concat();
    assert_refused(
        &mut lo,
        CVR,
        data(&with_device),
        ErrorCode::INVALID_DATA_ENCODING,
    );
    assert_refused(
        &mut lo,
        SR,
        data(&[0x0E, 0x0F]),
        ErrorCode::INVALID_DATA_ENCODING,
    );
    assert_refused(
        &mut lo,
        MVR,
        data(&AO_3_PV_4[..5]),
        ErrorCode::INVALID_DATA_ENCODING,
    );
    assert_refused(&mut lo, CVR, data(&[]), ErrorCode::INVALID_DATA_ENCODING);
}
