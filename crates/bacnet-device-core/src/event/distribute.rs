//! Notification Class distribution less the send (Clauses 12.21, 13.2.5):
//! a transition's notification, its priority and Ack_Required, the
//! recipients whose days, window and transitions match, and the record the
//! device's Event Log objects keep.

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::EventTransition;
use bacnet_objects::notification_class::{
    RecipientLookupOutcome, lookup_notification_recipients, resolve_transition_priority_ack,
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

/// Build the EventNotification for one transition under the database
/// guard, select its recipients and log it to the device's Event Log
/// objects. `fallback` is the day and time the recipient filter uses when
/// the Device clock gives none.
pub fn distribute_event(
    db: &mut ObjectDatabase,
    device: ObjectIdentifier,
    oid: &ObjectIdentifier,
    transition: NotificationTransition,
    fallback: (DaysOfWeek, Time),
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

    let (today, current_time) = match recipient_clock {
        SampledEventClock::Valid(clock_frame) => (
            clock_frame
                .day_of_week()
                .expect("validated ClockFrame has a day of week"),
            clock_frame.local_time,
        ),
        SampledEventClock::Unavailable | SampledEventClock::Invalid => fallback,
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

    let lookup =
        lookup_notification_recipients(db, notification_class, transition, today, &current_time);
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
