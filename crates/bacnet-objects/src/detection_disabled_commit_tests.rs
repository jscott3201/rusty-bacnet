//! Every built-in `EventReporting` lender refuses a transition commit away
//! from NORMAL while its Event_Detection_Enable is FALSE (Clause 13.2.2.1,
//! #2271), leaving its event properties as they were, and takes the same
//! commit once detection is on. The table below sorts each lender by what
//! its commit does; a lender added to, or moved within, the catalogue fails
//! here until the table changes with it.

use crate::catalogue::representatives;
use crate::event::{
    EventStateChange, EventTransition, EventTransitionCommit, EventTransitionCommitError,
};
use crate::traits::BACnetObject;
use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use bacnet_types::enums::{EventState, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{BACnetTimeStamp, PropertyValue};

/// Lenders whose commit takes a target other than NORMAL, so the detection
/// gate decides it.
const GUARDED: [ObjectType; 12] = [
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

/// Lenders whose commit refuses every target other than NORMAL as
/// `Unsupported`, detection on or off: the logs (BUFFER_READY has only the
/// NORMAL state) and Alert Enrollment (no commit of its own).
const NEVER_LEAVE_NORMAL: [ObjectType; 4] = [
    ObjectType::TREND_LOG,
    ObjectType::TREND_LOG_MULTIPLE,
    ObjectType::EVENT_LOG,
    ObjectType::ALERT_ENROLLMENT,
];

fn to_fault() -> EventTransitionCommit {
    EventTransitionCommit {
        change: EventStateChange {
            from: EventState::NORMAL,
            to: EventState::FAULT,
        },
        coordinate: EventTransition::ToFault,
        ack_required: true,
        timestamp: BACnetTimeStamp::SequenceNumber(7),
        message_text: Some(String::from("fault")),
    }
}

/// The event properties a commit writes, as served, plus the coordinate of
/// the latest commit (which the served history may mask while detection
/// is off).
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
    let summary = object
        .event_reporting()
        .and_then(|reporting| reporting.enrollment_summary_capability())
        .map(|capability| capability.last_transition);
    rows.push(format!("{summary:?}"));
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

#[test]
fn every_lender_refuses_leaving_normal_while_detection_is_disabled() {
    let mut failures = Vec::new();
    let mut seen = BTreeSet::new();
    let disabled = representatives();
    let enabled = representatives();
    for (mut off, mut on) in disabled.into_iter().zip(enabled) {
        let object_type = off.object_identifier().object_type();
        if off.event_reporting().is_none() {
            continue;
        }
        seen.insert(object_type.to_raw());
        let guarded = GUARDED.contains(&object_type);
        assert!(
            guarded || NEVER_LEAVE_NORMAL.contains(&object_type),
            "{object_type:?} lends EventReporting but is in neither table"
        );

        set_detection(off.as_mut(), false);
        let before = snapshot(off.as_ref());
        let refused = off
            .event_reporting_mut()
            .unwrap()
            .commit_event_transition(to_fault());
        let expected = if guarded {
            EventTransitionCommitError::DetectionDisabled
        } else {
            EventTransitionCommitError::Unsupported
        };
        if refused != Err(expected) {
            failures.push(format!("{object_type:?} disabled: {refused:?}"));
        }
        if snapshot(off.as_ref()) != before {
            failures.push(format!("{object_type:?} disabled: event state changed"));
        }

        set_detection(on.as_mut(), true);
        let taken = on
            .event_reporting_mut()
            .unwrap()
            .commit_event_transition(to_fault());
        if guarded {
            let state = on.read_property(PropertyIdentifier::EVENT_STATE, None).ok();
            if taken.is_err()
                || state != Some(PropertyValue::Enumerated(EventState::FAULT.to_raw()))
            {
                failures.push(format!("{object_type:?} enabled: {taken:?}, {state:?}"));
            }
        } else if taken != Err(EventTransitionCommitError::Unsupported) {
            failures.push(format!("{object_type:?} enabled: {taken:?}"));
        }
    }
    for object_type in GUARDED.iter().chain(&NEVER_LEAVE_NORMAL) {
        assert!(
            seen.contains(&object_type.to_raw()),
            "{object_type:?} is not a catalogue lender"
        );
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn a_normal_target_passes_the_detection_gate() {
    // Disabling detection resets Event_State to NORMAL directly, so the
    // gate refuses only targets away from NORMAL; the kernel then judges a
    // NORMAL target on its own terms (here a stale FAULT source).
    let mut failures = Vec::new();
    for mut object in representatives() {
        let object_type = object.object_identifier().object_type();
        if !GUARDED.contains(&object_type) {
            continue;
        }
        set_detection(object.as_mut(), false);
        let result = object
            .event_reporting_mut()
            .unwrap()
            .commit_event_transition(EventTransitionCommit {
                change: EventStateChange {
                    from: EventState::FAULT,
                    to: EventState::NORMAL,
                },
                coordinate: EventTransition::ToNormal,
                ack_required: false,
                timestamp: BACnetTimeStamp::SequenceNumber(1),
                message_text: None,
            });
        let expected = Err(EventTransitionCommitError::CurrentStateMismatch {
            expected: EventState::FAULT,
            actual: EventState::NORMAL,
        });
        if result != expected {
            failures.push(format!("{object_type:?}: {result:?}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
