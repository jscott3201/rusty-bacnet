//! Event_Algorithm_Inhibit follows the property its reference names (#1329).

use bacnet_encoding::constructed::encode_object_property_reference;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier as P, Reliability};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

use super::ObjectDatabase;
use crate::analog::AnalogInputObject;
use crate::binary::{BinaryOutputObject, BinaryValueObject};

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn refer(db: &mut ObjectDatabase, from: ObjectIdentifier, to: ObjectIdentifier, property: P) {
    let mut octets = BytesMut::new();
    encode_object_property_reference(
        &mut octets,
        &BACnetObjectPropertyReference::new(to, property.to_raw()),
    );
    db.get_mut(&from)
        .unwrap()
        .write_property(
            P::EVENT_ALGORITHM_INHIBIT_REF,
            None,
            PropertyValue::ApplicationData(octets.to_vec()),
            None,
        )
        .unwrap();
}

fn inhibited(db: &ObjectDatabase, oid: ObjectIdentifier) -> bool {
    db.get(&oid)
        .unwrap()
        .read_property(P::EVENT_ALGORITHM_INHIBIT, None)
        .unwrap()
        == PropertyValue::Boolean(true)
}

fn set_binary(db: &mut ObjectDatabase, oid: ObjectIdentifier, active: bool) {
    db.get_mut(&oid)
        .unwrap()
        .write_property_from(
            P::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(u32::from(active)),
            Some(16),
            &crate::command_source::test_origin(),
        )
        .unwrap();
}

#[test]
fn the_inhibit_follows_a_binary_pv_or_a_boolean() {
    let mut db = ObjectDatabase::new();
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let switch = oid(ObjectType::BINARY_VALUE, 1);
    let other = oid(ObjectType::ANALOG_INPUT, 2);
    db.add(Box::new(
        AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogInputObject::new(2, "AI-2", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(BinaryValueObject::new(1, "BV-1").unwrap()))
        .unwrap();

    // Without a reference there is nothing to follow.
    assert!(!db.follow_event_algorithm_inhibit(&ai));
    refer(&mut db, ai, switch, P::PRESENT_VALUE);
    assert!(!db.follow_event_algorithm_inhibit(&ai), "INACTIVE is FALSE");
    set_binary(&mut db, switch, true);
    assert!(db.follow_event_algorithm_inhibit(&ai), "ACTIVE is TRUE");
    assert!(inhibited(&db, ai));
    assert!(!db.follow_event_algorithm_inhibit(&ai), "no change");
    set_binary(&mut db, switch, false);
    assert!(db.follow_event_algorithm_inhibit(&ai));
    assert!(!inhibited(&db, ai));

    // A Boolean: another input's Out_Of_Service.
    refer(&mut db, ai, other, P::OUT_OF_SERVICE);
    db.get_mut(&other)
        .unwrap()
        .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
        .unwrap();
    assert!(db.follow_event_algorithm_inhibit(&ai));
    assert!(inhibited(&db, ai));
}

#[test]
fn a_missing_or_unusable_property_reads_as_false() {
    let mut db = ObjectDatabase::new();
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let switch = oid(ObjectType::BINARY_VALUE, 1);
    db.add(Box::new(
        AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(BinaryValueObject::new(1, "BV-1").unwrap()))
        .unwrap();
    for (target, property) in [
        // An object this device doesn't hold.
        (oid(ObjectType::BINARY_VALUE, 9), P::PRESENT_VALUE),
        // A property the object doesn't have.
        (switch, P::HIGH_LIMIT),
        // A property of another datatype: its own REAL Present_Value.
        (ai, P::PRESENT_VALUE),
    ] {
        set_binary(&mut db, switch, true);
        refer(&mut db, ai, switch, P::PRESENT_VALUE);
        db.follow_event_algorithm_inhibit(&ai);
        assert!(inhibited(&db, ai));
        refer(&mut db, ai, target, property);
        assert!(
            db.follow_event_algorithm_inhibit(&ai),
            "{target:?} {property:?}"
        );
        assert!(!inhibited(&db, ai), "{target:?} {property:?}");
    }
    // An object that reports nothing has nothing to follow.
    assert!(!db.follow_event_algorithm_inhibit(&oid(ObjectType::DEVICE, 1)));
}

#[test]
fn an_enumerated_one_inhibits_only_as_a_binary_pv_active() {
    let mut db = ObjectDatabase::new();
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let faulted = oid(ObjectType::ANALOG_INPUT, 2);
    let switch = oid(ObjectType::BINARY_VALUE, 1);
    let output = oid(ObjectType::BINARY_OUTPUT, 1);
    db.add(Box::new(
        AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogInputObject::new(2, "AI-2", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(BinaryValueObject::new(1, "BV-1").unwrap()))
        .unwrap();
    db.add(Box::new(BinaryOutputObject::new(1, "BO-1").unwrap()))
        .unwrap();
    set_binary(&mut db, switch, true);
    set_binary(&mut db, output, true);
    // Reliability NO_SENSOR and Event_State FAULT both read as Enumerated 1.
    let input = db.get_mut(&faulted).unwrap();
    input
        .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
        .unwrap();
    input
        .write_property(
            P::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
            None,
        )
        .unwrap();
    let fault = input.evaluate_intrinsic_reporting().expect("into FAULT");
    crate::event::commit_test_proposal(input, fault);
    for property in [P::RELIABILITY, P::EVENT_STATE] {
        assert_eq!(
            db.get(&faulted)
                .unwrap()
                .read_property(property, None)
                .unwrap(),
            PropertyValue::Enumerated(1),
            "{property:?}"
        );
    }

    for (target, property, index, expected) in [
        (switch, P::PRESENT_VALUE, None, true),
        (faulted, P::RELIABILITY, None, false),
        (switch, P::PRESENT_VALUE, None, true),
        (faulted, P::EVENT_STATE, None, false),
        // A commanded slot of a binary output is a BinaryPV too; the array
        // size is not.
        (output, P::PRIORITY_ARRAY, Some(16), true),
        (output, P::PRIORITY_ARRAY, Some(0), false),
        (output, P::PRESENT_VALUE, None, true),
    ] {
        let mut octets = BytesMut::new();
        let mut reference = BACnetObjectPropertyReference::new(target, property.to_raw());
        reference.property_array_index = index;
        encode_object_property_reference(&mut octets, &reference);
        db.get_mut(&ai)
            .unwrap()
            .write_property(
                P::EVENT_ALGORITHM_INHIBIT_REF,
                None,
                PropertyValue::ApplicationData(octets.to_vec()),
                None,
            )
            .unwrap();
        db.follow_event_algorithm_inhibit(&ai);
        assert_eq!(
            inhibited(&db, ai),
            expected,
            "{target:?} {property:?} {index:?}"
        );
    }
}
