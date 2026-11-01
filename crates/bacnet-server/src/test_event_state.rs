//! Seeding an object's Event_State for a test through the transition commit
//! the evaluator itself uses (#2179); no setter bypasses that commit.

use bacnet_objects::event::{EventStateChange, EventTransition, EventTransitionCommit};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{EventState, PropertyIdentifier};
use bacnet_types::primitives::{BACnetTimeStamp, PropertyValue};

/// Commit a transition from the Event_State `object` reads now to `to`;
/// `ack_required` is the Notification Class's answer for it.
pub(crate) fn commit_event_state(
    object: &mut dyn BACnetObject,
    to: EventState,
    ack_required: bool,
) {
    let from = match object.read_property(PropertyIdentifier::EVENT_STATE, None) {
        Ok(PropertyValue::Enumerated(raw)) => EventState::from_raw(raw),
        other => panic!("expected an Event_State, got {other:?}"),
    };
    object
        .event_reporting_mut()
        .expect("the object reports events")
        .commit_event_transition(EventTransitionCommit {
            change: EventStateChange { from, to },
            coordinate: EventTransition::for_target_state(to),
            ack_required,
            timestamp: BACnetTimeStamp::SequenceNumber(0),
            message_text: None,
        })
        .expect("the transition commits");
}
