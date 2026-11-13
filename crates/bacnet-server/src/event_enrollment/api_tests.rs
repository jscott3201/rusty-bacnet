//! The wrappers' `u64` interval goes into the core as a `u32` (#2319 PR
//! 2b): an interval above `u32::MAX` saturates, which gives every delay the
//! same passes as before.

use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};
use bacnet_types::enums::{EngineeringUnits, EventState, EventType, PropertyIdentifier};

/// AI-1 at 85 and EE-1 watching it with OUT_OF_RANGE over 80 after
/// `time_delay` seconds.
fn over_the_limit(time_delay: u32) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai.set_present_value(85.0);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();
    let mut ee = EventEnrollmentObject::new(1, "EE-1", EventType::OUT_OF_RANGE).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )))
    .unwrap();
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee.set_event_enable(EventTransitionBits::all());
    db.add(Box::new(ee)).unwrap();
    db
}

#[test]
fn the_interval_saturates_into_the_core() {
    assert_eq!(super::core_interval(10), 10);
    assert_eq!(super::core_interval(u64::from(u32::MAX)), u32::MAX);
    assert_eq!(super::core_interval(u64::MAX), u32::MAX);
}

/// Above `u32::MAX` seconds any non-zero delay is one pass, as at
/// `u32::MAX` itself, and a zero delay fires at once.
#[test]
fn an_interval_above_u32_max_rounds_every_delay_as_before() {
    let max = u64::from(u32::MAX);
    for interval in [max, max + 1, u64::MAX] {
        for time_delay in [0, 5, u32::MAX] {
            let mut db = over_the_limit(time_delay);
            let first = super::evaluate_event_enrollments(&mut db, interval);
            let second = super::evaluate_event_enrollments(&mut db, interval);
            let fired = |transitions: &[super::EventEnrollmentTransition]| {
                transitions.iter().map(|t| t.change.to).collect::<Vec<_>>()
            };
            let (first, second) = (fired(&first), fired(&second));
            if time_delay == 0 {
                assert_eq!(first, [EventState::HIGH_LIMIT], "{interval} {time_delay}");
                assert!(second.is_empty(), "{interval} {time_delay}");
            } else {
                assert!(
                    first.is_empty(),
                    "{interval} {time_delay}: one pass to wait"
                );
                assert_eq!(second, [EventState::HIGH_LIMIT], "{interval} {time_delay}");
            }
        }
    }
}
