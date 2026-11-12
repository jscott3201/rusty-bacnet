//! Transition profiles consumed by the shared EventNotification sender.

use alloc::string::String;
use bacnet_objects::event::EventStateChange;
use bacnet_types::enums::EventType;
use bacnet_types::primitives::BACnetTimeStamp;

use super::payload::CommittedNotificationPayload;
use super::timestamp::SampledEventClock;

/// One exact transition-coordinate projection from object-owned event history.
#[derive(Debug, Clone, PartialEq)]
pub struct CommittedHistorySnapshot {
    /// The transition's Event_Time_Stamps entry.
    pub timestamp: BACnetTimeStamp,
    /// Its Event_Message_Texts entry, `None` when empty.
    pub message_text: Option<String>,
}

/// The source boundary for the notification's timestamp and message.
pub enum NotificationHistorySource {
    /// Non-intrinsic events and acknowledgment notices sample at distribution.
    SendTime,
    /// Atomic transitions use the exact history coordinate read after commit.
    Committed {
        /// What the commit wrote at the transition's coordinate.
        snapshot: CommittedHistorySnapshot,
        /// The Device clock the commit's timestamp came from.
        recipient_clock: SampledEventClock,
    },
}

/// Whether the sender builds an ordinary event or an acknowledgment notice.
pub enum NotificationConstruction {
    /// An ALARM or EVENT notification, by the object's Notify_Type.
    Event,
    /// An ACK_NOTIFICATION.
    Acknowledgment,
}

/// One transition to notify, as distribution takes it.
pub struct NotificationTransition {
    /// The states it moved between.
    pub change: EventStateChange,
    /// The notification's Event Type.
    pub event_type: EventType,
    /// Where its timestamp and message text come from.
    pub history_source: NotificationHistorySource,
    /// Ack_Required as committed; `None` reads it from the class.
    pub ack_required: Option<bool>,
    /// The Event Values, projected when it committed.
    pub event_values: Option<CommittedNotificationPayload>,
    /// An ordinary event or an acknowledgment.
    pub construction: NotificationConstruction,
}

impl NotificationTransition {
    /// The ACK_NOTIFICATION for an accepted acknowledgement of `change`.
    pub fn acknowledgment(change: EventStateChange, event_type: EventType) -> Self {
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

/// One intrinsic transition committed under the database write guard.
pub struct CommittedIntrinsicTransition {
    /// The states it moved between.
    pub change: EventStateChange,
    /// Its Event Type.
    pub event_type: EventType,
    /// Whether Event_Enable lets it be distributed (Clause 13.2.5).
    pub distribute: bool,
    /// The timestamp and message text it committed.
    pub history_snapshot: CommittedHistorySnapshot,
    /// The Device clock its timestamp came from.
    pub recipient_clock: SampledEventClock,
    /// The class's Ack_Required for its transition, as committed.
    pub ack_required: bool,
    /// Its Event Values; `None` when they don't project, and then it is
    /// committed but not distributed.
    pub event_values: Option<CommittedNotificationPayload>,
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
