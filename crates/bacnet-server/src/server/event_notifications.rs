use super::event_message_policy::intrinsic_event_message_text;
use super::event_notification_payload::{project_intrinsic_payload, CommittedNotificationPayload};
#[path = "event_notification_profile.rs"]
mod profile;
use self::profile::{CommittedHistorySnapshot, NotificationConstruction, NotificationTransition};
pub(super) use self::profile::{CommittedIntrinsicTransition, NotificationHistorySource};
use super::event_recipient_route::system_utc_recipient_filter_time;
use super::event_timestamp::{
    confirm_event_timestamp, sample_event_timestamp, stage_event_timestamp, SampledEventClock,
};
use super::*;
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_objects::event::{EventTransition, EventTransitionCommit, TransitionOutcome};
use bacnet_objects::traits::BACnetObject;

use crate::event_enrollment::{CommittedEventEnrollmentDelivery, CommittedEventEnrollmentResult};

#[path = "event_recipient_lookup.rs"]
mod recipient_lookup;
use recipient_lookup::matched_recipients_or_log;

/// Whether a committed projection needs Event_Message_Texts.
#[derive(Clone, Copy)]
enum MessageTexts {
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
fn read_committed_history_snapshot(
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
            Err(Error::Protocol {
                class: ErrorClass::PROPERTY,
                code: ErrorCode::UNKNOWN_PROPERTY,
            }),
            MessageTexts::Optional,
        ) => String::new(),
        _ => return None,
    };

    Some(CommittedHistorySnapshot {
        timestamp,
        message_text: (!message_text.is_empty()).then_some(message_text),
    })
}

/// Project an already-committed Event Enrollment result without another
/// transition commit or timestamp sample.
pub(super) fn resolve_committed_event_enrollment_transition(
    db: &ObjectDatabase,
    committed: CommittedEventEnrollmentDelivery,
) -> Option<(ObjectIdentifier, bool, NotificationTransition)> {
    let CommittedEventEnrollmentDelivery {
        result,
        ack_required,
        recipient_clock,
        event_type,
        event_values,
    } = committed;
    let (oid, change, distribute) = match result {
        CommittedEventEnrollmentResult::Normal(result) => {
            (result.enrollment_oid, result.change, result.distribute)
        }
        CommittedEventEnrollmentResult::Reliability(result) => {
            let change = result.state_change.clone()?;
            (result.enrollment_oid, change, result.distribute)
        }
    };

    let coordinate = change.transition();
    let history_snapshot = db.get(&oid).and_then(|object| {
        read_committed_history_snapshot(object, coordinate, MessageTexts::Optional)
    })?;
    Some((
        oid,
        distribute,
        NotificationTransition {
            change,
            event_type,
            history_source: NotificationHistorySource::Committed {
                snapshot: history_snapshot,
                recipient_clock,
            },
            ack_required: Some(ack_required),
            event_values: Some(event_values),
            construction: NotificationConstruction::Event,
        },
    ))
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Resolve policy and atomically commit one intrinsic proposal.
    pub(super) fn commit_intrinsic_transition(
        db: &mut ObjectDatabase,
        oid: &ObjectIdentifier,
        outcome: TransitionOutcome,
    ) -> Option<CommittedIntrinsicTransition> {
        let coordinate = outcome.change.transition();
        let notification_class = db
            .get(oid)?
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

        if let Err(error) = db.get_mut(oid)?.commit_event_transition_internal(commit) {
            debug!(%oid, ?error, "Intrinsic transition commit rejected");
            return None;
        }

        let recipient_clock = confirm_event_timestamp(db, staged_timestamp).clock;
        let history_snapshot = match db.get(oid).and_then(|object| {
            read_committed_history_snapshot(object, coordinate, MessageTexts::Required)
        }) {
            Some(snapshot) => snapshot,
            None => {
                debug!(
                    %oid,
                    ?coordinate,
                    "Committed intrinsic history projection rejected; suppressing distribution"
                );
                return None;
            }
        };
        let event_type = outcome.change.event_type(outcome.event_type);
        let device = db.local_device().identifier();
        let event_values = db
            .get(oid)
            .and_then(|object| {
                project_intrinsic_payload(object, &outcome.change, event_type, device)
            })
            .or_else(|| {
                debug!(
                    %oid,
                    ?event_type,
                    "Committed intrinsic Event Values projection rejected; suppressing distribution"
                );
                None
            });
        Some(CommittedIntrinsicTransition {
            change: outcome.change,
            event_type,
            distribute: outcome.distribute,
            history_snapshot,
            recipient_clock,
            ack_required,
            event_values,
        })
    }

    /// Evaluate intrinsic reporting on an object and send event notifications
    /// to the recipients the object's NotificationClass names.
    /// DCC gates network-message initiation (Clause 16.1), not the local
    /// transition actions in Clause 13.2.2.1.4. The outbound sender below
    /// suppresses distribution while communications are disabled.
    ///
    /// This is the per-write entry point: it probes the detector, which fires
    /// immediately only when `Time_Delay == 0`. For a nonzero delay the probe
    /// seeds a pending transition (returning `None`, so no notification is
    /// sent here) and the one-second [`intrinsic_reporting_task`](Self::start)
    /// advances the countdown and sends the notification on expiry.
    ///
    /// A caller owes the object a COV fanout afterwards, as the periodic task
    /// gives one, since a proposal can commit (changing Status_Flags) even
    /// when its projection is refused.
    pub(super) async fn fire_event_notifications_with_bindings(
        ctx: &EventDelivery<'_, T>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        oid: &ObjectIdentifier,
    ) {
        let db = ctx.db;
        let resolved = {
            let mut db = db.write().await;
            // Event_Algorithm_Inhibit follows its reference as of now (#1329).
            db.follow_event_algorithm_inhibit(oid);
            let outcome = db
                .get_mut(oid)
                .and_then(|object| object.evaluate_intrinsic_reporting());
            let resolved = outcome
                .and_then(|outcome| Self::commit_intrinsic_transition(&mut db, oid, outcome));
            // A committed transition changes Status_Flags: capture it at its
            // own time for timestamped COV-multiple references.
            if resolved.is_some() {
                let capture = cov_table.read().await.timed_capture(*oid);
                capture.run(&db);
            }
            resolved
        };

        // Local transition actions commit before Event_Enable or DCC can suppress
        // external distribution (Clauses 13.2.2.1.4 and 13.2.5).
        if let Some(resolved) = resolved {
            if resolved.distribute && resolved.event_values.is_some() {
                Self::build_and_send_event_notification_with_bindings(ctx, oid, resolved).await;
            }
        }
    }

    /// Run the per-write evaluation on every object a confirmed service
    /// wrote. Each such service (WriteProperty, WritePropertyMultiple,
    /// AddListElement and RemoveListElement) has already put every object it
    /// wrote, Life Safety objects aside, in the COV fanout that runs after
    /// this, so a committed transition's Status_Flags change reaches
    /// subscribers.
    pub(super) async fn fire_written_event_notifications(
        ctx: &EventDelivery<'_, T>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        written_oids: &[ObjectIdentifier],
    ) {
        for oid in written_oids {
            Self::fire_event_notifications_with_bindings(ctx, cov_table, oid).await;
        }
    }

    /// Build an `EventNotificationRequest` for a pre-computed transition and
    /// send it to the recipients the object's NotificationClass names.
    ///
    /// Shared by the per-write path and the
    /// periodic `Time_Delay` confirmation path, so both emit identical
    /// notifications. Skipped while DCC restricts initiation. Re-reads
    /// `Notification_Class` / `Notify_Type` under a brief `db.write()` guard,
    /// then drops the lock before any network send.
    ///
    /// Every notification built here also goes to the device's Event Log
    /// objects ([`ObjectDatabase::log_event_notification`]), under the build
    /// guard and before the network send, with Process Identifier 0, the value
    /// it has before a recipient's own is filled in. It is logged when the
    /// Notification Class selects nobody, since Clause 13.2.5 keeps the
    /// Recipient_List out of distribution to local objects, but not when the
    /// recipient lookup fails closed: that transition is refused whole, and a
    /// record would carry a priority and ack policy the class never gave.
    /// Event_Enable and DCC, which stop the notification being built, keep it
    /// out of the logs too, a local choice. Received notifications never come
    /// here.
    pub(super) async fn build_and_send_event_notification_with_bindings(
        ctx: &EventDelivery<'_, T>,
        oid: &ObjectIdentifier,
        transition: impl Into<NotificationTransition>,
    ) {
        let &EventDelivery {
            db,
            comm_state,
            suppressions,
            ..
        } = ctx;
        if comm_state.initiation_restricted() {
            return;
        }

        let NotificationTransition {
            change,
            event_type,
            history_source,
            ack_required: ack_required_snapshot,
            event_values,
            construction,
        } = transition.into();
        let system_utc = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let (notification, recipients) = {
            let mut db = db.write().await;

            let (timestamp, message_text, recipient_clock) = match history_source {
                NotificationHistorySource::SendTime => {
                    let sample = sample_event_timestamp(&mut db);
                    (sample.timestamp, None, sample.clock)
                }
                NotificationHistorySource::Committed {
                    snapshot,
                    recipient_clock,
                } => (snapshot.timestamp, snapshot.message_text, recipient_clock),
            };

            let device_oid = db
                .selected_device()
                .unwrap_or_else(|| ObjectIdentifier::new(ObjectType::DEVICE, 0).unwrap());

            let (today, current_time) = match recipient_clock {
                SampledEventClock::Valid(clock_frame) => (
                    clock_frame
                        .day_of_week()
                        .expect("validated ClockFrame has a day of week"),
                    clock_frame.local_time,
                ),
                SampledEventClock::Unavailable => {
                    debug!("Using system UTC to filter recipients without a Device clock");
                    system_utc_recipient_filter_time(system_utc)
                }
                SampledEventClock::Invalid => {
                    debug!(
                        "Using system UTC to filter recipients with an invalid Device clock frame"
                    );
                    system_utc_recipient_filter_time(system_utc)
                }
            };

            let object = match db.get_mut(oid) {
                Some(o) => o,
                None => return,
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
                resolve_transition_priority_ack(&db, notification_class, transition);
            let ack_required = ack_required_snapshot.unwrap_or(resolved_ack_required);

            let Some(recipients) = matched_recipients_or_log(
                lookup_notification_recipients(
                    &db,
                    notification_class,
                    transition,
                    today,
                    &current_time,
                ),
                notification_class,
                transition,
                suppressions,
            ) else {
                return;
            };

            let base_notification = EventNotificationRequest {
                process_identifier: 0,
                initiating_device_identifier: device_oid,
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

            db.log_event_notification(&base_notification);
            (base_notification, recipients)
        };

        if !recipients.is_empty() {
            Self::deliver_local_notification(ctx, notification, &recipients).await;
        }
    }

    /// Distribute a successfully accepted acknowledgment after its requester
    /// response path has completed.
    pub(super) async fn send_acknowledgment_notification_with_bindings(
        ctx: &EventDelivery<'_, T>,
        accepted: handlers::AcceptedAcknowledgeAlarm,
    ) {
        let Some(notification) = accepted.notification else {
            return;
        };
        if !notification.distribute {
            return;
        }
        Self::build_and_send_event_notification_with_bindings(
            ctx,
            &accepted.event_object_identifier,
            NotificationTransition::acknowledgment(notification.change, notification.event_type),
        )
        .await;
    }
}
