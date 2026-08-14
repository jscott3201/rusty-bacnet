use super::super::*;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::event_enrollment::{
    EventEnrollmentEvalState, EventEnrollmentObject, EventEnrollmentPending,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};

fn add_out_of_range_enrollment(
    db: &mut ObjectDatabase,
    instance: u32,
    monitored_oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: u32,
    time_delay: u32,
) -> ObjectIdentifier {
    let mut enrollment = EventEnrollmentObject::new(
        instance,
        format!("EE-index-{instance}"),
        EventType::OUT_OF_RANGE.to_raw(),
    )
    .unwrap();
    enrollment.set_object_property_reference(Some(
        BACnetDeviceObjectPropertyReference::new_local(monitored_oid, property.to_raw())
            .with_index(index),
    ));
    enrollment.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 1.0,
    });
    enrollment.set_event_enable(0x07);
    let oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();
    oid
}

#[test]
fn indexed_priority_array_element_drives_evaluation() {
    let mut db = ObjectDatabase::new();
    let mut value = AnalogValueObject::new(1, "AV-indexed", 62).unwrap();
    value
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(1),
        )
        .unwrap();
    let value_oid = value.object_identifier();
    db.add(Box::new(value)).unwrap();
    add_out_of_range_enrollment(
        &mut db,
        1,
        value_oid,
        PropertyIdentifier::PRIORITY_ARRAY,
        1,
        0,
    );

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].monitored_oid, value_oid);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
}

#[test]
fn array_index_zero_monitors_the_element_count() {
    let mut db = ObjectDatabase::new();
    let value = AnalogValueObject::new(2, "AV-count", 62).unwrap();
    let value_oid = value.object_identifier();
    db.add(Box::new(value)).unwrap();
    add_out_of_range_enrollment(
        &mut db,
        2,
        value_oid,
        PropertyIdentifier::PRIORITY_ARRAY,
        0,
        0,
    );

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::LOW_LIMIT);
}

#[test]
fn out_of_range_index_clears_state_without_whole_property_fallback() {
    let mut db = ObjectDatabase::new();
    let value = AnalogValueObject::new(3, "AV-invalid-index", 62).unwrap();
    let value_oid = value.object_identifier();
    db.add(Box::new(value)).unwrap();
    let enrollment_oid = add_out_of_range_enrollment(
        &mut db,
        3,
        value_oid,
        PropertyIdentifier::PRIORITY_ARRAY,
        17,
        1,
    );
    let stale = EventEnrollmentEvalState {
        pending: Some(EventEnrollmentPending {
            state: EventState::HIGH_LIMIT,
            remaining: 1,
            condition: 0,
            params_fingerprint: 1,
        }),
        cov_baseline: Some(PropertyValue::Real(90.0)),
        last_offnormal_value: Some(1),
    };
    db.get_mut(&enrollment_oid)
        .unwrap()
        .set_enrollment_eval_state_internal(stale)
        .unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .enrollment_eval_state_internal(),
        Some(EventEnrollmentEvalState::default())
    );
}

#[test]
fn index_on_scalar_property_does_not_read_the_scalar() {
    let mut db = ObjectDatabase::new();
    let mut value = AnalogValueObject::new(4, "AV-scalar", 62).unwrap();
    value.set_present_value(90.0);
    let value_oid = value.object_identifier();
    db.add(Box::new(value)).unwrap();
    add_out_of_range_enrollment(
        &mut db,
        4,
        value_oid,
        PropertyIdentifier::PRESENT_VALUE,
        1,
        0,
    );

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
}

#[test]
fn index_change_restarts_the_pending_delay() {
    let mut db = ObjectDatabase::new();
    let mut value = AnalogValueObject::new(5, "AV-retarget", 62).unwrap();
    for priority in [1, 2] {
        value
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(90.0),
                Some(priority),
            )
            .unwrap();
    }
    let value_oid = value.object_identifier();
    db.add(Box::new(value)).unwrap();
    let enrollment_oid = add_out_of_range_enrollment(
        &mut db,
        5,
        value_oid,
        PropertyIdentifier::PRIORITY_ARRAY,
        1,
        2,
    );

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    let pending = db
        .get(&enrollment_oid)
        .unwrap()
        .enrollment_eval_state_internal()
        .unwrap();
    assert_eq!(pending.pending.as_ref().unwrap().remaining, 2);

    assert!(db.remove(&enrollment_oid).is_some());
    let replacement_oid = add_out_of_range_enrollment(
        &mut db,
        5,
        value_oid,
        PropertyIdentifier::PRIORITY_ARRAY,
        2,
        2,
    );
    assert_eq!(replacement_oid, enrollment_oid);
    db.get_mut(&enrollment_oid)
        .unwrap()
        .set_enrollment_eval_state_internal(pending)
        .unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .enrollment_eval_state_internal()
            .unwrap()
            .pending
            .unwrap()
            .remaining,
        2
    );
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
}
