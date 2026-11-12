//! The intrinsic commit (Clause 13.2.2.1.4): a transition's Event_State,
//! Acked_Transitions, Event_Time_Stamps and message text commit at once,
//! before anyone asks whether to send it, and the committed history is read
//! back for the notification.

use alloc::string::String;
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::{
    EventTransition, EventTransitionCommit, EventTransitionCommitError, TransitionOutcome,
};
use bacnet_objects::notification_class::resolve_transition_priority_ack;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::{
    CommittedHistorySnapshot, CommittedIntrinsicTransition, confirm_event_timestamp,
    intrinsic_event_message_text, project_intrinsic_payload, stage_event_timestamp,
};

/// Whether a committed projection needs Event_Message_Texts.
#[derive(Clone, Copy)]
pub enum MessageTexts {
    /// Intrinsic reporters serve it; failing to read it fails closed.
    Required,
    /// Event Enrollment serves it as an optional row (Table 12-14). An object
    /// that doesn't (UNKNOWN_PROPERTY) sends no message text; any other
    /// failure, or a value that isn't a CharacterString, still fails closed.
    Optional,
}

/// Read one exact committed transition coordinate through the object contract.
///
/// Required properties are projected while the caller still owns the database
/// write guard. A malformed or incomplete projection is not equivalent to a
/// missing message/timestamp: the committed transition remains local, but no
/// outward frame can be built from an unproven history snapshot. An empty
/// Event_Message_Texts entry sends no message text: an Event Enrollment
/// commits one unless Event_Message_Texts_Config gives it a text (#1507).
pub fn read_committed_history_snapshot(
    object: &dyn BACnetObject,
    coordinate: EventTransition,
    message_texts: MessageTexts,
) -> Option<CommittedHistorySnapshot> {
    let array_index = u32::try_from(coordinate.index() + 1)
        .expect("three event transition coordinates fit in u32");
    let PropertyValue::ApplicationData(encoded_timestamp) = object
        .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, Some(array_index))
        .ok()?
    else {
        return None;
    };
    let (timestamp, consumed) = decode_timestamp_choice(&encoded_timestamp, 0).ok()?;
    if consumed != encoded_timestamp.len() {
        return None;
    }

    let message_text = match (
        object.read_property(PropertyIdentifier::EVENT_MESSAGE_TEXTS, Some(array_index)),
        message_texts,
    ) {
        (Ok(PropertyValue::CharacterString(message_text)), _) => message_text,
        (
            Err(Error::Remote(RemoteError::Error {
                class: ErrorClass::PROPERTY,
                code: ErrorCode::UNKNOWN_PROPERTY,
            })),
            MessageTexts::Optional,
        ) => String::new(),
        _ => return None,
    };

    Some(CommittedHistorySnapshot {
        timestamp,
        message_text: (!message_text.is_empty()).then_some(message_text),
    })
}

/// Why [`commit_intrinsic_transition`] committed nothing to distribute.
#[derive(Debug)]
pub enum IntrinsicCommitRefusal {
    /// The object is gone.
    Gone,
    /// The object refused the commit; its event state is unchanged.
    Rejected(EventTransitionCommitError),
    /// The transition committed, but its timestamp or message text at
    /// `coordinate` doesn't read back, so no notification can be built.
    HistoryUnreadable(EventTransition),
}

/// Resolve policy and atomically commit one intrinsic proposal.
///
/// A committed transition whose Event Values don't project comes back with
/// `event_values` empty: committed, but not to be distributed.
pub fn commit_intrinsic_transition(
    db: &mut ObjectDatabase,
    oid: &ObjectIdentifier,
    outcome: TransitionOutcome,
) -> Result<CommittedIntrinsicTransition, IntrinsicCommitRefusal> {
    let coordinate = outcome.change.transition();
    let notification_class = db
        .get(oid)
        .ok_or(IntrinsicCommitRefusal::Gone)?
        .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
        .ok()
        .and_then(|value| match value {
            PropertyValue::Unsigned(number) => Some(number as u32),
            _ => None,
        })
        .unwrap_or(0);
    let (_, ack_required) = resolve_transition_priority_ack(db, notification_class, coordinate);
    let staged_timestamp = stage_event_timestamp(db);
    let message_text = intrinsic_event_message_text(oid, &outcome.change);
    let commit = EventTransitionCommit {
        change: outcome.change.clone(),
        coordinate,
        ack_required,
        timestamp: staged_timestamp.sample.timestamp.clone(),
        message_text: Some(message_text),
    };

    db.get_mut(oid)
        .ok_or(IntrinsicCommitRefusal::Gone)?
        .event_reporting_mut()
        .map_or(Err(EventTransitionCommitError::Unsupported), |reporting| {
            reporting.commit_event_transition(commit)
        })
        .map_err(IntrinsicCommitRefusal::Rejected)?;

    let recipient_clock = confirm_event_timestamp(db, staged_timestamp).clock;
    let history_snapshot = db
        .get(oid)
        .and_then(|object| {
            read_committed_history_snapshot(object, coordinate, MessageTexts::Required)
        })
        .ok_or(IntrinsicCommitRefusal::HistoryUnreadable(coordinate))?;
    let event_type = outcome.change.event_type(outcome.event_type);
    let device = db.local_device().identifier();
    let event_values = db
        .get(oid)
        .and_then(|object| project_intrinsic_payload(object, &outcome.change, event_type, device));
    Ok(CommittedIntrinsicTransition {
        change: outcome.change,
        event_type,
        distribute: outcome.distribute,
        history_snapshot,
        recipient_clock,
        ack_required,
        event_values,
    })
}
