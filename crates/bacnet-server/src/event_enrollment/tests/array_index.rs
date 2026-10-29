use super::super::*;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::event_enrollment::{EventEnrollmentEvalState, EventEnrollmentObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, ChangeOfValueCriteria,
};
use bacnet_types::enums::{EngineeringUnits, ErrorClass, ErrorCode};
use bacnet_types::error::Error;
use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

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
        EventType::OUT_OF_RANGE,
    )
    .unwrap();
    enrollment
        .set_object_property_reference(Some(
            BACnetDeviceObjectPropertyReference::new_local(monitored_oid, property.to_raw())
                .with_index(index),
        ))
        .unwrap();
    enrollment.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 1.0,
    });
    enrollment.set_event_enable(EventTransitionBits::all());
    let oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();
    oid
}

fn add_cov_enrollment(
    db: &mut ObjectDatabase,
    instance: u32,
    monitored_oid: ObjectIdentifier,
    index: u32,
) -> ObjectIdentifier {
    let mut enrollment = EventEnrollmentObject::new(
        instance,
        format!("EE-COV-index-{instance}"),
        EventType::CHANGE_OF_VALUE,
    )
    .unwrap();
    enrollment
        .set_object_property_reference(Some(
            BACnetDeviceObjectPropertyReference::new_local(
                monitored_oid,
                PropertyIdentifier::PRIORITY_ARRAY.to_raw(),
            )
            .with_index(index),
        ))
        .unwrap();
    enrollment.set_event_parameters(BACnetEventParameter::ChangeOfValue {
        time_delay: 0,
        criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(5.0),
    });
    enrollment.set_event_enable(EventTransitionBits::all());
    let oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();
    oid
}

struct IndexedReadProbe {
    inner: AnalogValueObject,
    control: ProbeControl,
}

#[derive(Clone)]
struct ProbeControl {
    reads: Arc<Mutex<Vec<Option<u32>>>>,
    transient_failure: Arc<AtomicBool>,
}

impl IndexedReadProbe {
    fn new(instance: u32) -> (Self, ProbeControl) {
        let control = ProbeControl {
            reads: Arc::new(Mutex::new(Vec::new())),
            transient_failure: Arc::new(AtomicBool::new(false)),
        };
        (
            Self {
                inner: AnalogValueObject::new(
                    instance,
                    format!("AV-probe-{instance}"),
                    EngineeringUnits::DEGREES_CELSIUS,
                )
                .unwrap(),
                control: control.clone(),
            },
            control,
        )
    }
}

impl BACnetObject for IndexedReadProbe {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.inner.object_identifier()
    }

    fn object_name(&self) -> &str {
        self.inner.object_name()
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if property != PropertyIdentifier::PRIORITY_ARRAY {
            return self.inner.read_property(property, array_index);
        }
        self.control.reads.lock().unwrap().push(array_index);
        if self.control.transient_failure.load(Ordering::SeqCst) {
            return Err(Error::Encoding("transient indexed read".into()));
        }
        if array_index == Some(17) {
            return Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::INVALID_ARRAY_INDEX,
            ));
        }
        Ok(PropertyValue::Real(90.0))
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.inner.write_property_from(
            property,
            array_index,
            value,
            priority,
            &crate::command_source::test_origin(),
        )
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.inner.property_list()
    }

    fn is_array_property(&self, property: PropertyIdentifier) -> bool {
        self.inner.is_array_property(property)
    }
}

#[test]
fn indexed_priority_array_element_drives_evaluation() {
    let mut db = ObjectDatabase::new();
    let mut value =
        AnalogValueObject::new(1, "AV-indexed", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    value
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(1),
            &crate::command_source::test_origin(),
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
    let value = AnalogValueObject::new(2, "AV-count", EngineeringUnits::DEGREES_CELSIUS).unwrap();
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
    let (value, control) = IndexedReadProbe::new(3);
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
        pending: None,
        cov_baseline: Some(PropertyValue::Real(90.0)),
        last_offnormal_value: Some(1),
    };
    db.get_mut(&enrollment_oid)
        .unwrap()
        .set_enrollment_eval_state_internal(stale)
        .unwrap();
    db.get_mut(&enrollment_oid)
        .unwrap()
        .set_enrollment_eval_source_internal(Some((
            value_oid,
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(17),
        )))
        .unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(*control.reads.lock().unwrap(), vec![Some(17)]);
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .enrollment_eval_state_internal(),
        Some(EventEnrollmentEvalState::default())
    );
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .enrollment_eval_source_internal(),
        Some(None)
    );
}

#[test]
fn index_on_scalar_property_does_not_read_the_scalar() {
    let mut db = ObjectDatabase::new();
    let mut value =
        AnalogValueObject::new(4, "AV-scalar", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    value.set_relinquish_default(90.0).unwrap();
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
    let mut value =
        AnalogValueObject::new(5, "AV-retarget", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    for priority in [1, 2] {
        value
            .write_property_from(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(90.0),
                Some(priority),
                &crate::command_source::test_origin(),
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

    assert!(db.remove(&enrollment_oid).unwrap().is_some());
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

#[test]
fn index_change_discards_the_previous_element_baselines() {
    let mut db = ObjectDatabase::new();
    let mut value =
        AnalogValueObject::new(6, "AV-COV-retarget", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    for (priority, sample) in [(1, 10.0), (2, 90.0)] {
        value
            .write_property_from(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(sample),
                Some(priority),
                &crate::command_source::test_origin(),
            )
            .unwrap();
    }
    let value_oid = value.object_identifier();
    db.add(Box::new(value)).unwrap();
    let enrollment_oid = add_cov_enrollment(&mut db, 6, value_oid, 1);

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    let mut prior_state = db
        .get(&enrollment_oid)
        .unwrap()
        .enrollment_eval_state_internal()
        .unwrap();
    assert_eq!(prior_state.cov_baseline, Some(PropertyValue::Real(10.0)));
    prior_state.last_offnormal_value = Some(7);

    assert!(db.remove(&enrollment_oid).unwrap().is_some());
    assert_eq!(add_cov_enrollment(&mut db, 6, value_oid, 2), enrollment_oid);
    db.get_mut(&enrollment_oid)
        .unwrap()
        .set_enrollment_eval_state_internal(prior_state)
        .unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    let state = db
        .get(&enrollment_oid)
        .unwrap()
        .enrollment_eval_state_internal()
        .unwrap();
    assert_eq!(state.cov_baseline, Some(PropertyValue::Real(90.0)));
    assert_eq!(state.last_offnormal_value, None);
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .enrollment_eval_source_internal(),
        Some(Some((
            value_oid,
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(2)
        )))
    );
}

#[test]
fn null_indexed_element_interrupts_the_pending_delay() {
    let mut db = ObjectDatabase::new();
    let mut value =
        AnalogValueObject::new(7, "AV-null", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    value
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(1),
            &crate::command_source::test_origin(),
        )
        .unwrap();
    let value_oid = value.object_identifier();
    db.add(Box::new(value)).unwrap();
    let enrollment_oid = add_out_of_range_enrollment(
        &mut db,
        7,
        value_oid,
        PropertyIdentifier::PRIORITY_ARRAY,
        1,
        2,
    );

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    db.get_mut(&value_oid)
        .unwrap()
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Null,
            Some(1),
            &crate::command_source::test_origin(),
        )
        .unwrap();
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(
        db.get(&enrollment_oid)
            .unwrap()
            .enrollment_eval_state_internal()
            .unwrap()
            .pending
            .is_none()
    );

    db.get_mut(&value_oid)
        .unwrap()
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(90.0),
            Some(1),
            &crate::command_source::test_origin(),
        )
        .unwrap();
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
}

#[test]
fn transient_indexed_read_failure_clears_private_continuity() {
    let mut db = ObjectDatabase::new();
    let (value, control) = IndexedReadProbe::new(8);
    let value_oid = value.object_identifier();
    db.add(Box::new(value)).unwrap();
    let enrollment_oid = add_out_of_range_enrollment(
        &mut db,
        8,
        value_oid,
        PropertyIdentifier::PRIORITY_ARRAY,
        1,
        2,
    );

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    let mut stale = db
        .get(&enrollment_oid)
        .unwrap()
        .enrollment_eval_state_internal()
        .unwrap();
    stale.cov_baseline = Some(PropertyValue::Real(42.0));
    stale.last_offnormal_value = Some(3);
    db.get_mut(&enrollment_oid)
        .unwrap()
        .set_enrollment_eval_state_internal(stale)
        .unwrap();
    control.reads.lock().unwrap().clear();
    control.transient_failure.store(true, Ordering::SeqCst);

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(*control.reads.lock().unwrap(), vec![Some(1)]);
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .enrollment_eval_state_internal(),
        Some(EventEnrollmentEvalState::default())
    );
    assert_eq!(
        db.get(&enrollment_oid)
            .unwrap()
            .enrollment_eval_source_internal(),
        Some(None)
    );
}

#[test]
fn null_indexed_floating_setpoint_interrupts_the_pending_delay() {
    let mut db = ObjectDatabase::new();
    let mut monitored = AnalogValueObject::new(
        9,
        "AV-floating-monitored",
        EngineeringUnits::DEGREES_CELSIUS,
    )
    .unwrap();
    monitored.set_relinquish_default(90.0).unwrap();
    let monitored_oid = monitored.object_identifier();
    db.add(Box::new(monitored)).unwrap();

    let mut setpoint = AnalogValueObject::new(
        10,
        "AV-floating-setpoint",
        EngineeringUnits::DEGREES_CELSIUS,
    )
    .unwrap();
    setpoint
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(50.0),
            Some(1),
            &crate::command_source::test_origin(),
        )
        .unwrap();
    let setpoint_oid = setpoint.object_identifier();
    db.add(Box::new(setpoint)).unwrap();

    let mut enrollment =
        EventEnrollmentObject::new(9, "EE-indexed-setpoint", EventType::FLOATING_LIMIT).unwrap();
    enrollment
        .set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            monitored_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )))
        .unwrap();
    enrollment.set_event_parameters(BACnetEventParameter::FloatingLimit {
        time_delay: 2,
        setpoint_reference: BACnetDeviceObjectPropertyReference::new_local(
            setpoint_oid,
            PropertyIdentifier::PRIORITY_ARRAY.to_raw(),
        )
        .with_index(1),
        low_diff_limit: 10.0,
        high_diff_limit: 10.0,
        deadband: 1.0,
    });
    enrollment.set_event_enable(EventTransitionBits::all());
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    db.get_mut(&setpoint_oid)
        .unwrap()
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Null,
            Some(1),
            &crate::command_source::test_origin(),
        )
        .unwrap();
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(
        db.get(&enrollment_oid)
            .unwrap()
            .enrollment_eval_state_internal()
            .unwrap()
            .pending
            .is_none()
    );

    db.get_mut(&setpoint_oid)
        .unwrap()
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(50.0),
            Some(1),
            &crate::command_source::test_origin(),
        )
        .unwrap();
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());
    assert_eq!(evaluate_event_enrollments(&mut db, 1).len(), 1);
}
