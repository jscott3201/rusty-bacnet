//! Per algorithm, two enrollments that transition in the same pass: the
//! commits, and so the notifications, come out in database order (the
//! order the enrollments were added, not their identifiers'), each with its
//! own enrollment's monitored object, states and values. The pass runs
//! each algorithm in a function of its own (#2558); these pin that the
//! split kept both the order and the pairing.

use super::super::*;
use super::*;
use bacnet_services::alarm_event::NotificationParameters;
use bacnet_types::constructed::ChangeOfValueCriteria;
use bacnet_types::primitives::StatusFlags;

const PRESENT_VALUE: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;

/// Add Event Enrollment `instance` of `event_type`, watching `property` of
/// `monitored` with `params`, every transition enabled.
fn enroll(
    db: &mut ObjectDatabase,
    instance: u32,
    (monitored, property): (ObjectIdentifier, PropertyIdentifier),
    event_type: EventType,
    params: BACnetEventParameter,
) -> ObjectIdentifier {
    let mut ee =
        EventEnrollmentObject::new(instance, format!("EE-{instance}"), event_type).unwrap();
    let reference = BACnetDeviceObjectPropertyReference::new_local(monitored, property.to_raw());
    ee.set_object_property_reference(Some(reference)).unwrap();
    ee.set_event_parameters(params);
    ee.set_event_enable(EventTransitionBits::all());
    let oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();
    oid
}

fn analog(db: &mut ObjectDatabase, instance: u32, value: f32) -> ObjectIdentifier {
    let mut ai = AnalogInputObject::new(
        instance,
        format!("AI-{instance}"),
        EngineeringUnits::DEGREES_CELSIUS,
    )
    .unwrap();
    ai.set_present_value(value);
    let oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();
    oid
}

fn binary(db: &mut ObjectDatabase, instance: u32, value: u32) -> ObjectIdentifier {
    let mut bi = BinaryInputObject::new(instance, format!("BI-{instance}")).unwrap();
    bi.set_present_value(value);
    let oid = bi.object_identifier();
    db.add(Box::new(bi)).unwrap();
    oid
}

type Committed = (ObjectIdentifier, ObjectIdentifier, EventState, EventState);

/// The event-state transitions a pass committed, in its order, as
/// (enrollment, monitored object, from, to), with their notifications'
/// values: the report's list and the deliveries must agree.
fn committed(
    batch: &EventEnrollmentEvaluationBatch,
) -> (Vec<Committed>, Vec<NotificationParameters>) {
    let reported: Vec<Committed> = batch
        .report
        .transitions
        .iter()
        .map(|t| {
            (
                t.enrollment_oid,
                t.monitored_oid,
                t.change.from,
                t.change.to,
            )
        })
        .collect();
    let (mut delivered, mut values) = (Vec::new(), Vec::new());
    for delivery in &batch.deliveries {
        if let CommittedEventEnrollmentResult::Normal(t) = &delivery.result {
            delivered.push((
                t.enrollment_oid,
                t.monitored_oid,
                t.change.from,
                t.change.to,
            ));
            values.push(delivery.event_values.clone().into_parameters());
        }
    }
    assert_eq!(reported, delivered);
    (reported, values)
}

fn out_of_range() -> BACnetEventParameter {
    BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    }
}

/// Limits at the setpoint plus 15 and less 10: unequal, so a value between
/// the two readings of them tells the high and low limits apart.
fn floating_limit(setpoint: ObjectIdentifier) -> BACnetEventParameter {
    BACnetEventParameter::FloatingLimit {
        time_delay: 0,
        setpoint_reference: BACnetDeviceObjectPropertyReference::new_local(
            setpoint,
            PRESENT_VALUE.to_raw(),
        ),
        low_diff_limit: 10.0,
        high_diff_limit: 15.0,
        deadband: 2.0,
    }
}

fn change_of_state(alarm: u32) -> BACnetEventParameter {
    BACnetEventParameter::ChangeOfState {
        time_delay: 0,
        list_of_values: vec![BACnetPropertyStates::BinaryValue(alarm)],
    }
}

#[test]
fn out_of_range_commits_in_database_order() {
    let mut db = ObjectDatabase::new();
    let high = analog(&mut db, 1, 90.0);
    let low = analog(&mut db, 2, 10.0);
    let ee2 = enroll(
        &mut db,
        2,
        (high, PRESENT_VALUE),
        EventType::OUT_OF_RANGE,
        out_of_range(),
    );
    let ee1 = enroll(
        &mut db,
        1,
        (low, PRESENT_VALUE),
        EventType::OUT_OF_RANGE,
        out_of_range(),
    );

    let batch = evaluate_event_enrollments_for_delivery(&mut db, 1);
    let (transitions, values) = committed(&batch);
    assert_eq!(
        transitions,
        [
            (ee2, high, EventState::NORMAL, EventState::HIGH_LIMIT),
            (ee1, low, EventState::NORMAL, EventState::LOW_LIMIT),
        ]
    );
    assert_eq!(
        values,
        [
            NotificationParameters::OutOfRange {
                exceeding_value: 90.0,
                status_flags: StatusFlags::empty(),
                deadband: 2.0,
                exceeded_limit: 80.0,
            },
            NotificationParameters::OutOfRange {
                exceeding_value: 10.0,
                status_flags: StatusFlags::empty(),
                deadband: 2.0,
                exceeded_limit: 20.0,
            },
        ]
    );
}

#[test]
fn floating_limit_commits_in_database_order() {
    let mut db = ObjectDatabase::new();
    // Over 65 and under 40; with the limits swapped (60 and 35), 38 would
    // stay NORMAL.
    let high = analog(&mut db, 1, 70.0);
    let low = analog(&mut db, 2, 38.0);
    let setpoint = analog(&mut db, 3, 50.0);
    let params = floating_limit(setpoint);
    let ee2 = enroll(
        &mut db,
        2,
        (high, PRESENT_VALUE),
        EventType::FLOATING_LIMIT,
        params.clone(),
    );
    let ee1 = enroll(
        &mut db,
        1,
        (low, PRESENT_VALUE),
        EventType::FLOATING_LIMIT,
        params,
    );

    let batch = evaluate_event_enrollments_for_delivery(&mut db, 1);
    let (transitions, values) = committed(&batch);
    assert_eq!(
        transitions,
        [
            (ee2, high, EventState::NORMAL, EventState::HIGH_LIMIT),
            (ee1, low, EventState::NORMAL, EventState::LOW_LIMIT),
        ]
    );
    let floating = |reference_value, error_limit| NotificationParameters::FloatingLimit {
        reference_value,
        status_flags: StatusFlags::empty(),
        setpoint_value: 50.0,
        error_limit,
    };
    assert_eq!(values, [floating(70.0, 15.0), floating(38.0, 10.0)]);
}

#[test]
fn change_of_state_commits_in_database_order() {
    let mut db = ObjectDatabase::new();
    let active = binary(&mut db, 1, 1);
    let inactive = binary(&mut db, 2, 0);
    let state = EventType::CHANGE_OF_STATE;
    let ee2 = enroll(
        &mut db,
        2,
        (active, PRESENT_VALUE),
        state,
        change_of_state(1),
    );
    let ee1 = enroll(
        &mut db,
        1,
        (inactive, PRESENT_VALUE),
        state,
        change_of_state(0),
    );

    let batch = evaluate_event_enrollments_for_delivery(&mut db, 1);
    let (transitions, values) = committed(&batch);
    assert_eq!(
        transitions,
        [
            (ee2, active, EventState::NORMAL, EventState::OFFNORMAL),
            (ee1, inactive, EventState::NORMAL, EventState::OFFNORMAL),
        ]
    );
    let new_state = |value| NotificationParameters::ChangeOfState {
        new_state: BACnetPropertyStates::BinaryValue(value),
        status_flags: StatusFlags::empty(),
    };
    assert_eq!(values, [new_state(1), new_state(0)]);
}

/// The monitored bit strings are two other enrollments' Event_Enable: all
/// three bits (wire 0xE0), and to-offnormal with to-normal (0xA0).
#[test]
fn change_of_bitstring_commits_in_database_order() {
    let mut db = ObjectDatabase::new();
    let mut all = EventEnrollmentObject::new(50, "EE-50", EventType::NONE).unwrap();
    all.set_event_enable(EventTransitionBits::all());
    let mut two = EventEnrollmentObject::new(60, "EE-60", EventType::NONE).unwrap();
    two.set_event_enable(EventTransitionBits::TO_OFFNORMAL | EventTransitionBits::TO_NORMAL);
    let (all, two) = (all.object_identifier(), {
        let oid = two.object_identifier();
        db.add(Box::new(all)).unwrap();
        db.add(Box::new(two)).unwrap();
        oid
    });
    let bits = |alarm| BACnetEventParameter::ChangeOfBitstring {
        time_delay: 0,
        bitmask: (0, vec![0xE0]),
        list_of_values: vec![(0, vec![alarm])],
    };
    let enable = PropertyIdentifier::EVENT_ENABLE;
    let bitstring = EventType::CHANGE_OF_BITSTRING;
    let ee2 = enroll(&mut db, 2, (all, enable), bitstring, bits(0xE0));
    let ee1 = enroll(&mut db, 1, (two, enable), bitstring, bits(0xA0));

    let batch = evaluate_event_enrollments_for_delivery(&mut db, 1);
    let (transitions, values) = committed(&batch);
    assert_eq!(
        transitions,
        [
            (ee2, all, EventState::NORMAL, EventState::OFFNORMAL),
            (ee1, two, EventState::NORMAL, EventState::OFFNORMAL),
        ]
    );
    let referenced: Vec<_> = values
        .iter()
        .map(|values| match values {
            NotificationParameters::ChangeOfBitstring {
                referenced_bitstring,
                ..
            } => referenced_bitstring.1.clone(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(referenced, [vec![0xE0], vec![0xA0]]);
}

/// The first pass takes each baseline and indicates nothing; the second
/// sees both values move by the increment or more.
#[test]
fn change_of_value_commits_in_database_order() {
    let mut db = ObjectDatabase::new();
    let first = analog(&mut db, 1, 10.0);
    let second = analog(&mut db, 2, 20.0);
    let params = BACnetEventParameter::ChangeOfValue {
        time_delay: 0,
        criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(1.0),
    };
    let cov = EventType::CHANGE_OF_VALUE;
    let ee2 = enroll(&mut db, 2, (first, PRESENT_VALUE), cov, params.clone());
    let ee1 = enroll(&mut db, 1, (second, PRESENT_VALUE), cov, params);
    let baseline = evaluate_event_enrollments_for_delivery(&mut db, 1);
    assert!(committed(&baseline).0.is_empty());

    for (oid, value) in [(first, 15.0), (second, 30.0)] {
        let ai = db.get_mut(&oid).unwrap();
        let out_of_service = PropertyValue::Boolean(true);
        ai.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            out_of_service,
            None,
        )
        .unwrap();
        ai.write_property(PRESENT_VALUE, None, PropertyValue::Real(value), None)
            .unwrap();
    }
    let batch = evaluate_event_enrollments_for_delivery(&mut db, 1);
    let (transitions, values) = committed(&batch);
    assert_eq!(
        transitions,
        [
            (ee2, first, EventState::NORMAL, EventState::NORMAL),
            (ee1, second, EventState::NORMAL, EventState::NORMAL),
        ]
    );
    let changed: Vec<_> = values
        .iter()
        .map(|values| match values {
            NotificationParameters::ChangeOfValue { new_value, .. } => new_value.clone(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        changed,
        [
            bacnet_types::constructed::ChangeOfValueChoice::ChangedValue(15.0),
            bacnet_types::constructed::ChangeOfValueChoice::ChangedValue(30.0),
        ]
    );
}

/// Different algorithms in one pass keep database order across them too.
#[test]
fn mixed_algorithms_commit_in_database_order() {
    let mut db = ObjectDatabase::new();
    let high = analog(&mut db, 1, 70.0);
    let low = analog(&mut db, 2, 10.0);
    let setpoint = analog(&mut db, 3, 50.0);
    let active = binary(&mut db, 1, 1);
    let ee3 = enroll(
        &mut db,
        3,
        (high, PRESENT_VALUE),
        EventType::FLOATING_LIMIT,
        floating_limit(setpoint),
    );
    let ee1 = enroll(
        &mut db,
        1,
        (active, PRESENT_VALUE),
        EventType::CHANGE_OF_STATE,
        change_of_state(1),
    );
    let ee2 = enroll(
        &mut db,
        2,
        (low, PRESENT_VALUE),
        EventType::OUT_OF_RANGE,
        out_of_range(),
    );

    let batch = evaluate_event_enrollments_for_delivery(&mut db, 1);
    assert_eq!(
        committed(&batch).0,
        [
            (ee3, high, EventState::NORMAL, EventState::HIGH_LIMIT),
            (ee1, active, EventState::NORMAL, EventState::OFFNORMAL),
            (ee2, low, EventState::NORMAL, EventState::LOW_LIMIT),
        ]
    );
    let types: Vec<_> = batch.deliveries.iter().map(|d| d.event_type).collect();
    assert_eq!(
        types,
        [
            EventType::FLOATING_LIMIT,
            EventType::CHANGE_OF_STATE,
            EventType::OUT_OF_RANGE,
        ]
    );
}
