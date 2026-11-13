//! Notification Class distribution less the send (Clauses 12.21, 13.2.5):
//! a transition's notification, its priority and Ack_Required, the
//! recipients whose days, window and transitions match, and the record the
//! device's Event Log objects keep.

use bacnet_encoding::npdu::view::NetworkPriority;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::EventTransition;
use bacnet_objects::notification_class::{
    RecipientLookupOutcome, lookup_notification_recipients_at, resolve_transition_priority_ack,
};
use bacnet_services::alarm_event::EventNotificationRequest;
use bacnet_types::bitstring::DaysOfWeek;
use bacnet_types::enums::{NotifyType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, Time};

use super::{
    CommittedNotificationPayload, NotificationConstruction, NotificationHistorySource,
    NotificationTransition, SampledEventClock, sample_event_timestamp,
};
/// What [`distribute_event`] made of one transition, under the caller's
/// database guard.
pub struct EventDistribution {
    /// The Device clock the recipient filter read. Anything but
    /// [`SampledEventClock::Valid`] means the filter used the fallback.
    pub clock: SampledEventClock,
    /// Whether a notification was built, and for whom.
    pub outcome: DistributionOutcome,
}

/// The end of one [`distribute_event`] call.
pub enum DistributionOutcome {
    /// The event object is gone.
    Gone,
    /// The Notification Class lookup failed closed: nothing was built or
    /// logged, and the transition goes to nobody.
    Refused {
        /// The class the object names.
        notification_class: u32,
        /// The transition's coordinate.
        transition: EventTransition,
        /// Why the lookup failed.
        lookup: RecipientLookupOutcome,
    },
    /// Built and logged to the Event Logs. `lookup` holds the recipients
    /// ([`RecipientLookupOutcome::Matched`]) or says why there are none.
    Built {
        /// The notification, Process Identifier 0.
        notification: EventNotificationRequest,
        /// The class the object names.
        notification_class: u32,
        /// The transition's coordinate.
        transition: EventTransition,
        /// The selection.
        lookup: RecipientLookupOutcome,
    },
}

/// The local day and time a Notification Class destination's window is
/// judged at (Clause 12.21; §6.3 of the design).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipientWindow {
    /// The Device clock's frame when it holds a valid date and time,
    /// otherwise `fallback`: what the server does, its fallback the system
    /// clock's UTC day and time.
    Frame {
        /// The day and time when the frame gives none; `None` fails closed.
        fallback: Option<(DaysOfWeek, Time)>,
    },
    /// This day and time, whatever the frame reads: a firmware clock's
    /// [`window`](crate::clock::SyncedClock::window), `None` until it is set.
    /// An unsynchronized `DeviceClock` serves a valid 1990 frame, which must
    /// not open a window (decision 14).
    Given(Option<(DaysOfWeek, Time)>),
}

/// Build the EventNotification for one transition under the database
/// guard, select its recipients and log it to the device's Event Log
/// objects. `window` says the day and time the recipients' windows are
/// judged at; without one, only a destination open every day, all day, is
/// selected (fail closed).
pub fn distribute_event(
    db: &mut ObjectDatabase,
    device: ObjectIdentifier,
    oid: &ObjectIdentifier,
    transition: NotificationTransition,
    window: RecipientWindow,
) -> EventDistribution {
    let NotificationTransition {
        change,
        event_type,
        history_source,
        ack_required: ack_required_snapshot,
        event_values,
        construction,
    } = transition;

    let (timestamp, message_text, recipient_clock) = match history_source {
        NotificationHistorySource::SendTime => {
            let sample = sample_event_timestamp(db);
            (sample.timestamp, None, sample.clock)
        }
        NotificationHistorySource::Committed {
            snapshot,
            recipient_clock,
        } => (snapshot.timestamp, snapshot.message_text, recipient_clock),
    };

    let at = match (window, recipient_clock) {
        (RecipientWindow::Given(at), _) => at,
        (RecipientWindow::Frame { .. }, SampledEventClock::Valid(clock_frame)) => Some((
            clock_frame
                .day_of_week()
                .expect("validated ClockFrame has a day of week"),
            clock_frame.local_time,
        )),
        (RecipientWindow::Frame { fallback }, _) => fallback,
    };
    let distribution = |outcome| EventDistribution {
        clock: recipient_clock,
        outcome,
    };

    let object = match db.get_mut(oid) {
        Some(o) => o,
        None => return distribution(DistributionOutcome::Gone),
    };

    let notification_class = object
        .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
        .ok()
        .and_then(|v| match v {
            PropertyValue::Unsigned(n) => Some(n as u32),
            _ => None,
        })
        .unwrap_or(0);

    let notify_type = match construction {
        NotificationConstruction::Acknowledgment => NotifyType::ACK_NOTIFICATION,
        NotificationConstruction::Event => object
            .read_property(PropertyIdentifier::NOTIFY_TYPE, None)
            .ok()
            .and_then(|v| match v {
                PropertyValue::Enumerated(n) => Some(NotifyType::from_raw(n)),
                _ => None,
            })
            .unwrap_or(NotifyType::ALARM),
    };

    let transition = change.transition();

    // Resolve the per-transition Priority and Ack_Required from the
    // referenced NotificationClass (ASHRAE 135-2020 §13.2.1), falling
    // back to the BACnet defaults (Priority 255, no ack) when the
    // class is missing.
    let (priority, resolved_ack_required) =
        resolve_transition_priority_ack(db, notification_class, transition);
    let ack_required = ack_required_snapshot.unwrap_or(resolved_ack_required);

    let lookup = lookup_notification_recipients_at(db, notification_class, transition, at);
    if matches!(
        lookup,
        RecipientLookupOutcome::NotificationClassMissing
            | RecipientLookupOutcome::RecipientListUnavailable
            | RecipientLookupOutcome::RecipientListInvalid
            | RecipientLookupOutcome::RecipientListTooLong
    ) {
        return distribution(DistributionOutcome::Refused {
            notification_class,
            transition,
            lookup,
        });
    }

    let notification = EventNotificationRequest {
        process_identifier: 0,
        initiating_device_identifier: device,
        event_object_identifier: *oid,
        timestamp,
        notification_class,
        priority,
        event_type,
        message_text,
        notify_type,
        // ack_required is only meaningful for ALARM/EVENT notify types
        // (ACK_NOTIFICATION omits the field on the wire). Per §13.2.1 the
        // value is the NotificationClass's per-transition Ack_Required,
        // not a function of Notify_Type alone.
        ack_required: if notify_type == NotifyType::ACK_NOTIFICATION {
            false
        } else {
            ack_required
        },
        from_state: change.from,
        to_state: change.to,
        event_values: if notify_type == NotifyType::ACK_NOTIFICATION {
            None
        } else {
            event_values.map(CommittedNotificationPayload::into_parameters)
        },
    };

    db.log_event_notification(&notification);
    distribution(DistributionOutcome::Built {
        notification,
        notification_class,
        transition,
        lookup,
    })
}

/// The network priority Table 13-6 gives an event notification of event
/// priority `priority` (Clause 13.2.5.4): 0 to 63 life safety, 64 to 127
/// critical equipment, 128 to 191 urgent, the rest normal.
pub fn network_priority(priority: u8) -> NetworkPriority {
    match priority {
        0..=63 => NetworkPriority::LifeSafety,
        64..=127 => NetworkPriority::CriticalEquipment,
        128..=191 => NetworkPriority::Urgent,
        192..=255 => NetworkPriority::Normal,
    }
}
