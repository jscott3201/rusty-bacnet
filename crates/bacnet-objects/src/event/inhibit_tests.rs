//! Event_Algorithm_Inhibit and Event_Message_Texts_Config on the objects
//! that report intrinsically (#1329): the inhibit stops the algorithm's
//! offnormal and normal transitions but not fault detection (Clause
//! 13.2.2.1), and a configured text is what a transition stores.

use bacnet_types::bitstring::LimitEnable;
use bacnet_types::enums::{
    EngineeringUnits, EventState, EventType, PropertyIdentifier as P, Reliability,
};
use bacnet_types::primitives::{BACnetTimeStamp, PropertyValue};

use super::{commit_test_proposal, EventTransitionCommit, TransitionOutcome};
use crate::access_control::AccessZoneObject;
use crate::analog::AnalogInputObject;
use crate::binary::{BinaryOutputObject, BinaryValueObject};
use crate::event_log::EventLogObject;
use crate::traits::BACnetObject;

fn write(object: &mut dyn BACnetObject, property: P, value: PropertyValue) {
    object.write_property(property, None, value, None).unwrap();
}

fn inhibit(object: &mut dyn BACnetObject, on: bool) {
    write(
        object,
        P::EVENT_ALGORITHM_INHIBIT,
        PropertyValue::Boolean(on),
    );
}

fn state(object: &dyn BACnetObject) -> EventState {
    match object.read_property(P::EVENT_STATE, None).unwrap() {
        PropertyValue::Enumerated(raw) => EventState::from_raw(raw),
        other => panic!("Event_State read {other:?}"),
    }
}

/// Propose and commit one transition, returning where it went.
fn step(object: &mut dyn BACnetObject, outcome: Option<TransitionOutcome>) -> Option<EventState> {
    outcome.map(|outcome| commit_test_proposal(object, outcome).change.to)
}

/// An Analog Input above its High_Limit, with the delays given.
fn high_input(time_delay: u32) -> AnalogInputObject {
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    write(&mut ai, P::HIGH_LIMIT, PropertyValue::Real(80.0));
    write(
        &mut ai,
        P::LIMIT_ENABLE,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![LimitEnable::all().to_bacnet()],
        },
    );
    write(
        &mut ai,
        P::TIME_DELAY,
        PropertyValue::Unsigned(u64::from(time_delay)),
    );
    write(&mut ai, P::TIME_DELAY_NORMAL, PropertyValue::Unsigned(9));
    ai.set_present_value(90.0);
    ai
}

#[test]
fn an_inhibited_algorithm_proposes_no_offnormal_transition_but_still_faults() {
    let mut ai = high_input(0);
    inhibit(&mut ai, true);
    assert_eq!(ai.evaluate_intrinsic_reporting(), None);
    assert_eq!(ai.tick_intrinsic_reporting(), None);
    assert_eq!(state(&ai), EventState::NORMAL);

    // Fault detection runs on: into FAULT, and back to NORMAL from it.
    write(&mut ai, P::OUT_OF_SERVICE, PropertyValue::Boolean(true));
    write(
        &mut ai,
        P::RELIABILITY,
        PropertyValue::Enumerated(Reliability::OVER_RANGE.to_raw()),
    );
    let entered = ai.evaluate_intrinsic_reporting();
    assert_eq!(step(&mut ai, entered), Some(EventState::FAULT));
    write(
        &mut ai,
        P::RELIABILITY,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
    );
    let recovered = ai.evaluate_intrinsic_reporting();
    assert_eq!(step(&mut ai, recovered), Some(EventState::NORMAL));
    // Still inhibited, the out-of-range value proposes nothing.
    assert_eq!(ai.evaluate_intrinsic_reporting(), None);

    // Cleared, the condition is reported.
    inhibit(&mut ai, false);
    let offnormal = ai.evaluate_intrinsic_reporting();
    assert_eq!(step(&mut ai, offnormal), Some(EventState::HIGH_LIMIT));
}

#[test]
fn turning_the_inhibit_on_returns_an_offnormal_object_to_normal_at_once() {
    let mut ai = high_input(0);
    let offnormal = ai.evaluate_intrinsic_reporting();
    assert_eq!(step(&mut ai, offnormal), Some(EventState::HIGH_LIMIT));
    inhibit(&mut ai, true);
    // No Time_Delay_Normal wait: the inhibit, not the algorithm, decides.
    let outcome = ai.tick_intrinsic_reporting().expect("back to NORMAL");
    assert_eq!(outcome.change.from, EventState::HIGH_LIMIT);
    assert_eq!(outcome.change.to, EventState::NORMAL);
    assert_eq!(outcome.event_type, EventType::OUT_OF_RANGE);
    commit_test_proposal(&mut ai, outcome);
    assert_eq!(ai.tick_intrinsic_reporting(), None);
}

#[test]
fn a_condition_has_to_last_its_whole_delay_after_the_inhibit_clears() {
    let mut ai = high_input(2);
    // A countdown under way is dropped by the inhibit.
    assert_eq!(ai.evaluate_intrinsic_reporting(), None);
    assert_eq!(ai.tick_intrinsic_reporting(), None);
    inhibit(&mut ai, true);
    assert_eq!(ai.tick_intrinsic_reporting(), None);
    inhibit(&mut ai, false);
    // Two whole seconds from the change to FALSE.
    assert_eq!(ai.tick_intrinsic_reporting(), None);
    assert_eq!(ai.tick_intrinsic_reporting(), None);
    let outcome = ai.tick_intrinsic_reporting();
    assert_eq!(step(&mut ai, outcome), Some(EventState::HIGH_LIMIT));
}

#[test]
fn the_change_of_state_and_command_failure_detectors_take_the_inhibit_too() {
    // A Binary Value in its alarm state.
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    write(&mut bv, P::ALARM_VALUE, PropertyValue::Enumerated(1));
    // A Binary Output whose feedback disagrees with its command.
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    for object in [&mut bv as &mut dyn BACnetObject, &mut bo] {
        // A Binary Output starts with detection off, when the inhibit can't
        // be written.
        write(
            object,
            P::EVENT_DETECTION_ENABLE,
            PropertyValue::Boolean(true),
        );
        object
            .write_property_from(
                P::PRESENT_VALUE,
                None,
                PropertyValue::Enumerated(1),
                Some(8),
                &crate::command_source::test_origin(),
            )
            .unwrap();
    }
    for object in [&mut bv as &mut dyn BACnetObject, &mut bo] {
        let name = object.object_name().to_owned();
        inhibit(object, true);
        assert_eq!(object.evaluate_intrinsic_reporting(), None, "{name}");
        inhibit(object, false);
        let offnormal = object.evaluate_intrinsic_reporting();
        assert_eq!(
            step(object, offnormal),
            Some(EventState::OFFNORMAL),
            "{name}"
        );
        inhibit(object, true);
        let normal = object.evaluate_intrinsic_reporting();
        assert_eq!(step(object, normal), Some(EventState::NORMAL), "{name}");
    }
}

#[test]
fn an_access_zone_takes_the_rows_and_the_inhibit() {
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    inhibit(&mut zone, true);
    assert_eq!(
        zone.read_property(P::EVENT_ALGORITHM_INHIBIT, None)
            .unwrap(),
        PropertyValue::Boolean(true)
    );
    let texts = PropertyValue::List(
        ["Full", "Broken", "Clear"]
            .map(|text| PropertyValue::CharacterString(text.into()))
            .to_vec(),
    );
    write(&mut zone, P::EVENT_MESSAGE_TEXTS_CONFIG, texts.clone());
    assert_eq!(
        zone.read_property(P::EVENT_MESSAGE_TEXTS_CONFIG, None)
            .unwrap(),
        texts
    );
    // Detection off: the inhibit can't be written, and the rows survive the
    // reset of the event history.
    write(
        &mut zone,
        P::EVENT_DETECTION_ENABLE,
        PropertyValue::Boolean(false),
    );
    assert!(zone
        .write_property(
            P::EVENT_ALGORITHM_INHIBIT,
            None,
            PropertyValue::Boolean(false),
            None
        )
        .is_err());
    assert_eq!(
        zone.read_property(P::EVENT_MESSAGE_TEXTS_CONFIG, None)
            .unwrap(),
        texts
    );
}

#[test]
fn a_configured_text_is_the_message_a_transition_stores() {
    let mut ai = high_input(0);
    write(
        &mut ai,
        P::EVENT_MESSAGE_TEXTS_CONFIG,
        PropertyValue::List(vec![
            PropertyValue::CharacterString("Too hot".into()),
            PropertyValue::CharacterString(String::new()),
            PropertyValue::CharacterString(String::new()),
        ]),
    );
    let offnormal = ai.evaluate_intrinsic_reporting().unwrap();
    let commit = |outcome: TransitionOutcome| EventTransitionCommit {
        coordinate: outcome.change.transition(),
        change: outcome.change,
        ack_required: false,
        timestamp: BACnetTimeStamp::SequenceNumber(1),
        message_text: Some("server text".into()),
    };
    ai.commit_event_transition_internal(commit(offnormal))
        .unwrap();
    assert_eq!(
        ai.read_property(P::EVENT_MESSAGE_TEXTS, Some(1)).unwrap(),
        PropertyValue::CharacterString("Too hot".into())
    );
    // An empty entry leaves the server's text.
    write(&mut ai, P::TIME_DELAY_NORMAL, PropertyValue::Unsigned(0));
    ai.set_present_value(10.0);
    let normal = ai.evaluate_intrinsic_reporting().unwrap();
    ai.commit_event_transition_internal(commit(normal)).unwrap();
    assert_eq!(
        ai.read_property(P::EVENT_MESSAGE_TEXTS, Some(3)).unwrap(),
        PropertyValue::CharacterString("server text".into())
    );
}

#[test]
fn an_inhibited_log_holds_its_buffer_ready_report() {
    let mut log = EventLogObject::new(1, "EL-1", 8).unwrap();
    log.bind_clock_internal(Some(std::sync::Arc::new(FixedClock)));
    write(
        &mut log,
        P::NOTIFICATION_THRESHOLD,
        PropertyValue::Unsigned(1),
    );
    inhibit(&mut log, true);
    log.add_event_log_record(record()).unwrap();
    assert_eq!(log.tick_intrinsic_reporting(), None);
    assert_eq!(log.evaluate_intrinsic_reporting(), None);
    // The record still counts, so the report falls due once it clears.
    inhibit(&mut log, false);
    let report = log.tick_intrinsic_reporting().expect("due");
    assert_eq!(report.change.to, EventState::NORMAL);
}

struct FixedClock;

impl crate::clock::ClockReader for FixedClock {
    fn read_clock(&self) -> Option<crate::clock::ClockFrame> {
        Some(crate::clock::ClockFrame {
            local_date: DATE,
            local_time: TIME,
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

const DATE: bacnet_types::primitives::Date = bacnet_types::primitives::Date {
    year: 126,
    month: 10,
    day: 5,
    day_of_week: 1,
};

const TIME: bacnet_types::primitives::Time = bacnet_types::primitives::Time {
    hour: 9,
    minute: 0,
    second: 0,
    hundredths: 0,
};

fn record() -> bacnet_types::constructed::BACnetEventLogRecord {
    bacnet_types::constructed::BACnetEventLogRecord {
        date: DATE,
        time: TIME,
        log_datum: bacnet_types::constructed::EventLogDatum::TimeChange(1.0),
    }
}
