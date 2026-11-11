//! AcknowledgeAlarm on an Event Enrollment (Clause 13.9): refused with
//! OBJECT / NO_ALARM_CONFIGURED while Event_Detection_Enable is FALSE or
//! while there is no monitored reference, and nothing on the object moves
//! (#2183).

use super::super::*;
use crate::event::{EventStateChange, EventTransition, EventTransitionCommit};
use crate::traits::EventReporting;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::BACnetTimeStamp;

const STAMP: BACnetTimeStamp = BACnetTimeStamp::SequenceNumber(42);

/// An enrollment monitoring BI-1's Present_Value that has gone OFFNORMAL
/// with an acknowledgment pending.
fn offnormal() -> EventEnrollmentObject {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_BITSTRING).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ObjectIdentifier::new(ObjectType::BINARY_INPUT, 1).unwrap(),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )))
    .unwrap();
    commit_offnormal(&mut ee, STAMP);
    assert_eq!(
        read(&ee, PropertyIdentifier::ACKED_TRANSITIONS),
        PropertyValue::BitString {
            unused_bits: 5,
            // TO_OFFNORMAL cleared, TO_FAULT and TO_NORMAL set.
            data: vec![0x60],
        }
    );
    ee
}

/// Commit NORMAL to OFFNORMAL at `timestamp`, acknowledgment required.
fn commit_offnormal(ee: &mut EventEnrollmentObject, timestamp: BACnetTimeStamp) {
    ee.commit_event_transition(EventTransitionCommit {
        change: EventStateChange {
            from: EventState::NORMAL,
            to: EventState::OFFNORMAL,
        },
        coordinate: EventTransition::ToOffnormal,
        ack_required: true,
        timestamp,
        message_text: None,
    })
    .unwrap();
}

fn read(ee: &EventEnrollmentObject, property: PropertyIdentifier) -> PropertyValue {
    ee.read_property(property, None).unwrap()
}

/// Everything an acknowledgment could change.
fn snapshot(ee: &EventEnrollmentObject) -> [PropertyValue; 4] {
    [
        PropertyIdentifier::EVENT_STATE,
        PropertyIdentifier::ACKED_TRANSITIONS,
        PropertyIdentifier::EVENT_TIME_STAMPS,
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
    ]
    .map(|property| read(ee, property))
}

fn set_detection(ee: &mut EventEnrollmentObject, enable: bool) {
    ee.write_property(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        None,
        PropertyValue::Boolean(enable),
        None,
    )
    .unwrap();
}

fn assert_no_alarm_configured(result: Result<Option<crate::event::EventStateChange>, Error>) {
    match result {
        Err(Error::Remote(RemoteError::Error {
            class: ErrorClass::OBJECT,
            code: ErrorCode::NO_ALARM_CONFIGURED,
        })) => {}
        other => panic!("expected OBJECT / NO_ALARM_CONFIGURED, got {other:?}"),
    }
}

#[test]
fn an_enabled_enrollment_takes_the_acknowledgment() {
    let mut ee = offnormal();
    ee.acknowledge_alarm(EventState::OFFNORMAL, &STAMP).unwrap();
    assert_eq!(
        read(&ee, PropertyIdentifier::ACKED_TRANSITIONS),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xE0],
        }
    );
}

#[test]
fn a_detection_disabled_enrollment_refuses_the_acknowledgment() {
    let mut ee = offnormal();
    set_detection(&mut ee, false);
    // Disabling reset the object to NORMAL with every transition acked.
    let disabled = snapshot(&ee);
    assert_eq!(
        disabled[0],
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
    assert_eq!(
        disabled[1],
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xE0],
        }
    );
    // Whatever the state or time stamp named, nothing is acknowledged.
    for (state, stamp) in [
        (EventState::OFFNORMAL, STAMP),
        (EventState::NORMAL, STAMP),
        (EventState::OFFNORMAL, BACnetTimeStamp::SequenceNumber(7)),
    ] {
        assert_no_alarm_configured(ee.acknowledge_alarm(state, &stamp));
        assert_eq!(snapshot(&ee), disabled);
    }
}

#[test]
fn the_refusal_lifts_once_detection_is_back() {
    let mut ee = offnormal();
    set_detection(&mut ee, false);
    assert_no_alarm_configured(ee.acknowledge_alarm(EventState::OFFNORMAL, &STAMP));

    set_detection(&mut ee, true);
    // The reset dropped the earlier OFFNORMAL transition, so its time stamp
    // matches nothing now, and nothing moves.
    let before = snapshot(&ee);
    match ee.acknowledge_alarm(EventState::OFFNORMAL, &STAMP) {
        Err(Error::Remote(RemoteError::Error {
            class: ErrorClass::SERVICES,
            code: ErrorCode::INVALID_TIMESTAMP,
        })) => {}
        other => panic!("expected SERVICES / INVALID_TIMESTAMP, got {other:?}"),
    }
    assert_eq!(snapshot(&ee), before);

    // A fresh transition after re-enabling is acknowledged.
    let fresh = BACnetTimeStamp::SequenceNumber(43);
    commit_offnormal(&mut ee, fresh.clone());
    ee.acknowledge_alarm(EventState::OFFNORMAL, &fresh).unwrap();
    assert_eq!(
        read(&ee, PropertyIdentifier::ACKED_TRANSITIONS),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xE0],
        }
    );
}

#[test]
fn an_enrollment_without_a_reference_refuses_the_acknowledgment() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_BITSTRING).unwrap();
    let before = snapshot(&ee);
    assert_no_alarm_configured(ee.acknowledge_alarm(EventState::OFFNORMAL, &STAMP));
    assert_eq!(snapshot(&ee), before);
}
