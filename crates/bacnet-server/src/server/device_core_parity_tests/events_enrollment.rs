//! Event Enrollment (#2319): the server's pass on its task and the core
//! `Reporter`'s, at the same interval over the same fixture, given the same
//! writes at the same paused instants, send the same notifications, octet
//! for octet: each algorithm the pass evaluates, both ways, a
//! FAULT_OUT_OF_RANGE fault and its clearing, and a Time_Delay counted in
//! passes at two intervals.
//!
//! Both run their first pass when they start and one an interval after
//! each, so their passes fall together; each test steps through the
//! `Reporter`'s deadlines as the intrinsic tests do.
use super::*;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetPropertyStates,
    ChangeOfValueCriteria, FaultParameters,
};
use bacnet_types::enums::EventType;

fn av(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::ANALOG_VALUE, instance)
}

fn bv2() -> ObjectIdentifier {
    oid(ObjectType::BINARY_VALUE, 2)
}

/// Analog Value `instance` at `value`, with no intrinsic reporting.
fn value(instance: u32, value: f32) -> Box<dyn BACnetObject> {
    let name = format!("AV-{instance}");
    let mut object = AnalogValueObject::new(instance, &name, EngineeringUnits::NO_UNITS).unwrap();
    object.set_relinquish_default(value).unwrap();
    Box::new(object)
}

/// Event Enrollment `instance` watching `property` of `monitored` with
/// `parameters`, every transition enabled, through Notification Class 1.
fn enrollment(
    instance: u32,
    (monitored, property): (ObjectIdentifier, P),
    (event_type, parameters): (EventType, BACnetEventParameter),
) -> EventEnrollmentObject {
    let name = format!("EE-{instance}");
    let mut ee = EventEnrollmentObject::new(instance, &name, event_type).unwrap();
    let reference = BACnetDeviceObjectPropertyReference::new_local(monitored, property.to_raw());
    ee.set_object_property_reference(Some(reference)).unwrap();
    ee.set_event_parameters(parameters);
    ee.set_event_enable(EventTransitionBits::all());
    ee.set_notification_class(1);
    ee
}

fn out_of_range(time_delay: u32) -> (EventType, BACnetEventParameter) {
    let parameters = BACnetEventParameter::OutOfRange {
        time_delay,
        low_limit: 0.0,
        high_limit: 80.0,
        deadband: 1.0,
    };
    (EventType::OUT_OF_RANGE, parameters)
}

/// Notification Class 1, its one destination open all day, its
/// TO_OFFNORMAL needing acknowledgement, and event priorities other than
/// 255, so the network priority is compared.
fn class() -> Box<dyn BACnetObject> {
    let mut class = NotificationClass::new(1, "NC-1").unwrap();
    class.ack_required = EventTransitionBits::TO_OFFNORMAL;
    class.priority = [100, 200, 255];
    class.add_destination(recipients().swap_remove(0)).unwrap();
    Box::new(class)
}

/// One enrollment per algorithm, each on a point of its own: OUT_OF_RANGE
/// on AV-2, FLOATING_LIMIT on AV-3 against AV-4's setpoint,
/// CHANGE_OF_STATE on BV-2, CHANGE_OF_BITSTRING on AV-5's Status_Flags
/// (OUT_OF_SERVICE), CHANGE_OF_VALUE on AV-6, and OUT_OF_RANGE with a
/// FAULT_OUT_OF_RANGE fault on AV-7.
fn algorithms(identity: &bacnet_objects::DeviceIdentity) -> ObjectDatabase {
    let floating = BACnetEventParameter::FloatingLimit {
        time_delay: 0,
        setpoint_reference: BACnetDeviceObjectPropertyReference::new_local(
            av(4),
            P::PRESENT_VALUE.to_raw(),
        ),
        low_diff_limit: 10.0,
        high_diff_limit: 10.0,
        deadband: 2.0,
    };
    let state = BACnetEventParameter::ChangeOfState {
        time_delay: 0,
        list_of_values: vec![BACnetPropertyStates::BinaryValue(1)],
    };
    let bitstring = BACnetEventParameter::ChangeOfBitstring {
        time_delay: 0,
        bitmask: (4, vec![0x10]),
        list_of_values: vec![(4, vec![0x10])],
    };
    let change = BACnetEventParameter::ChangeOfValue {
        time_delay: 0,
        criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(5.0),
    };
    let mut faulting = enrollment(7, (av(7), P::PRESENT_VALUE), out_of_range(0));
    faulting.set_fault_parameters(Some(FaultParameters::FaultOutOfRange {
        min_normal: -40.0,
        max_normal: 120.0,
    }));
    let enrollments = [
        enrollment(2, (av(2), P::PRESENT_VALUE), out_of_range(0)),
        enrollment(
            3,
            (av(3), P::PRESENT_VALUE),
            (EventType::FLOATING_LIMIT, floating),
        ),
        enrollment(
            4,
            (bv2(), P::PRESENT_VALUE),
            (EventType::CHANGE_OF_STATE, state),
        ),
        enrollment(
            5,
            (av(5), P::STATUS_FLAGS),
            (EventType::CHANGE_OF_BITSTRING, bitstring),
        ),
        enrollment(
            6,
            (av(6), P::PRESENT_VALUE),
            (EventType::CHANGE_OF_VALUE, change),
        ),
        faulting,
    ];
    let mut more: Vec<Box<dyn BACnetObject>> = vec![
        value(2, 20.0),
        value(3, 50.0),
        value(4, 50.0),
        Box::new(BinaryValueObject::new(2, "BV-2").unwrap()),
        value(5, 20.0),
        value(6, 20.0),
        value(7, 20.0),
        class(),
    ];
    more.extend(enrollments.map(|ee| Box::new(ee) as Box<dyn BACnetObject>));
    database_with(identity, more)
}

/// OUT_OF_RANGE on AV-2 after a Time_Delay of 5 s.
fn delayed_enrollment(identity: &bacnet_objects::DeviceIdentity) -> ObjectDatabase {
    let ee = enrollment(2, (av(2), P::PRESENT_VALUE), out_of_range(5));
    database_with(identity, vec![value(2, 20.0), class(), Box::new(ee)])
}

fn real(value: f32) -> PropertyValue {
    PropertyValue::Real(value)
}

/// Each algorithm both ways and a fault, a pass a second: each change is
/// written half a second before a pass, which reports it, and both send
/// the same notifications at every pass.
#[tokio::test(start_paused = true)]
async fn each_algorithm_both_ways_and_a_fault_match() {
    let mut both = EventParity::start_with(algorithms, true, Some(1)).await;
    let pv = P::PRESENT_VALUE;
    let steps: [(&str, ObjectIdentifier, P, PropertyValue); 12] = [
        ("OUT_OF_RANGE over", av(2), pv, real(90.0)),
        ("OUT_OF_RANGE back", av(2), pv, real(50.0)),
        ("FLOATING_LIMIT over", av(3), pv, real(70.0)),
        ("FLOATING_LIMIT back", av(3), pv, real(50.0)),
        (
            "CHANGE_OF_STATE to its alarm value",
            bv2(),
            pv,
            PropertyValue::Enumerated(1),
        ),
        (
            "CHANGE_OF_STATE back",
            bv2(),
            pv,
            PropertyValue::Enumerated(0),
        ),
        (
            "CHANGE_OF_BITSTRING out of service",
            av(5),
            P::OUT_OF_SERVICE,
            PropertyValue::Boolean(true),
        ),
        (
            "CHANGE_OF_BITSTRING back",
            av(5),
            P::OUT_OF_SERVICE,
            PropertyValue::Boolean(false),
        ),
        ("CHANGE_OF_VALUE by more than 5", av(6), pv, real(30.0)),
        ("CHANGE_OF_VALUE again", av(6), pv, real(40.0)),
        (
            "past FAULT_OUT_OF_RANGE's normal range",
            av(7),
            pv,
            real(130.0),
        ),
        ("back in range", av(7), pv, real(50.0)),
    ];
    let mut sent = Vec::new();
    for (step, (what, object, property, value)) in (1u64..).zip(steps) {
        let at = step * 2_000;
        // The passes before the write run first, as the server's do: a
        // pass reads the value as it stands when it runs.
        assert_eq!(both.run_to(what, at - 500).await, 0, "{what}: not before");
        let with_reply = both
            .write_to(what, at - 500, (object, property), value)
            .await;
        assert_eq!(with_reply, 0, "{what}: nothing with the reply");
        sent.push(both.run_to(what, at + 100).await);
    }
    assert_eq!(sent, [1; 12], "one transition each, at the pass after it");
}

/// A Time_Delay of 5 s counts passes, rounded up: at a pass a second it
/// fires five passes after the one that saw the change, at a pass every
/// ten seconds one pass after.
#[tokio::test(start_paused = true)]
async fn the_delay_counts_passes_at_two_intervals() {
    for (interval, before, fires) in [(1, 5_900, 6_100), (10, 19_900, 20_100)] {
        let mut both = EventParity::start_with(delayed_enrollment, true, Some(interval)).await;
        let what = format!("over the limit, a pass every {interval} s");
        let with_reply = both
            .write_to(&what, 500, (av(2), P::PRESENT_VALUE), real(90.0))
            .await;
        assert_eq!(with_reply, 0);
        assert_eq!(both.run_to(&what, before).await, 0, "{what}: not yet");
        assert_eq!(both.run_to(&what, fires).await, 1, "{what}: fired");
    }
}
