use crate::event::history::{EventHistory, EventTransitionState};
use crate::event::{EventTransitionCommit, EventTransitionCommitError};
use bacnet_types::enums::{EventState, Reliability};

/// All object-owned values needed to commit Event Enrollment Reliability.
///
/// A transition is present whenever a Reliability observation indicates a
/// FAULT entry, re-entry, or recovery. Keeping it optional also permits an
/// idempotent Reliability-only commit without fabricating event history.
#[derive(Debug, Clone, PartialEq)]
pub struct EventEnrollmentReliabilityCommit {
    /// Desired value of the Event Enrollment object's Reliability property.
    pub reliability: Reliability,
    /// Event transition to commit atomically with Reliability, when indicated.
    pub transition: Option<EventTransitionCommit>,
}

pub(super) fn commit_event_transition(
    event_state: &mut u32,
    acked_transitions: &mut u8,
    event_history: &mut EventHistory,
    commit: EventTransitionCommit,
) -> Result<(), EventTransitionCommitError> {
    // Event Enrollment stores Event_State as its wire enumeration. Stage
    // typed and cloned locals so validation and kernel mutation complete
    // before any object-owned field changes.
    let mut typed_event_state = EventState::from_raw(*event_state);
    let mut staged_acknowledgments = *acked_transitions;
    let mut staged_history = event_history.clone();
    EventTransitionState::new(
        &mut typed_event_state,
        &mut staged_acknowledgments,
        &mut staged_history,
    )
    .commit(commit)?;

    *event_state = typed_event_state.to_raw();
    *acked_transitions = staged_acknowledgments;
    *event_history = staged_history;
    Ok(())
}

pub(super) fn commit_reliability(
    reliability: &mut u32,
    event_state: &mut u32,
    acked_transitions: &mut u8,
    event_history: &mut EventHistory,
    commit: EventEnrollmentReliabilityCommit,
) -> Result<(), EventTransitionCommitError> {
    // Reliability is staged alongside the three event-transition stores. A
    // rejected transition therefore leaves all four properties unchanged.
    let staged_reliability = commit.reliability.to_raw();
    let mut typed_event_state = EventState::from_raw(*event_state);
    let mut staged_acknowledgments = *acked_transitions;
    let mut staged_history = event_history.clone();

    if let Some(transition) = commit.transition {
        EventTransitionState::new(
            &mut typed_event_state,
            &mut staged_acknowledgments,
            &mut staged_history,
        )
        .commit(transition)?;
    }

    *reliability = staged_reliability;
    *event_state = typed_event_state.to_raw();
    *acked_transitions = staged_acknowledgments;
    *event_history = staged_history;
    Ok(())
}

macro_rules! impl_event_enrollment_transition_commit {
    () => {
        fn commit_event_transition_internal(
            &mut self,
            commit: EventTransitionCommit,
        ) -> Result<(), EventTransitionCommitError> {
            transition::commit_event_transition(
                &mut self.event_state,
                &mut self.acked_transitions,
                &mut self.event_history,
                commit,
            )
        }

        fn commit_event_enrollment_reliability_internal(
            &mut self,
            commit: $crate::event_enrollment::EventEnrollmentReliabilityCommit,
        ) -> Result<(), EventTransitionCommitError> {
            transition::commit_reliability(
                &mut self.reliability,
                &mut self.event_state,
                &mut self.acked_transitions,
                &mut self.event_history,
                commit,
            )
        }
    };
}

pub(super) use impl_event_enrollment_transition_commit;
