//! Transition profiles consumed by the shared EventNotification sender.

use bacnet_objects::event::{EventStateChange, TransitionOutcome};
use bacnet_types::enums::EventType;
use bacnet_types::primitives::BACnetTimeStamp;

use super::super::event_notification_payload::CommittedNotificationPayload;
use super::super::event_timestamp::SampledEventClock;

/// One exact transition-coordinate projection from object-owned event history.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::server) struct CommittedHistorySnapshot {
    pub(in crate::server) timestamp: BACnetTimeStamp,
    pub(in crate::server) message_text: Option<String>,
}

pub(in crate::server) enum CommittedMessageProjection {
    RequiredProperty,
    IntentionallyAbsent,
}

/// The source boundary for the notification's timestamp and message.
pub(in crate::server) enum NotificationHistorySource {
    /// Legacy/default objects sample their timestamp when distribution begins.
    SendTime,
    /// Atomic transitions use the exact history coordinate read after commit.
    Committed {
        snapshot: CommittedHistorySnapshot,
        recipient_clock: SampledEventClock,
    },
}

/// Whether the sender builds an ordinary event or an acknowledgment notice.
pub(in crate::server) enum NotificationConstruction {
    Event,
    Acknowledgment,
}

pub(in crate::server) struct NotificationTransition {
    pub(in crate::server) change: EventStateChange,
    pub(in crate::server) event_type: EventType,
    pub(in crate::server) history_source: NotificationHistorySource,
    pub(in crate::server) ack_required: Option<bool>,
    pub(in crate::server) event_values: Option<CommittedNotificationPayload>,
    pub(in crate::server) construction: NotificationConstruction,
}

impl NotificationTransition {
    pub(in crate::server) fn acknowledgment(
        change: EventStateChange,
        event_type: EventType,
    ) -> Self {
        Self {
            change,
            event_type,
            history_source: NotificationHistorySource::SendTime,
            ack_required: Some(false),
            event_values: None,
            construction: NotificationConstruction::Acknowledgment,
        }
    }
}

impl From<(EventStateChange, EventType)> for NotificationTransition {
    fn from((change, event_type): (EventStateChange, EventType)) -> Self {
        Self {
            change,
            event_type,
            history_source: NotificationHistorySource::SendTime,
            ack_required: None,
            event_values: None,
            construction: NotificationConstruction::Event,
        }
    }
}

/// One built-in intrinsic transition committed under the database write guard.
pub(in crate::server) struct CommittedIntrinsicTransition {
    pub(in crate::server) change: EventStateChange,
    pub(in crate::server) event_type: EventType,
    pub(in crate::server) distribute: bool,
    pub(in crate::server) history_snapshot: CommittedHistorySnapshot,
    pub(in crate::server) recipient_clock: SampledEventClock,
    pub(in crate::server) ack_required: bool,
    pub(in crate::server) event_values: Option<CommittedNotificationPayload>,
}

impl From<CommittedIntrinsicTransition> for NotificationTransition {
    fn from(committed: CommittedIntrinsicTransition) -> Self {
        Self {
            change: committed.change,
            event_type: committed.event_type,
            history_source: NotificationHistorySource::Committed {
                snapshot: committed.history_snapshot,
                recipient_clock: committed.recipient_clock,
            },
            ack_required: Some(committed.ack_required),
            event_values: committed.event_values,
            construction: NotificationConstruction::Event,
        }
    }
}

/// A server-ready intrinsic outcome under its declared object contract.
///
/// Built-ins carry their atomic commit snapshot. Legacy implementations have
/// already mutated state during evaluation and retain send-time policy and
/// timestamp sampling.
pub(in crate::server) enum ResolvedIntrinsicTransition {
    Committed(CommittedIntrinsicTransition),
    Legacy(TransitionOutcome),
}

impl ResolvedIntrinsicTransition {
    pub(in crate::server) fn distribute(&self) -> bool {
        match self {
            Self::Committed(committed) => committed.distribute,
            Self::Legacy(outcome) => outcome.distribute,
        }
    }

    pub(in crate::server) fn can_emit(&self) -> bool {
        !matches!(self, Self::Committed(committed) if committed.event_values.is_none())
    }
}

impl From<ResolvedIntrinsicTransition> for NotificationTransition {
    fn from(transition: ResolvedIntrinsicTransition) -> Self {
        match transition {
            ResolvedIntrinsicTransition::Committed(committed) => committed.into(),
            ResolvedIntrinsicTransition::Legacy(outcome) => {
                (outcome.change, outcome.event_type).into()
            }
        }
    }
}
