//! An application running a Loop's algorithm follows the references a client
//! writes (#1312). The stack doesn't run the algorithm itself: the
//! application reads each reference as the server serves it, decodes it with
//! the shared Clause 21 codecs, and then measures, takes the setpoint and
//! commands the output where the references point. A WriteProperty that
//! re-points a reference moves the next pass to the new property.

use super::*;
use bacnet_encoding::constructed::{decode_object_property_reference, decode_setpoint_reference};
use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::loop_obj::LoopObject;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::EngineeringUnits;

const CVR: PropertyIdentifier = PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE;
const MVR: PropertyIdentifier = PropertyIdentifier::MANIPULATED_VARIABLE_REFERENCE;
const SR: PropertyIdentifier = PropertyIdentifier::SETPOINT_REFERENCE;
const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn write(
    db: &mut ObjectDatabase,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    property_value: Vec<u8>,
    priority: Option<u8>,
) {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: object,
        property_identifier: property,
        property_array_index: None,
        property_value,
        priority,
    }
    .encode(&mut request)
    .unwrap();
    sourced_wp(db, &request)
        .unwrap_or_else(|e| panic!("{object:?} {property:?} write refused: {e:?}"));
}

fn read_bytes(
    db: &ObjectDatabase,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
) -> Vec<u8> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: object,
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut request);
    let mut ack = BytesMut::new();
    handle_read_property(db, &request, &mut ack).unwrap();
    ReadPropertyACK::decode(&ack).unwrap().property_value
}

fn real(value: f32) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_property_value(&mut buf, &PropertyValue::Real(value)).unwrap();
    buf.to_vec()
}

fn read_real(db: &ObjectDatabase, object: ObjectIdentifier, property: PropertyIdentifier) -> f32 {
    match db
        .get(&object)
        .unwrap()
        .read_property(property, None)
        .unwrap()
    {
        PropertyValue::Real(value) => value,
        other => panic!("{object:?} {property:?} is {other:?}"),
    }
}

fn reference_bytes(target: ObjectIdentifier) -> Vec<u8> {
    let mut buf = BytesMut::new();
    bacnet_encoding::constructed::encode_object_property_reference(
        &mut buf,
        &BACnetObjectPropertyReference::new(target, PV.to_raw()),
    );
    buf.to_vec()
}

/// One pass of the application's algorithm, reading every reference through
/// ReadProperty: Controlled_Variable_Value from the controlled variable,
/// the setpoint from Setpoint_Reference or else Setpoint, and the output
/// (setpoint minus measurement) written at Priority_For_Writing (16) to the
/// manipulated variable. Returns the output.
fn run_pass(db: &mut ObjectDatabase, lo: ObjectIdentifier) -> f32 {
    let measure = decode_object_property_reference(&read_bytes(db, lo, CVR)).unwrap();
    let measured = read_real(
        db,
        measure.object_identifier,
        PropertyIdentifier::from_raw(measure.property_identifier),
    );
    db.get_mut(&lo)
        .unwrap()
        .set_controlled_variable_value_internal(PropertyValue::Real(measured))
        .unwrap();
    let setpoint = match decode_setpoint_reference(&read_bytes(db, lo, SR)).unwrap() {
        Some(reference) => read_real(
            db,
            reference.object_identifier,
            PropertyIdentifier::from_raw(reference.property_identifier),
        ),
        None => read_real(db, lo, PropertyIdentifier::SETPOINT),
    };
    let output = setpoint - measured;
    db.get_mut(&lo)
        .unwrap()
        .set_present_value_internal(PropertyValue::Real(output))
        .unwrap();
    let manipulate = decode_object_property_reference(&read_bytes(db, lo, MVR)).unwrap();
    write(
        db,
        manipulate.object_identifier,
        PropertyIdentifier::from_raw(manipulate.property_identifier),
        real(output),
        Some(16),
    );
    output
}

#[test]
fn loop_follows_the_references_a_client_writes() {
    let mut db = ObjectDatabase::new();
    let lo = oid(ObjectType::LOOP, 1);
    db.add(Box::new(
        LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    for (instance, value) in [(7, 20.0), (8, 30.0), (10, 22.5)] {
        let mut ai = AnalogInputObject::new(
            instance,
            format!("AI-{instance}"),
            EngineeringUnits::DEGREES_CELSIUS,
        )
        .unwrap();
        ai.set_present_value(value);
        db.add(Box::new(ai)).unwrap();
    }
    for instance in [3, 4] {
        db.add(Box::new(
            AnalogOutputObject::new(
                instance,
                format!("AO-{instance}"),
                EngineeringUnits::DEGREES_CELSIUS,
            )
            .unwrap(),
        ))
        .unwrap();
    }
    let ai_7 = oid(ObjectType::ANALOG_INPUT, 7);
    let ai_8 = oid(ObjectType::ANALOG_INPUT, 8);
    let ai_10 = oid(ObjectType::ANALOG_INPUT, 10);
    let ao_3 = oid(ObjectType::ANALOG_OUTPUT, 3);
    let ao_4 = oid(ObjectType::ANALOG_OUTPUT, 4);

    // A client points the loop at AI-7, AI-10 for the setpoint, and AO-3.
    write(&mut db, lo, CVR, reference_bytes(ai_7), None);
    write(
        &mut db,
        lo,
        SR,
        [&[0x0E][..], &reference_bytes(ai_10), &[0x0F]].concat(),
        None,
    );
    write(&mut db, lo, MVR, reference_bytes(ao_3), None);
    assert_eq!(run_pass(&mut db, lo), 2.5);
    assert_eq!(
        read_real(&db, lo, PropertyIdentifier::CONTROLLED_VARIABLE_VALUE),
        20.0
    );
    assert_eq!(read_real(&db, lo, PV), 2.5);
    assert_eq!(read_real(&db, ao_3, PV), 2.5);

    // It re-points them: AI-8, no setpoint reference (so the loop's own
    // Setpoint), and AO-4. The next pass follows, and AO-3 keeps its value.
    write(&mut db, lo, CVR, reference_bytes(ai_8), None);
    write(&mut db, lo, SR, Vec::new(), None);
    write(&mut db, lo, PropertyIdentifier::SETPOINT, real(31.0), None);
    write(&mut db, lo, MVR, reference_bytes(ao_4), None);
    assert_eq!(run_pass(&mut db, lo), 1.0);
    assert_eq!(
        read_real(&db, lo, PropertyIdentifier::CONTROLLED_VARIABLE_VALUE),
        30.0
    );
    assert_eq!(read_real(&db, ao_4, PV), 1.0);
    assert_eq!(read_real(&db, ao_3, PV), 2.5);
}
