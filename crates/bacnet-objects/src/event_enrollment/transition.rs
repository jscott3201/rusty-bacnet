use crate::event::history::{EventHistory, EventTransitionState};
use crate::event::{EventTransitionCommit, EventTransitionCommitError};
use bacnet_types::enums::EventState;

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
    };
}

pub(super) use impl_event_enrollment_transition_commit;
