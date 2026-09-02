use super::*;

use crate::event::{EventStateChange, EventTransition, EventTransitionCommit, PendingTransition};

fn committed_input() -> AnalogInputObject {
    let mut object = AnalogInputObject::new(1, "AI-correlated", 62).unwrap();
    object
        .commit_event_transition_internal(EventTransitionCommit {
            change: EventStateChange {
                from: EventState::NORMAL,
                to: EventState::HIGH_LIMIT,
            },
            coordinate: EventTransition::ToOffnormal,
            ack_required: true,
            timestamp: BACnetTimeStamp::SequenceNumber(42),
            message_text: Some("high".into()),
        })
        .unwrap();
    object.event_detector.pending = Some(PendingTransition {
        state: EventState::NORMAL,
        remaining: 3,
    });
    object.event_detector.fault_reliability = Some(Reliability::OVER_RANGE.to_raw());
    object
}

fn assert_invalid_event_state(error: Error) {
    assert!(matches!(
        error,
        Error::Protocol { class, code }
            if class == ErrorClass::SERVICES.to_raw() as u32
                && code == ErrorCode::INVALID_EVENT_STATE.to_raw() as u32
    ));
}

#[test]
fn rejected_correlation_preserves_detector_pending_and_all_event_history() {
    let mut object = committed_input();
    let before = (
        object.event_detector.event_state,
        object.event_detector.acked_transitions,
        object.event_detector.pending.clone(),
        object.event_detector.fault_reliability,
        object.event_history.clone(),
    );

    let error = object
        .acknowledge_alarm_correlated_internal(
            EventState::LOW_LIMIT,
            &BACnetTimeStamp::SequenceNumber(99),
        )
        .unwrap_err();

    assert_invalid_event_state(error);
    assert_eq!(
        (
            object.event_detector.event_state,
            object.event_detector.acked_transitions,
            object.event_detector.pending,
            object.event_detector.fault_reliability,
            object.event_history,
        ),
        before
    );
}

#[test]
fn idempotent_valid_correlation_changes_no_detector_or_history_state() {
    let mut object = committed_input();
    object.event_detector.acked_transitions |= EventTransition::ToOffnormal.bit_mask();
    let before = (
        object.event_detector.event_state,
        object.event_detector.acked_transitions,
        object.event_detector.pending.clone(),
        object.event_detector.fault_reliability,
        object.event_history.clone(),
    );

    object
        .acknowledge_alarm_correlated_internal(
            EventState::HIGH_LIMIT,
            &BACnetTimeStamp::SequenceNumber(42),
        )
        .unwrap();

    assert_eq!(
        (
            object.event_detector.event_state,
            object.event_detector.acked_transitions,
            object.event_detector.pending,
            object.event_detector.fault_reliability,
            object.event_history,
        ),
        before
    );
}
