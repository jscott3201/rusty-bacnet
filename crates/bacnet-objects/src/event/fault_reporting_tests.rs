//! Clause 13.2.2 fault precedence through intrinsic reporting's tick, which
//! needs the `event-reporting` feature (#2215); `fault_tests` holds the rest.

use super::*;
use crate::event::test_clock::{test_now, test_tick};
use crate::traits::EventReporting as _;

const NO_FAULT: Reliability = Reliability::NO_FAULT_DETECTED;
const OVER_RANGE: Reliability = Reliability::OVER_RANGE;

#[test]
fn ticking_an_object_uses_reliability_for_fault_and_recovery() {
    // The running server drives this periodic macro arm.
    use crate::analog::AnalogInputObject;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::EngineeringUnits;
    use bacnet_types::enums::PropertyIdentifier;
    use bacnet_types::primitives::PropertyValue;

    let mut ai =
        AnalogInputObject::new(3, "ai-3", EngineeringUnits::DEGREES_CELSIUS).expect("construct");
    ai.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .expect("Out_Of_Service is writable");
    ai.write_property(
        PropertyIdentifier::RELIABILITY,
        None,
        PropertyValue::Enumerated(OVER_RANGE.to_raw()),
        None,
    )
    .expect("reliability is writable");
    let proposal = ai
        .tick_intrinsic_reporting(test_tick())
        .expect("periodic tick must observe bad reliability");
    let fault = commit_test_proposal(&mut ai, proposal);
    assert_eq!(fault.change.to, EventState::FAULT);

    ai.write_property(
        PropertyIdentifier::RELIABILITY,
        None,
        PropertyValue::Enumerated(NO_FAULT.to_raw()),
        None,
    )
    .expect("reliability is writable");
    let recovery = ai
        .tick_intrinsic_reporting(test_tick())
        .expect("periodic tick must observe recovered reliability");
    assert_eq!(recovery.change.from, EventState::FAULT);
    assert_eq!(recovery.change.to, EventState::NORMAL);
}

#[test]
fn faulted_object_reports_both_fault_and_in_alarm_status_flags() {
    // Clause 12.2 derives IN_ALARM from Event_State and FAULT from Reliability,
    // without specifying how the individual flags relate to each other.
    // They are reconciled upstream: while event-state-detection is
    // enabled, Clause 13.2.2 makes a bad Reliability determine Event_State =
    // FAULT, so both flags read TRUE together. Before #167 this object reported
    // FAULT TRUE with IN_ALARM FALSE, because Reliability never reached
    // Event_State.
    use crate::analog::AnalogInputObject;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::EngineeringUnits;
    use bacnet_types::enums::PropertyIdentifier;
    use bacnet_types::primitives::{PropertyValue, StatusFlags};

    let mut ai =
        AnalogInputObject::new(2, "ai-2", EngineeringUnits::DEGREES_CELSIUS).expect("construct");
    ai.reliability_eval()
        .unwrap()
        .set_reliability(OVER_RANGE)
        .expect("in-service reliability evaluation is supported");
    let proposal = ai
        .evaluate_intrinsic_reporting(test_now())
        .expect("bad reliability must propose FAULT");
    commit_test_proposal(&mut ai, proposal);

    let flags = ai
        .read_property(PropertyIdentifier::STATUS_FLAGS, None)
        .expect("status flags readable");
    let PropertyValue::BitString { data, .. } = flags else {
        panic!("Status_Flags is a bit string");
    };
    let bits = data[0] >> 4;
    assert_ne!(
        bits & StatusFlags::FAULT.bits(),
        0,
        "FAULT follows Reliability"
    );
    assert_ne!(
        bits & StatusFlags::IN_ALARM.bits(),
        0,
        "IN_ALARM follows Event_State, which Reliability now drives"
    );
}
