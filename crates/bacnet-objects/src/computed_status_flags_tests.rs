//! Status_Flags is computed from Reliability, Out_Of_Service and Event_State
//! on Loop, Schedule, Trend Log and Trend Log Multiple (#978). Calendar has no
//! Status_Flags at all (Table 12-11, #984).
//!
//! These objects used to return a Status_Flags fixed at construction, so FAULT
//! and OUT_OF_SERVICE never moved. Each step below checks the four bits against
//! the object's own Reliability, Out_Of_Service and Event_State readbacks, then
//! pins the expected bits outright. The two log objects fix OUT_OF_SERVICE and
//! OVERRIDDEN at FALSE (Clauses 12.25.30 and 12.30.5), so for them only FAULT
//! and IN_ALARM may move.

use crate::loop_obj::LoopObject;
use crate::schedule::{CalendarObject, ScheduleObject};
use crate::traits::BACnetObject;
use crate::trend::{TrendLogMultipleObject, TrendLogObject};
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, EventState, PropertyIdentifier, Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{PropertyValue, StatusFlags};

fn read(object: &dyn BACnetObject, property: PropertyIdentifier) -> PropertyValue {
    object
        .read_property(property, None)
        .expect("test property must be readable")
}

fn status_flags(object: &dyn BACnetObject) -> StatusFlags {
    match read(object, PropertyIdentifier::STATUS_FLAGS) {
        PropertyValue::BitString {
            unused_bits: 4,
            data,
        } if data.len() == 1 => StatusFlags::from_bits_truncate(data[0] >> 4),
        other => panic!("Status_Flags must be a four-bit bit string, got {other:?}"),
    }
}

/// How an object's OUT_OF_SERVICE and OVERRIDDEN bits behave.
#[derive(Clone, Copy)]
enum Kind {
    /// OUT_OF_SERVICE follows the Out_Of_Service property.
    FollowsOutOfService,
    /// OUT_OF_SERVICE and OVERRIDDEN are always FALSE (Trend Log and Trend
    /// Log Multiple, which have no Out_Of_Service property).
    LogFixedFalse,
}

/// Assert the flags agree with the object's readable state, then that they
/// equal `expected`.
fn assert_flags(object: &dyn BACnetObject, kind: Kind, expected: StatusFlags) {
    let flags = status_flags(object);
    let reliability = match read(object, PropertyIdentifier::RELIABILITY) {
        PropertyValue::Enumerated(raw) => Reliability::from_raw(raw),
        other => panic!("Reliability must be Enumerated, got {other:?}"),
    };
    assert_eq!(
        flags.contains(StatusFlags::FAULT),
        reliability != Reliability::NO_FAULT_DETECTED,
        "FAULT must follow Reliability ({reliability})"
    );
    match kind {
        Kind::FollowsOutOfService => assert_eq!(
            flags.contains(StatusFlags::OUT_OF_SERVICE),
            read(object, PropertyIdentifier::OUT_OF_SERVICE) == PropertyValue::Boolean(true),
            "OUT_OF_SERVICE must follow Out_Of_Service"
        ),
        Kind::LogFixedFalse => assert!(
            !flags.intersects(StatusFlags::OUT_OF_SERVICE | StatusFlags::OVERRIDDEN),
            "a log object's OUT_OF_SERVICE and OVERRIDDEN must stay FALSE"
        ),
    }
    assert_eq!(
        flags.contains(StatusFlags::IN_ALARM),
        read(object, PropertyIdentifier::EVENT_STATE)
            != PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        "IN_ALARM must follow Event_State"
    );
    assert_eq!(flags, expected);
}

fn write(object: &mut dyn BACnetObject, property: PropertyIdentifier, value: PropertyValue) {
    object
        .write_property(property, None, value, None)
        .expect("test write must succeed");
}

fn set_out_of_service(object: &mut dyn BACnetObject, value: bool) {
    write(
        object,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(value),
    );
}

/// Loop and Schedule own Reliability in service and let a client simulate it
/// while Out_Of_Service is TRUE; the evaluated value comes back on return.
fn assert_simulation_and_evaluation_drive_flags(object: &mut dyn BACnetObject) {
    assert_flags(object, Kind::FollowsOutOfService, StatusFlags::empty());

    object
        .set_reliability_internal(Reliability::CONFIGURATION_ERROR)
        .unwrap();
    assert_flags(object, Kind::FollowsOutOfService, StatusFlags::FAULT);

    set_out_of_service(object, true);
    assert_flags(
        object,
        Kind::FollowsOutOfService,
        StatusFlags::FAULT | StatusFlags::OUT_OF_SERVICE,
    );

    write(
        object,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
    );
    assert_flags(
        object,
        Kind::FollowsOutOfService,
        StatusFlags::OUT_OF_SERVICE,
    );

    write(
        object,
        PropertyIdentifier::RELIABILITY,
        PropertyValue::Enumerated(Reliability::OPEN_LOOP.to_raw()),
    );
    assert_flags(
        object,
        Kind::FollowsOutOfService,
        StatusFlags::FAULT | StatusFlags::OUT_OF_SERVICE,
    );

    // Leaving Out_Of_Service restores the evaluated CONFIGURATION_ERROR.
    set_out_of_service(object, false);
    assert_flags(object, Kind::FollowsOutOfService, StatusFlags::FAULT);

    object
        .set_reliability_internal(Reliability::NO_FAULT_DETECTED)
        .unwrap();
    assert_flags(object, Kind::FollowsOutOfService, StatusFlags::empty());
}

#[test]
fn loop_status_flags_follow_reliability_and_out_of_service() {
    assert_simulation_and_evaluation_drive_flags(
        &mut LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    );
}

#[test]
fn schedule_status_flags_follow_reliability_and_out_of_service() {
    assert_simulation_and_evaluation_drive_flags(
        &mut ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(0.0)).unwrap(),
    );
}

#[test]
fn trend_log_flags_match_its_fixed_state() {
    // Table 12-29 defines no Out_Of_Service, so #985 removed the
    // compatibility row that #978 kept away from the flags. Its Reliability
    // can't change yet (writes are refused and there is no internal route),
    // so FAULT stays FALSE and agrees with the NO_FAULT_DETECTED readback.
    let mut log = TrendLogObject::new(1, "TL-1", 10).unwrap();
    assert_flags(&log, Kind::LogFixedFalse, StatusFlags::empty());
    assert!(log
        .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
        .is_err());
    assert!(log
        .write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::OPEN_LOOP.to_raw()),
            None,
        )
        .is_err());
    assert!(log
        .set_reliability_internal(Reliability::OPEN_LOOP)
        .is_err());
    assert_flags(&log, Kind::LogFixedFalse, StatusFlags::empty());
}

#[test]
fn trend_log_multiple_flags_match_its_fixed_state() {
    // Trend Log Multiple can't change Reliability yet and has no
    // Out_Of_Service (Table 12-35), so the computed flags stay clear and agree
    // with the Reliability readback.
    assert_flags(
        &TrendLogMultipleObject::new(1, "TLM-1", 10).unwrap(),
        Kind::LogFixedFalse,
        StatusFlags::empty(),
    );
}

#[test]
fn calendar_has_no_status_flags_to_compute() {
    // #984 removed the always-clear Status_Flags #978 computed for Calendar,
    // along with Event_State and Out_Of_Service: Table 12-11 defines none of
    // them, so each reads as an unknown property.
    let calendar = CalendarObject::new(1, "CAL-1").unwrap();
    for property in [
        PropertyIdentifier::STATUS_FLAGS,
        PropertyIdentifier::EVENT_STATE,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyIdentifier::RELIABILITY,
    ] {
        assert!(
            matches!(
                calendar.read_property(property, None),
                Err(Error::Protocol { class, code })
                    if class == ErrorClass::PROPERTY
                        && code == ErrorCode::UNKNOWN_PROPERTY
            ),
            "Calendar must not serve {property:?}"
        );
        assert!(!calendar.property_list().contains(&property));
    }
}
