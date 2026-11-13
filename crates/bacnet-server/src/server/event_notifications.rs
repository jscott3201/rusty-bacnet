use super::event_recipient_route::system_utc_recipient_filter_time;
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

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Evaluate intrinsic reporting on an object and send event notifications
    /// to the recipients the object's NotificationClass names.
    /// DCC gates network-message initiation (Clause 16.1), not the local
    /// transition actions in Clause 13.2.2.1.4. The outbound sender below
    /// suppresses distribution while communications are disabled.
    ///
    /// This is the per-write entry point: it probes the detector, which fires
    /// immediately only when `Time_Delay == 0`. For a nonzero delay the probe
    /// seeds a pending transition due when the delay ends (returning `None`,
    /// so no notification is sent here), and the intrinsic-reporting task
    /// ([`start`](Self::start)) sweeps at that instant and sends the
    /// notification.
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
            let resolved = after_write(&mut db, oid, crate::runtime_clock::core_now())
                .and_then(|result| logged_intrinsic_commit(oid, result));
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
        if let Some(resolved) = resolved
            && resolved.distribute
            && resolved.event_values.is_some()
        {
            Self::build_and_send_event_notification_with_bindings(ctx, oid, resolved).await;
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
