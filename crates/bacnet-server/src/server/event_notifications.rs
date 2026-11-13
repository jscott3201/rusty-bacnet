use super::event_recipient_route::system_utc_recipient_filter_time;
use super::event_send::Settle;
use super::*;
use bacnet_device_core::event::{
    AcceptedAcknowledgeAlarm, CommittedIntrinsicTransition, DistributionOutcome, IntrinsicCommit,
    IntrinsicCommitRefusal, NotificationTransition, RecipientWindow, SampledEventClock,
    after_write, distribute_event,
};

#[path = "event_recipient_lookup.rs"]
mod recipient_lookup;
use recipient_lookup::matched_recipients_or_log;

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
/// that committed the write, before it is dropped (#2530). True when it
/// committed a transition.
///
/// The write and the transition it causes then commit as one step: no
/// ReadProperty, sweep or COV fanout sees the object half updated, with a
/// new Reliability or value and the Event_State from before. The detector
/// fires at once only for a `Time_Delay` of 0; a nonzero delay is seeded,
/// due when it ends, and the intrinsic-reporting task
/// ([`start`](BACnetServer::start)) sweeps then and sends that notification.
///
/// It captures nothing for timestamped COV-multiple references: the caller
/// captures the write once the transition has committed, so a reference
/// records the two as one row (#2535). A WritePropertyMultiple, which
/// captured each attempt as it committed, captures an object whose
/// transition committed again, replacing the newest row of each reference
/// its attempts queued one for. A
/// WriteProperty of a Life Safety object takes its exact capture before
/// this, as the core's request path does.
///
/// A transition to distribute is built, logged to the Event Logs and pushed
/// onto its recipients' lanes here, under `db`
/// ([`BACnetServer::enqueue_event_notification`]), so it goes out in commit
/// order (#2537); `settle` gathers the pushes, for the caller's COV fanout
/// to wait on (#889). One that committed but isn't distributed goes nowhere:
/// Event_Enable or a refused projection stop only the notification, not the
/// local transition actions (Clauses 13.2.2.1.4 and 13.2.5). Either way the
/// caller owes the object its COV fanout, since a commit changes
/// Status_Flags.
///
/// `delivery` is called only for a transition to distribute, so a write
/// that commits none does nothing here it didn't do before (#2537).
pub(super) fn commit_written_transition<'a, T: TransportPort + 'static>(
    delivery: impl FnOnce() -> EventDelivery<'a, T>,
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    settle: &mut Settle,
) -> bool {
    let Some(committed) = after_write(db, &oid, crate::runtime_clock::core_now())
        .and_then(|result| logged_intrinsic_commit(&oid, result))
    else {
        return false;
    };
    if committed.distribute && committed.event_values.is_some() {
        let ctx = delivery();
        BACnetServer::<T>::enqueue_event_notification(&ctx, db, &oid, committed, Some(settle));
    }
    true
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Build an `EventNotificationRequest` for a committed transition of
    /// `oid`, under `db`, the guard that committed it, and push it onto the
    /// lane of each recipient the object's Notification Class names (#2537),
    /// counting each push in `settle` for a writer that waits on them. Never
    /// waits: the lanes' tasks send.
    ///
    /// Shared by every path that commits a transition, the periodic
    /// `Time_Delay` sweep included, so all emit identical notifications.
    /// Skipped while DCC restricts initiation. Each recipient's day and time
    /// window is judged now, when the transition commits, not when its lane
    /// gets to it.
    ///
    /// Every notification built here also goes to the device's Event Log
    /// objects ([`ObjectDatabase::log_event_notification`]), under the same
    /// guard, so the logs hold transitions in commit order, with Process
    /// Identifier 0, the value it has before a recipient's own is filled in.
    /// It is logged when the Notification Class selects nobody, since Clause
    /// 13.2.5 keeps the Recipient_List out of distribution to local objects,
    /// but not when the recipient lookup fails closed: that transition is
    /// refused whole, and a record would carry a priority and ack policy the
    /// class never gave. Event_Enable and DCC, which stop the notification
    /// being built, keep it out of the logs too, a local choice. Received
    /// notifications never come here.
    pub(super) fn enqueue_event_notification(
        ctx: &EventDelivery<'_, T>,
        db: &mut ObjectDatabase,
        oid: &ObjectIdentifier,
        transition: impl Into<NotificationTransition>,
        settle: Option<&mut Settle>,
    ) {
        if ctx.comm_state.initiation_restricted() {
            return;
        }
        let system_utc = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        // The Device clock's frame, else the system clock's UTC day and
        // time, as the server has always judged windows.
        let window = RecipientWindow::Frame {
            fallback: Some(system_utc_recipient_filter_time(system_utc)),
        };
        let distribution = distribute_event(db, ctx.device, oid, transition.into(), window);
        match distribution.clock {
            SampledEventClock::Valid(_) => {}
            SampledEventClock::Unavailable => {
                debug!("Using system UTC to filter recipients without a Device clock");
            }
            SampledEventClock::Invalid => {
                debug!("Using system UTC to filter recipients with an invalid Device clock frame");
            }
        }
        let suppressions = ctx.suppressions;
        match distribution.outcome {
            DistributionOutcome::Gone => {}
            DistributionOutcome::Refused {
                notification_class,
                transition,
                lookup,
            } => {
                matched_recipients_or_log(lookup, notification_class, transition, suppressions);
            }
            DistributionOutcome::Built {
                notification,
                notification_class,
                transition,
                lookup,
            } => {
                if let Some(recipients) =
                    matched_recipients_or_log(lookup, notification_class, transition, suppressions)
                    && !recipients.is_empty()
                {
                    Self::push_local_notification(ctx, db, notification, recipients, settle);
                }
            }
        }
    }

    /// [`Self::enqueue_event_notification`] under a database guard of its
    /// own, for a notification no write commits: an accepted
    /// acknowledgment, once its SimpleACK is issued, so the acknowledgment
    /// follows its transition in its lanes (#2537). Counted in `settle` when
    /// one is given.
    pub(super) async fn enqueue_with_guard(
        ctx: &EventDelivery<'_, T>,
        oid: &ObjectIdentifier,
        transition: impl Into<NotificationTransition>,
        settle: Option<&mut Settle>,
    ) {
        let mut db = ctx.db.write().await;
        Self::enqueue_event_notification(ctx, &mut db, oid, transition, settle);
    }

    /// Build and push a transition's notification, then wait until each of
    /// its lanes has handed it off: the old send-and-return shape, for tests.
    #[cfg(test)]
    pub(super) async fn build_and_send_event_notification_with_bindings(
        ctx: &EventDelivery<'_, T>,
        oid: &ObjectIdentifier,
        transition: impl Into<NotificationTransition>,
    ) {
        let mut settle = Settle::default();
        Self::enqueue_with_guard(ctx, oid, transition, Some(&mut settle)).await;
        if let Some(settled) = settle.into_wait() {
            let _ = settled.await;
        }
    }

    /// Push an accepted acknowledgment's notification onto its recipients'
    /// lanes once its requester's response has gone, behind the transition
    /// it acknowledges. Nothing waits for it: its lanes send it.
    pub(super) async fn send_acknowledgment_notification_with_bindings(
        ctx: &EventDelivery<'_, T>,
        accepted: AcceptedAcknowledgeAlarm,
    ) {
        let Some((oid, transition)) = accepted.into_notification() else {
            return;
        };
        Self::enqueue_with_guard(ctx, &oid, transition, None).await;
    }
}
