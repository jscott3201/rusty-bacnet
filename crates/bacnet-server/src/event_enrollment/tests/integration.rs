//! Multiple-enrollment integration tests.
//!
//! Split out of `tests.rs` to keep every file under the 700-LOC cap.

use super::super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};
use std::borrow::Cow;

// ---- Integration: multiple enrollments ----

#[test]
fn evaluates_multiple_enrollments() {
    let mut db = ObjectDatabase::new();

    // Two analog inputs
    let mut ai1 = AnalogInputObject::new(80, "AI-80", 62).unwrap();
    ai1.set_present_value(90.0); // will trigger HIGH_LIMIT
    let ai1_oid = ai1.object_identifier();
    db.add(Box::new(ai1)).unwrap();

    let mut ai2 = AnalogInputObject::new(81, "AI-81", 62).unwrap();
    ai2.set_present_value(50.0); // normal
    let ai2_oid = ai2.object_identifier();
    db.add(Box::new(ai2)).unwrap();

    // Two enrollments
    let mut ee1 =
        EventEnrollmentObject::new(80, "EE-80", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee1.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai1_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee1.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee1.set_event_enable(0x07);
    db.add(Box::new(ee1)).unwrap();

    let mut ee2 =
        EventEnrollmentObject::new(81, "EE-81", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee2.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai2_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee2.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee2.set_event_enable(0x07);
    db.add(Box::new(ee2)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db, 1);
    // Only AI-80 triggers (90 > 80)
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].monitored_oid, ai1_oid);
}

#[test]
fn missing_monitored_object_is_skipped() {
    let mut db = ObjectDatabase::new();

    let fake_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 999).unwrap();
    let mut ee =
        EventEnrollmentObject::new(90, "EE-miss", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        fake_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee.set_event_enable(0x07);
    db.add(Box::new(ee)).unwrap();

    // Should not panic or return transitions
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert!(transitions.is_empty());
}

fn setup_qualified_reference(
    local_device_instances: &[u32],
    reference_device_instance: u32,
) -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    for instance in local_device_instances {
        let device = DeviceObject::new(DeviceConfig {
            instance: *instance,
            name: format!("Device-{instance}"),
            ..DeviceConfig::default()
        })
        .unwrap();
        db.add(Box::new(device)).unwrap();
    }

    let mut ai = AnalogInputObject::new(3, "AI-3", 62).unwrap();
    ai.set_present_value(90.0);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let reference_device_oid =
        ObjectIdentifier::new(ObjectType::DEVICE, reference_device_instance).unwrap();
    let mut ee = EventEnrollmentObject::new(3, "EE-3", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_remote(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
        reference_device_oid,
    )));
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee.set_event_enable(0x07);
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    (db, ee_oid, ai_oid)
}

#[test]
fn self_qualified_reference_evaluates_local_object() {
    let (mut db, _ee_oid, ai_oid) = setup_qualified_reference(&[100], 100);

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].monitored_oid, ai_oid);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
}

#[test]
fn foreign_reference_does_not_evaluate_same_numbered_local_object() {
    let (mut db, ee_oid, _ai_oid) = setup_qualified_reference(&[100], 200);

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(
        db.get(&ee_oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
    let PropertyValue::List(reference) = db
        .get(&ee_oid)
        .unwrap()
        .read_property(PropertyIdentifier::OBJECT_PROPERTY_REFERENCE, None)
        .unwrap()
    else {
        panic!("expected object property reference list");
    };
    assert_eq!(
        reference[3],
        PropertyValue::ObjectIdentifier(ObjectIdentifier::new(ObjectType::DEVICE, 200).unwrap())
    );
}

#[test]
fn qualified_reference_requires_one_containing_device() {
    let (mut missing, _, _) = setup_qualified_reference(&[], 100);
    assert!(evaluate_event_enrollments(&mut missing, 1).is_empty());

    let (mut ambiguous, _, _) = setup_qualified_reference(&[100, 200], 100);
    assert!(evaluate_event_enrollments(&mut ambiguous, 1).is_empty());

    let (mut wildcard, _, _) = setup_qualified_reference(
        &[ObjectIdentifier::WILDCARD_INSTANCE],
        ObjectIdentifier::WILDCARD_INSTANCE,
    );
    assert!(evaluate_event_enrollments(&mut wildcard, 1).is_empty());
}

#[test]
fn unresolvable_reference_clears_private_evaluator_state() {
    let (mut db, ee_oid, _) = setup_qualified_reference(&[100], 200);
    db.get_mut(&ee_oid)
        .unwrap()
        .set_enrollment_eval_state_internal(
            bacnet_objects::event_enrollment::EventEnrollmentEvalState {
                pending: Some(bacnet_objects::event_enrollment::EventEnrollmentPending {
                    state: EventState::HIGH_LIMIT,
                    remaining: 1,
                    condition: 0,
                    params_fingerprint: 1,
                }),
                cov_baseline: Some(PropertyValue::Real(90.0)),
                last_offnormal_value: Some(1),
            },
        )
        .unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(
        db.get(&ee_oid)
            .unwrap()
            .enrollment_eval_state_internal()
            .unwrap(),
        bacnet_objects::event_enrollment::EventEnrollmentEvalState::default()
    );
}

struct ReferenceValueObject(PropertyValue);

impl BACnetObject for ReferenceValueObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 999).unwrap()
    }

    fn object_name(&self) -> &str {
        "reference-value"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, bacnet_types::error::Error> {
        if property == PropertyIdentifier::OBJECT_PROPERTY_REFERENCE {
            Ok(self.0.clone())
        } else {
            Err(bacnet_types::error::Error::Encoding(
                "unsupported test property".into(),
            ))
        }
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), bacnet_types::error::Error> {
        Err(bacnet_types::error::Error::Encoding(
            "test object is read-only".into(),
        ))
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[])
    }
}

#[test]
fn malformed_reference_shapes_do_not_become_local() {
    let target = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
    let foreign_device = ObjectIdentifier::new(ObjectType::DEVICE, 200).unwrap();
    let property = PropertyIdentifier::PRESENT_VALUE;
    let malformed = [
        vec![
            PropertyValue::ObjectIdentifier(target),
            PropertyValue::Unsigned(property.to_raw() as u64),
            PropertyValue::ObjectIdentifier(foreign_device),
        ],
        vec![
            PropertyValue::ObjectIdentifier(target),
            PropertyValue::Unsigned(property.to_raw() as u64),
            PropertyValue::Null,
            PropertyValue::Null,
            PropertyValue::ObjectIdentifier(foreign_device),
        ],
        vec![
            PropertyValue::ObjectIdentifier(target),
            PropertyValue::Unsigned(property.to_raw() as u64),
            PropertyValue::Boolean(false),
            PropertyValue::Null,
        ],
    ];

    for items in malformed {
        let enrollment = ReferenceValueObject(PropertyValue::List(items));
        assert!(super::super::read_object_property_ref(&enrollment).is_none());
    }

    let legacy = ReferenceValueObject(PropertyValue::List(vec![
        PropertyValue::ObjectIdentifier(target),
        PropertyValue::Unsigned(property.to_raw() as u64),
    ]));
    assert_eq!(
        super::super::read_object_property_ref(&legacy),
        Some((target, property, None))
    );
}
