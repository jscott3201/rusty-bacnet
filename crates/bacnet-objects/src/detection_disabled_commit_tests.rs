//! Every built-in `EventReporting` lender with a commit path refuses every
//! transition commit while its Event_Detection_Enable is FALSE, NORMAL
//! targets included (Clause 13.2.2.1, #2271), leaving its event properties
//! as they were, and takes the same commit once detection is on. The tables
//! below sort each lender by the targets its commit takes; a lender added
//! to, or moved within, the catalogue fails here until they change with it.

use crate::catalogue::representatives;
use crate::event::{
    EventStateChange, EventTransition, EventTransitionCommit, EventTransitionCommitError,
};
use crate::traits::BACnetObject;
use alloc::boxed::Box;
#[cfg(feature = "event-reporting")]
use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use bacnet_types::enums::{EventState, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{BACnetTimeStamp, PropertyValue};

#[cfg(feature = "event-reporting")]
/// Lenders whose commit takes any target.
const ANY_TARGET: [ObjectType; 12] = [
    ObjectType::ANALOG_INPUT,
    ObjectType::ANALOG_OUTPUT,
    ObjectType::ANALOG_VALUE,
    ObjectType::BINARY_INPUT,
    ObjectType::BINARY_OUTPUT,
    ObjectType::BINARY_VALUE,
    ObjectType::MULTI_STATE_INPUT,
    ObjectType::MULTI_STATE_OUTPUT,
    ObjectType::MULTI_STATE_VALUE,
    ObjectType::ACCESS_ZONE,
    ObjectType::ACCESS_DOOR,
    ObjectType::EVENT_ENROLLMENT,
];

#[cfg(feature = "event-reporting")]
/// The logs: BUFFER_READY has only the NORMAL state, so any other target is
/// `Unsupported`, detection on or off.
const NORMAL_ONLY: [ObjectType; 3] = [
    ObjectType::TREND_LOG,
    ObjectType::TREND_LOG_MULTIPLE,
    ObjectType::EVENT_LOG,
];

/// Alert Enrollment lends no commit: always `Unsupported`.
const NO_COMMIT: [ObjectType; 1] = [ObjectType::ALERT_ENROLLMENT];

/// A commit from NORMAL to `to`, unacknowledged and with a message, so a
/// commit that slips through moves Acked_Transitions, a time stamp and a
/// message text.
fn from_normal(to: EventState) -> EventTransitionCommit {
    EventTransitionCommit {
        change: EventStateChange {
            from: EventState::NORMAL,
            to,
        },
        coordinate: EventTransition::for_target_state(to),
        ack_required: true,
        timestamp: BACnetTimeStamp::SequenceNumber(7),
        message_text: Some(String::from("slipped through")),
    }
}

/// The event properties a commit writes, as served, plus what a commit
/// records beside them: the latest coordinate (GetEnrollmentSummary's) and,
/// on a log, the last buffer-ready report.
fn snapshot(object: &dyn BACnetObject) -> Vec<String> {
    let mut rows: Vec<String> = [
        PropertyIdentifier::EVENT_STATE,
        PropertyIdentifier::ACKED_TRANSITIONS,
        PropertyIdentifier::EVENT_TIME_STAMPS,
        PropertyIdentifier::EVENT_MESSAGE_TEXTS,
    ]
    .iter()
    .map(|&property| format!("{:?}", object.read_property(property, None)))
    .collect();
    let reporting = object.event_reporting();
    let summary = reporting
        .and_then(|reporting| reporting.enrollment_summary_capability())
        .map(|capability| capability.last_transition);
    let report = reporting.and_then(|reporting| reporting.buffer_ready_report());
    rows.push(format!("{summary:?} {report:?}"));
    rows
}

/// Write Event_Detection_Enable as a network client would, and read it back.
/// The fresh Binary Output starts with detection off, so both branches set it.
fn set_detection(object: &mut dyn BACnetObject, enabled: bool) {
    let object_type = object.object_identifier().object_type();
    object
        .write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(enabled),
            None,
        )
        .unwrap_or_else(|error| panic!("{object_type:?}: Event_Detection_Enable: {error:?}"));
    assert_eq!(
        object
            .read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
            .ok(),
        Some(PropertyValue::Boolean(enabled)),
        "{object_type:?}"
    );
}

/// The lender types in a fresh catalogue, each paired with its object.
fn lenders() -> Vec<(ObjectType, Box<dyn BACnetObject>)> {
    representatives()
        .into_iter()
        .filter(|object| object.event_reporting().is_some())
        .map(|object| (object.object_identifier().object_type(), object))
        .collect()
}

#[cfg(feature = "event-reporting")]
#[test]
fn every_lender_refuses_every_commit_while_detection_is_disabled() {
    let mut failures = Vec::new();
    let mut seen = BTreeSet::new();
    for target in [EventState::FAULT, EventState::NORMAL] {
        for ((object_type, mut off), (_, mut on)) in lenders().into_iter().zip(lenders()) {
            seen.insert(object_type.to_raw());
            let takes = if ANY_TARGET.contains(&object_type) {
                true
            } else if NORMAL_ONLY.contains(&object_type) {
                target == EventState::NORMAL
            } else {
                assert!(
                    NO_COMMIT.contains(&object_type),
                    "{object_type:?} lends EventReporting but is in no table"
                );
                false
            };
            let unsupported = Err(EventTransitionCommitError::Unsupported);

            set_detection(off.as_mut(), false);
            let before = snapshot(off.as_ref());
            let refused = off
                .event_reporting_mut()
                .unwrap()
                .commit_event_transition(from_normal(target));
            let expected = if takes {
                Err(EventTransitionCommitError::DetectionDisabled)
            } else {
                unsupported
            };
            if refused != expected {
                failures.push(format!("{object_type:?} off, to {target}: {refused:?}"));
            }
            if snapshot(off.as_ref()) != before {
                failures.push(format!("{object_type:?} off, to {target}: state changed"));
            }

            set_detection(on.as_mut(), true);
            let taken = on
                .event_reporting_mut()
                .unwrap()
                .commit_event_transition(from_normal(target));
            let state = on.read_property(PropertyIdentifier::EVENT_STATE, None).ok();
            let landed = taken.is_ok()
                && state == Some(PropertyValue::Enumerated(target.to_raw()))
                && snapshot(on.as_ref()) != snapshot(off.as_ref());
            if takes != landed || (!takes && taken != unsupported) {
                failures.push(format!(
                    "{object_type:?} on, to {target}: {taken:?}, {state:?}"
                ));
            }
        }
    }
    for object_type in ANY_TARGET.iter().chain(&NORMAL_ONLY).chain(&NO_COMMIT) {
        assert!(
            seen.contains(&object_type.to_raw()),
            "{object_type:?} is not a catalogue lender"
        );
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn a_normal_to_normal_commit_is_refused_and_changes_nothing() {
    // The reviewer's probe: with detection off, NORMAL to NORMAL needs no
    // state change, yet an accepted one would still clear its
    // Acked_Transitions bit and stamp its time and message.
    let mut failures = Vec::new();
    for (object_type, mut object) in lenders() {
        if NO_COMMIT.contains(&object_type) {
            continue;
        }
        set_detection(object.as_mut(), false);
        let before = snapshot(object.as_ref());
        let result = object
            .event_reporting_mut()
            .unwrap()
            .commit_event_transition(from_normal(EventState::NORMAL));
        if result != Err(EventTransitionCommitError::DetectionDisabled)
            || snapshot(object.as_ref()) != before
        {
            failures.push(format!("{object_type:?}: {result:?}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
