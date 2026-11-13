use super::event_recipient_route::system_utc_recipient_filter_time;
use super::*;
use bacnet_device_core::event::{
    AcceptedAcknowledgeAlarm, CommittedIntrinsicTransition, DistributionOutcome, IntrinsicCommit,
    IntrinsicCommitRefusal, MessageTexts, NotificationConstruction, NotificationHistorySource,
    NotificationTransition, RecipientWindow, SampledEventClock, after_write, distribute_event,
    read_committed_history_snapshot,
};

use crate::event_enrollment::{CommittedEventEnrollmentDelivery, CommittedEventEnrollmentResult};

#[path = "event_recipient_lookup.rs"]
mod recipient_lookup;
use recipient_lookup::matched_recipients_or_log;

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

/// Log why an intrinsic commit gave nothing to distribute, as the commit
/// once did itself, and keep what it committed.
pub(super) fn logged_intrinsic_commit(
    oid: &ObjectIdentifier,
    result: IntrinsicCommit,
) -> Option<CommittedIntrinsicTransition> {
    match result {
        Ok(committed) => {
            if committed.event_values.is_none() {
                debug!(
                    %oid,
                    event_type = ?committed.event_type,
                    "Committed intrinsic Event Values projection rejected; suppressing distribution"
                );
            }
            Some(committed)
        }
        Err(IntrinsicCommitRefusal::Gone) => None,
        Err(IntrinsicCommitRefusal::Rejected(error)) => {
            debug!(%oid, ?error, "Intrinsic transition commit rejected");
            None
        }
        Err(IntrinsicCommitRefusal::HistoryUnreadable(coordinate)) => {
            debug!(
                %oid,
                ?coordinate,
                "Committed intrinsic history projection rejected; suppressing distribution"
            );
            None
        }
    }
}

/// The per-write intrinsic evaluation of `oid`, made under `db`, the guard
/// that committed the write, before it is dropped (#2530).
///
/// The write and the transition it causes then commit as one step: no
/// ReadProperty, sweep or ordinary COV fanout sees the object half updated,
/// with a new Reliability or value and the Event_State from before. The
/// detector fires at once only for a `Time_Delay` of 0; a nonzero delay is
/// seeded, due when it ends, and the intrinsic-reporting task
/// ([`start`](BACnetServer::start)) sweeps then and sends that notification.
/// A committed transition is captured for timestamped COV-multiple
/// references at its own time, the table read under `db` (database, then
/// table). The write's own capture runs before this, so a timestamped
/// reference still records the write and its transition as two changes.
///
/// A transition to send goes on `to_send`, for
/// [`BACnetServer::send_written_transitions`] as soon as `db` is dropped,
/// ahead of the write's COV fanout (#889) and, for a peer, of its response.
/// One that committed but isn't distributed goes nowhere: Event_Enable or a
/// refused projection stop only the notification, not the local transition
/// actions (Clauses 13.2.2.1.4 and 13.2.5). Either way the caller owes the
/// object its COV fanout, since a commit changes Status_Flags.
///
/// Transitions from one task go out in the order they committed. Across
/// tasks the window is the one between dropping `db` and the send: a later
/// write of the same object can commit and send its own transition there,
/// ahead of this one. Nothing orders sends across tasks.
///
/// The transition is moved out before the table read is awaited, so the
/// write's future holds no transition across it: the write handlers' futures
/// sit in every confirmed request's, and would grow by its size.
pub(super) async fn commit_written_transition(
    db: &mut ObjectDatabase,
    cov_table: &RwLock<CovSubscriptionTable>,
    oid: ObjectIdentifier,
    to_send: &mut Vec<(ObjectIdentifier, CommittedIntrinsicTransition)>,
) {
    {
        let Some(committed) = after_write(db, &oid, crate::runtime_clock::core_now())
            .and_then(|result| logged_intrinsic_commit(&oid, result))
        else {
            return;
        };
        if committed.distribute && committed.event_values.is_some() {
            to_send.push((oid, committed));
        }
    }
    let capture = cov_table.read().await.timed_capture(oid);
    capture.run(db);
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Send the transitions writes committed ([`commit_written_transition`]),
    /// in the order they committed, before the COV fanout of those writes. A
    /// confirmed service (WriteProperty, WritePropertyMultiple, AddListElement
    /// and RemoveListElement) has already put every object it wrote, Life
    /// Safety objects aside, in that fanout, so a committed transition's
    /// Status_Flags change reaches subscribers.
    pub(super) async fn send_written_transitions(
        ctx: &EventDelivery<'_, T>,
        written: Vec<(ObjectIdentifier, CommittedIntrinsicTransition)>,
    ) {
        for (oid, transition) in written {
            Self::build_and_send_event_notification_with_bindings(ctx, &oid, transition).await;
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

        let system_utc = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let fallback = system_utc_recipient_filter_time(system_utc);
        let (notification, recipients) = {
            let mut db = db.write().await;
            // The Device clock's frame, else the system clock's UTC day and
            // time, as the server has always judged windows.
            let window = RecipientWindow::Frame {
                fallback: Some(fallback),
            };
            let distribution =
                distribute_event(&mut db, ctx.device, oid, transition.into(), window);
            match distribution.clock {
                SampledEventClock::Valid(_) => {}
                SampledEventClock::Unavailable => {
                    debug!("Using system UTC to filter recipients without a Device clock");
                }
                SampledEventClock::Invalid => {
                    debug!(
                        "Using system UTC to filter recipients with an invalid Device clock frame"
                    );
                }
            }
            match distribution.outcome {
                DistributionOutcome::Gone => return,
                DistributionOutcome::Refused {
                    notification_class,
                    transition,
                    lookup,
                } => {
                    matched_recipients_or_log(lookup, notification_class, transition, suppressions);
                    return;
                }
                DistributionOutcome::Built {
                    notification,
                    notification_class,
                    transition,
                    lookup,
                } => {
                    let Some(recipients) = matched_recipients_or_log(
                        lookup,
                        notification_class,
                        transition,
                        suppressions,
                    ) else {
                        return;
                    };
                    (notification, recipients)
                }
            }
        };

        if !recipients.is_empty() {
            Self::deliver_local_notification(ctx, notification, &recipients).await;
        }
    }

    /// Distribute a successfully accepted acknowledgment after its requester
    /// response path has completed.
    pub(super) async fn send_acknowledgment_notification_with_bindings(
        ctx: &EventDelivery<'_, T>,
        accepted: AcceptedAcknowledgeAlarm,
    ) {
        let Some((oid, transition)) = accepted.into_notification() else {
            return;
        };
        Self::build_and_send_event_notification_with_bindings(ctx, &oid, transition).await;
    }
}
