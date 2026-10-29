//! Access Zone intrinsic reporting: CHANGE_OF_STATE on Occupancy_State
//! (Clause 12.32, Table 12-37, Clause 13.3.2, #1305).
//!
//! Configuration goes through `write_property`, the route a client takes.

use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::PropertyIdentifier as P;
use bacnet_types::enums::{
    AccessZoneOccupancyState as S, ErrorClass, ErrorCode, EventType, Reliability,
};
use bacnet_types::error::RemoteError;

use super::*;
use crate::event::{
    EventStateChange, EventTransition, EventTransitionCommit, TransitionOutcome,
    commit_test_proposal,
};

fn read(zone: &AccessZoneObject, property: P) -> PropertyValue {
    zone.read_property(property, None).unwrap()
}

fn write(zone: &mut AccessZoneObject, property: P, value: PropertyValue) -> Result<(), Error> {
    zone.write_property(property, None, value, None)
}

fn states(states: &[S]) -> PropertyValue {
    PropertyValue::List(
        states
            .iter()
            .map(|state| PropertyValue::Enumerated(state.to_raw()))
            .collect(),
    )
}

fn transition_bits(bits: EventTransitionBits) -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: 5,
        data: vec![bits.to_bacnet()],
    }
}

/// A zone with an upper limit of 5 that alarms above it, after `time_delay`
/// seconds, with every transition distributed.
fn alarming_zone(time_delay: u64) -> AccessZoneObject {
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    zone.set_occupancy_limits(0, 5).unwrap();
    write(&mut zone, P::ALARM_VALUES, states(&[S::ABOVE_UPPER_LIMIT])).unwrap();
    write(
        &mut zone,
        P::TIME_DELAY,
        PropertyValue::Unsigned(time_delay),
    )
    .unwrap();
    write(
        &mut zone,
        P::EVENT_ENABLE,
        transition_bits(EventTransitionBits::all()),
    )
    .unwrap();
    zone
}

fn adjust(zone: &mut AccessZoneObject, value: i32) {
    write(zone, P::ADJUST_VALUE, PropertyValue::Signed(value)).unwrap();
}

fn event_state(zone: &AccessZoneObject) -> PropertyValue {
    read(zone, P::EVENT_STATE)
}

fn enumerated(state: EventState) -> PropertyValue {
    PropertyValue::Enumerated(state.to_raw())
}

fn change(from: EventState, to: EventState) -> EventStateChange {
    EventStateChange { from, to }
}

fn assert_protocol_error(result: Result<(), Error>, class: ErrorClass, code: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class: c, code: e }))
            if c == class && e == code),
        "expected {class:?} / {code:?}, got {result:?}"
    );
}

#[test]
fn access_zone_serves_the_event_rows() {
    let zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    let no_stamp = PropertyValue::ApplicationData(vec![0x19, 0x00]);
    for (property, value) in [
        (P::TIME_DELAY, PropertyValue::Unsigned(0)),
        (P::NOTIFICATION_CLASS, PropertyValue::Unsigned(0)),
        (P::ALARM_VALUES, PropertyValue::List(vec![])),
        (
            P::EVENT_ENABLE,
            transition_bits(EventTransitionBits::empty()),
        ),
        (
            P::ACKED_TRANSITIONS,
            transition_bits(EventTransitionBits::all()),
        ),
        (P::NOTIFY_TYPE, PropertyValue::Enumerated(0)),
        (
            P::EVENT_TIME_STAMPS,
            PropertyValue::List(vec![no_stamp.clone(), no_stamp.clone(), no_stamp]),
        ),
        (
            P::EVENT_MESSAGE_TEXTS,
            PropertyValue::List(vec![PropertyValue::CharacterString(String::new()); 3]),
        ),
        (P::EVENT_DETECTION_ENABLE, PropertyValue::Boolean(true)),
        (P::TIME_DELAY_NORMAL, PropertyValue::Unsigned(0)),
        (P::EVENT_STATE, enumerated(EventState::NORMAL)),
    ] {
        assert_eq!(read(&zone, property), value, "{property:?}");
        assert!(zone.property_list().contains(&property), "{property:?}");
    }
    assert_eq!(
        zone.enrollment_summary_capability_internal()
            .map(|capability| capability.event_type),
        Some(EventType::CHANGE_OF_STATE)
    );
}

#[test]
fn access_zone_alarm_values_take_occupancy_states_only() {
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    let both = states(&[S::ABOVE_UPPER_LIMIT, S::BELOW_LOWER_LIMIT]);
    write(&mut zone, P::ALARM_VALUES, both.clone()).unwrap();
    assert_eq!(read(&zone, P::ALARM_VALUES), both);
    // A local write may give a one-element list as the element alone.
    write(&mut zone, P::ALARM_VALUES, PropertyValue::Enumerated(5)).unwrap();
    assert_eq!(read(&zone, P::ALARM_VALUES), states(&[S::DISABLED]));
    // A proprietary state is a BACnetAccessZoneOccupancyState too.
    let proprietary = PropertyValue::List(vec![PropertyValue::Enumerated(65_535)]);
    write(&mut zone, P::ALARM_VALUES, proprietary.clone()).unwrap();
    assert_eq!(read(&zone, P::ALARM_VALUES), proprietary);

    for (value, class, code, position) in [
        (
            PropertyValue::List(vec![
                PropertyValue::Enumerated(4),
                PropertyValue::Enumerated(7),
            ]),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
            2,
        ),
        (
            PropertyValue::List(vec![PropertyValue::Enumerated(65_536)]),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
            1,
        ),
        (
            PropertyValue::List(vec![PropertyValue::Unsigned(4)]),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
            1,
        ),
    ] {
        crate::common::assert_list_element_refused(
            write(&mut zone, P::ALARM_VALUES, value),
            class,
            code,
            position,
            "Alarm_Values",
        );
        assert_eq!(read(&zone, P::ALARM_VALUES), proprietary);
    }
    // A lone value of another datatype is a one-element list's element.
    crate::common::assert_list_element_refused(
        write(&mut zone, P::ALARM_VALUES, PropertyValue::Unsigned(4)),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
        1,
        "lone Unsigned",
    );
    assert_protocol_error(
        zone.write_property(P::ALARM_VALUES, Some(1), PropertyValue::Enumerated(4), None),
        ErrorClass::PROPERTY,
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
    );
    assert_eq!(read(&zone, P::ALARM_VALUES), proprietary);

    // The setter checks the same way.
    zone.set_alarm_values([S::AT_UPPER_LIMIT]).unwrap();
    assert_eq!(read(&zone, P::ALARM_VALUES), states(&[S::AT_UPPER_LIMIT]));
    assert!(zone.set_alarm_values([S::NORMAL, S::from_raw(63)]).is_err());
    assert_eq!(read(&zone, P::ALARM_VALUES), states(&[S::AT_UPPER_LIMIT]));
}

#[test]
fn access_zone_alarm_values_refuse_normal() {
    // NORMAL is the state with no limit crossed, so listing it as an alarm
    // value would alarm a zone whose count sits inside its limits (#1401).
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    let held = states(&[S::ABOVE_UPPER_LIMIT]);
    write(&mut zone, P::ALARM_VALUES, held.clone()).unwrap();
    for (value, position, context) in [
        (states(&[S::AT_UPPER_LIMIT, S::NORMAL]), 2, "NORMAL second"),
        (states(&[S::NORMAL]), 1, "NORMAL alone"),
        (PropertyValue::Enumerated(0), 1, "a lone NORMAL"),
    ] {
        crate::common::assert_list_element_refused(
            write(&mut zone, P::ALARM_VALUES, value),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
            position,
            context,
        );
        assert_eq!(read(&zone, P::ALARM_VALUES), held, "{context}");
    }
    crate::common::assert_list_element_refused(
        zone.set_alarm_values([S::BELOW_LOWER_LIMIT, S::NORMAL]),
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
        2,
        "the setter",
    );
    assert_eq!(read(&zone, P::ALARM_VALUES), held);

    // Every other named state, and the first proprietary one, is taken.
    let others = [
        S::BELOW_LOWER_LIMIT,
        S::AT_LOWER_LIMIT,
        S::AT_UPPER_LIMIT,
        S::ABOVE_UPPER_LIMIT,
        S::DISABLED,
        S::NOT_SUPPORTED,
        S::from_raw(64),
    ];
    zone.set_alarm_values(others).unwrap();
    assert_eq!(read(&zone, P::ALARM_VALUES), states(&others));
}

#[test]
fn access_zone_goes_offnormal_after_time_delay_and_back_to_normal() {
    let mut zone = alarming_zone(2);
    adjust(&mut zone, 6);
    assert_eq!(zone.occupancy_state(), S::ABOVE_UPPER_LIMIT);
    // The write seeds the countdown; nothing fires until it runs out.
    assert_eq!(zone.evaluate_intrinsic_reporting(), None);
    assert_eq!(zone.tick_intrinsic_reporting(), None);
    assert_eq!(event_state(&zone), enumerated(EventState::NORMAL));
    let offnormal = zone.tick_intrinsic_reporting().unwrap();
    assert_eq!(
        offnormal,
        TransitionOutcome {
            change: change(EventState::NORMAL, EventState::OFFNORMAL),
            event_type: EventType::CHANGE_OF_STATE,
            distribute: true,
        }
    );
    commit_test_proposal(&mut zone, offnormal);
    assert_eq!(event_state(&zone), enumerated(EventState::OFFNORMAL));
    assert_eq!(
        read(&zone, P::STATUS_FLAGS),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0x80],
        }
    );
    // Staying in alarm proposes nothing more.
    assert_eq!(zone.evaluate_intrinsic_reporting(), None);
    assert_eq!(zone.tick_intrinsic_reporting(), None);

    // Back below the limit: Time_Delay_Normal, unset, falls back to
    // Time_Delay.
    adjust(&mut zone, -3);
    assert_eq!(zone.occupancy_state(), S::NORMAL);
    assert_eq!(zone.evaluate_intrinsic_reporting(), None);
    assert_eq!(zone.tick_intrinsic_reporting(), None);
    let normal = zone.tick_intrinsic_reporting().unwrap();
    assert_eq!(
        normal.change,
        change(EventState::OFFNORMAL, EventState::NORMAL)
    );
    commit_test_proposal(&mut zone, normal);
    assert_eq!(event_state(&zone), enumerated(EventState::NORMAL));
    assert_eq!(
        read(&zone, P::STATUS_FLAGS),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0x00],
        }
    );
}

#[test]
fn access_zone_time_delay_normal_governs_only_the_return_to_normal() {
    let mut zone = alarming_zone(1);
    write(&mut zone, P::TIME_DELAY_NORMAL, PropertyValue::Unsigned(3)).unwrap();
    assert_eq!(read(&zone, P::TIME_DELAY), PropertyValue::Unsigned(1));
    assert_eq!(
        read(&zone, P::TIME_DELAY_NORMAL),
        PropertyValue::Unsigned(3)
    );

    // Into alarm on Time_Delay's single second.
    adjust(&mut zone, 6);
    assert_eq!(zone.evaluate_intrinsic_reporting(), None);
    let offnormal = zone.tick_intrinsic_reporting().unwrap();
    assert_eq!(
        offnormal.change,
        change(EventState::NORMAL, EventState::OFFNORMAL)
    );
    commit_test_proposal(&mut zone, offnormal);

    // Out of it only after Time_Delay_Normal's three.
    adjust(&mut zone, -3);
    assert_eq!(zone.evaluate_intrinsic_reporting(), None);
    assert_eq!(zone.tick_intrinsic_reporting(), None);
    assert_eq!(zone.tick_intrinsic_reporting(), None);
    assert_eq!(
        zone.tick_intrinsic_reporting()
            .map(|outcome| outcome.change),
        Some(change(EventState::OFFNORMAL, EventState::NORMAL))
    );
}

#[test]
fn access_zone_alarm_values_past_the_cap_are_no_space() {
    let cap = crate::multistate::MAX_ALARM_VALUES;
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    let full = PropertyValue::List(vec![PropertyValue::Enumerated(4); cap]);
    write(&mut zone, P::ALARM_VALUES, full.clone()).unwrap();
    crate::common::assert_list_element_refused(
        write(
            &mut zone,
            P::ALARM_VALUES,
            PropertyValue::List(vec![PropertyValue::Enumerated(4); cap + 1]),
        ),
        ErrorClass::RESOURCES,
        ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        u32::try_from(cap + 1).unwrap(),
        "one past the cap",
    );
    assert_eq!(read(&zone, P::ALARM_VALUES), full);
}

#[test]
fn access_zone_condition_cleared_within_time_delay_reports_nothing() {
    let mut zone = alarming_zone(3);
    adjust(&mut zone, 6);
    assert_eq!(zone.evaluate_intrinsic_reporting(), None);
    assert_eq!(zone.tick_intrinsic_reporting(), None);
    adjust(&mut zone, -1);
    assert_eq!(zone.occupancy_state(), S::AT_UPPER_LIMIT);
    assert_eq!(zone.evaluate_intrinsic_reporting(), None);
    for _ in 0..5 {
        assert_eq!(zone.tick_intrinsic_reporting(), None);
    }
    assert_eq!(event_state(&zone), enumerated(EventState::NORMAL));
}

#[test]
fn access_zone_event_enable_gates_distribution_not_the_transition() {
    let mut zone = alarming_zone(0);
    write(
        &mut zone,
        P::EVENT_ENABLE,
        transition_bits(EventTransitionBits::TO_NORMAL),
    )
    .unwrap();
    adjust(&mut zone, 6);
    let offnormal = zone.evaluate_intrinsic_reporting().unwrap();
    assert!(!offnormal.distribute);
    commit_test_proposal(&mut zone, offnormal);
    assert_eq!(event_state(&zone), enumerated(EventState::OFFNORMAL));
    adjust(&mut zone, 0);
    let normal = zone.evaluate_intrinsic_reporting().unwrap();
    assert_eq!(
        normal.change,
        change(EventState::OFFNORMAL, EventState::NORMAL)
    );
    assert!(normal.distribute);
}

#[test]
fn access_zone_event_detection_enable_suspends_and_resets_reporting() {
    let mut zone = alarming_zone(0);
    adjust(&mut zone, 6);
    let offnormal = zone.evaluate_intrinsic_reporting().unwrap();
    let stamp = BACnetTimeStamp::SequenceNumber(9);
    zone.commit_event_transition_internal(EventTransitionCommit {
        change: offnormal.change.clone(),
        coordinate: EventTransition::ToOffnormal,
        ack_required: true,
        timestamp: stamp.clone(),
        message_text: Some("too many".into()),
    })
    .unwrap();
    assert_eq!(
        read(&zone, P::ACKED_TRANSITIONS),
        transition_bits(EventTransitionBits::TO_FAULT | EventTransitionBits::TO_NORMAL)
    );

    write(
        &mut zone,
        P::EVENT_DETECTION_ENABLE,
        PropertyValue::Boolean(false),
    )
    .unwrap();
    let fresh = AccessZoneObject::new(2, "ZONE-2").unwrap();
    for property in [
        P::EVENT_STATE,
        P::ACKED_TRANSITIONS,
        P::EVENT_TIME_STAMPS,
        P::EVENT_MESSAGE_TEXTS,
        P::STATUS_FLAGS,
    ] {
        assert_eq!(
            read(&zone, property),
            read(&fresh, property),
            "{property:?}"
        );
    }
    // Still above the limit, but detection is off.
    assert_eq!(zone.evaluate_intrinsic_reporting(), None);
    assert_eq!(zone.tick_intrinsic_reporting(), None);
    assert_protocol_error(
        zone.acknowledge_alarm_correlated_internal(EventState::OFFNORMAL, &stamp),
        ErrorClass::OBJECT,
        ErrorCode::NO_ALARM_CONFIGURED,
    );
    assert_protocol_error(
        write(
            &mut zone,
            P::EVENT_DETECTION_ENABLE,
            PropertyValue::Unsigned(1),
        ),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
    );

    // Turned back on, the standing alarm is detected again.
    write(
        &mut zone,
        P::EVENT_DETECTION_ENABLE,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    assert_eq!(
        zone.evaluate_intrinsic_reporting()
            .map(|outcome| outcome.change),
        Some(change(EventState::NORMAL, EventState::OFFNORMAL))
    );
}

#[test]
fn access_zone_acknowledges_its_latest_offnormal_transition() {
    let mut zone = alarming_zone(0);
    adjust(&mut zone, 6);
    let offnormal = zone.evaluate_intrinsic_reporting().unwrap();
    let stamp = BACnetTimeStamp::SequenceNumber(41);
    zone.commit_event_transition_internal(EventTransitionCommit {
        change: offnormal.change,
        coordinate: EventTransition::ToOffnormal,
        ack_required: true,
        timestamp: stamp.clone(),
        message_text: None,
    })
    .unwrap();
    // A stale time stamp is refused and changes nothing.
    assert!(
        zone.acknowledge_alarm_correlated_detailed_internal(
            EventState::OFFNORMAL,
            &BACnetTimeStamp::SequenceNumber(40),
        )
        .is_err()
    );
    assert_eq!(
        zone.acknowledge_alarm_correlated_detailed_internal(EventState::OFFNORMAL, &stamp)
            .unwrap(),
        Some(change(EventState::NORMAL, EventState::OFFNORMAL))
    );
    assert_eq!(
        read(&zone, P::ACKED_TRANSITIONS),
        transition_bits(EventTransitionBits::all())
    );
    assert_eq!(
        read(&zone, P::EVENT_TIME_STAMPS),
        PropertyValue::List(vec![
            PropertyValue::ApplicationData(vec![0x19, 41]),
            PropertyValue::ApplicationData(vec![0x19, 0x00]),
            PropertyValue::ApplicationData(vec![0x19, 0x00]),
        ])
    );
}

#[test]
fn access_zone_simulated_fault_and_count_drive_the_event_state() {
    let mut zone = alarming_zone(0);
    write(&mut zone, P::OUT_OF_SERVICE, PropertyValue::Boolean(true)).unwrap();
    write(
        &mut zone,
        P::RELIABILITY,
        PropertyValue::Enumerated(Reliability::UNRELIABLE_OTHER.to_raw()),
    )
    .unwrap();
    let fault = zone.evaluate_intrinsic_reporting().unwrap();
    assert_eq!(fault.change, change(EventState::NORMAL, EventState::FAULT));
    assert_eq!(fault.event_type, EventType::CHANGE_OF_RELIABILITY);
    commit_test_proposal(&mut zone, fault);
    assert_eq!(event_state(&zone), enumerated(EventState::FAULT));

    write(
        &mut zone,
        P::RELIABILITY,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
    )
    .unwrap();
    let recovered = zone.evaluate_intrinsic_reporting().unwrap();
    assert_eq!(
        recovered.change,
        change(EventState::FAULT, EventState::NORMAL)
    );
    commit_test_proposal(&mut zone, recovered);

    // A simulated count over the limit is an alarm like a counted one
    // (Clause 12.32.10).
    write(&mut zone, P::OCCUPANCY_COUNT, PropertyValue::Unsigned(9)).unwrap();
    assert_eq!(
        zone.evaluate_intrinsic_reporting()
            .map(|outcome| outcome.change),
        Some(change(EventState::NORMAL, EventState::OFFNORMAL))
    );
}
