//! Spawn helpers for the tasks a server's start spawns, one each (#2558);
//! the Event Enrollment and Binary Lighting tasks have theirs beside their
//! code. Each helper is a plain `fn` that makes its task's future, spawns it
//! and returns its handle; the start spawns them in a fixed order. All but
//! the network-number helper spawn through `spawn_owned`, which puts the
//! future on the heap and has the task hold the target Audit owner, when
//! there is one, while it runs. The network-number task holds no owner: it
//! is spawned by the Number worker (on the heap) or, on a seat, by the
//! follower of the node's number (a plain `tokio::spawn`).

use super::super::audit_recipient::spawn_owned;
use super::super::cov_fanout::CovFanout;
use super::super::request_tasks::RequestTasks;
use super::super::server_seat::Apdus;
use super::*;
use bacnet_endpoint_core::segmented_request::SegmentedRequests;

/// The target Audit owner a task holds while it runs, when there is one.
pub(super) type AuditOwner = Option<Arc<bacnet_objects::database::AuditOwnership>>;

/// The network number's task: with the link's Number controls, the
/// server's own Number owner; on a seat, the follower of the node's.
pub(super) fn spawn_network_number<T: TransportPort + 'static>(
    network_controls: Option<mpsc::Receiver<bacnet_network::layer::ReceivedNetworkControl>>,
    seat_number: Option<bacnet_endpoint_core::seat::SeatNumber>,
    network: &Arc<NetworkLayer<T>>,
    db: &Arc<RwLock<ObjectDatabase>>,
    config: &ServerConfig,
    target_audit: &Option<Arc<super::super::audit_recipient::TargetAudit<T>>>,
) -> Option<JoinHandle<()>> {
    match (network_controls, seat_number) {
        (Some(controls), _) => Some(super::network_port::spawn_number_worker(
            network,
            db,
            config.registered_network_port,
            controls,
            target_audit.as_ref().map(Arc::downgrade),
        )),
        // On a seat the node's Number owner publishes; target Audit
        // follows its notice (#2617).
        (None, Some(number)) => target_audit
            .as_ref()
            .map(|audit| super::server_seat::follow_number(number, Arc::downgrade(audit))),
        (None, None) => None,
    }
}

/// What the dispatch loop owns: the Device's segment timeout, the
/// handles it hands [`BACnetServer::dispatch`], the request set it joins,
/// and where its APDUs come from.
pub(super) struct DispatchLoop<T: TransportPort + 'static, A> {
    pub(super) segment_timeout: Duration,
    pub(super) dispatch_context: DispatchContext<T>,
    pub(super) requests: Arc<RequestTasks>,
    pub(super) network_dispatch: Arc<NetworkLayer<T>>,
    pub(super) config_dispatch: Arc<ServerConfig>,
    pub(super) notification_transactions_dispatch: Arc<NotificationTransactions>,
    pub(super) apdu_rx: A,
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// The dispatch loop: each APDU the link hands the server, decoded and
    /// dispatched, a segmented request through the reassembly it opens, and
    /// the request and notification tasks joined as they finish.
    pub(super) fn spawn_dispatch_loop<A: Apdus>(
        audit_owner: AuditOwner,
        dispatch_loop: DispatchLoop<T, A>,
    ) -> JoinHandle<()> {
        let DispatchLoop {
            segment_timeout,
            dispatch_context,
            requests,
            network_dispatch,
            config_dispatch,
            notification_transactions_dispatch,
            mut apdu_rx,
        } = dispatch_loop;
        spawn_owned(audit_owner, move || async move {
            let mut seg_receivers = SegmentedRequests::new(segment_timeout);
            let mut notifications_open = true;
            let mut ingress_open = true;
            // A peer's flood warns once per interval, the rest at debug (#1950).
            let warnings = Arc::clone(&dispatch_context.services.warnings);
            let (decode_warning, segment_warning) = (&warnings.decode, &warnings.segment);

            // While ingress is open, finished handlers are joined at the top
            // of each turn, not awaited below (IdleReap, #1713).
            let mut idle_reap = super::request_tasks::IdleReap::new();
            loop {
                requests.reap_finished();
                // Drain all selected ownership before the first asynchronous Abort.
                // With no reassembly open nothing can expire, so the clock
                // isn't read (#1978 item 6).
                if !seg_receivers.is_empty() {
                    Self::reap_expired_requests(
                        &network_dispatch,
                        &mut seg_receivers,
                        runtime_clock::now(),
                    )
                    .await;
                }
                let received = tokio::select! {
                    () = super::segmented_receive::wait_receive_deadline(&seg_receivers), if !seg_receivers.is_empty() => continue,
                    result = notification_transactions_dispatch.join_next(), if notifications_open => {
                        notifications_open = result.is_some();
                        NotificationTransactions::observe(result);
                        continue;
                    }
                    result = requests.join_next(), if !ingress_open && !requests.is_empty() => {
                        super::request_tasks::RequestTasks::observe(result);
                        continue;
                    }
                    () = idle_reap.tick(), if ingress_open && !requests.is_empty() => continue,
                    received = apdu_rx.recv(), if ingress_open => match received {
                        Some(received) => received,
                        None => {
                            // Local/periodic notification producers outlive ingress.
                            // Keep both join consumers active, but never poll EOF again.
                            ingress_open = false;
                            seg_receivers.clear();
                            continue;
                        }
                    },
                    else => break,
                };
                // Input and the timer can become ready together. Apply expiry
                // before interpreting this segment in either scheduling order.
                if !seg_receivers.is_empty() {
                    Self::reap_expired_requests(
                        &network_dispatch,
                        &mut seg_receivers,
                        runtime_clock::now(),
                    )
                    .await;
                }

                match apdu::decode_apdu(received.apdu.clone()) {
                    Ok(decoded) => {
                        if notification_transactions_dispatch
                            .application_sealed
                            .load(Ordering::Acquire)
                        {
                            seg_receivers.clear();
                            if matches!(
                                decoded,
                                Apdu::ConfirmedRequest(_) | Apdu::UnconfirmedRequest(_)
                            ) {
                                continue;
                            }
                        }
                        // A confirmed request may only be addressed to one
                        // device (Clause 6.3). One that arrives by local,
                        // remote or global broadcast, or by multicast, leaves
                        // the server TSM idle (Clause 5.4.5.1): it is neither
                        // executed nor answered, whatever the service, and
                        // no reassembly starts for it.
                        if received.is_group && matches!(decoded, Apdu::ConfirmedRequest(_)) {
                            debug!("Ignoring a ConfirmedRequest addressed to a group");
                            continue;
                        }
                        // Nor one sent from a group address, where its
                        // answer would go (#1504).
                        if matches!(decoded, Apdu::ConfirmedRequest(_))
                            && dispatch_context
                                .confirmed_request_tracker
                                .refuse_group_source(&received.source_mac, |mac| {
                                    network_dispatch.transport().is_group_destination(mac)
                                })
                        {
                            continue;
                        }
                        let source_mac = MacAddr::from_slice(&received.source_mac);

                        // Clause 5.4.5.2 AbortPDU_Received: a peer's Abort
                        // ('server' = FALSE) ends any reassembly session for
                        // its transaction. A side effect, not a short
                        // circuit — the PDU still reaches `dispatch`, whose
                        // Abort arm cancels in-flight segmented response
                        // senders and records server-TSM results (#377).
                        super::segmented_receive::remove_peer_aborted_request(
                            &mut seg_receivers,
                            &received,
                            &decoded,
                        );

                        if let Apdu::ConfirmedRequest(ref req) = decoded
                            && req.segmented
                        {
                            // The shared receiver takes it (#2742); the plain
                            // path below is untouched.
                            Self::receive_request_segment(
                                &network_dispatch,
                                &dispatch_context,
                                &config_dispatch,
                                &mut seg_receivers,
                                segment_warning,
                                &source_mac,
                                req,
                                received,
                            )
                            .await;
                        } else {
                            Self::dispatch(&dispatch_context, &source_mac, decoded, received).await;
                        }
                    }
                    Err(e) => {
                        warn_throttled!(
                            decode_warning,
                            error = %e,
                            "Server failed to decode received APDU"
                        );
                    }
                }
            }
        })
    }
}

/// Every 30 seconds, the COV subscriptions whose lifetime has ended go.
pub(super) fn spawn_cov_purge(
    audit_owner: AuditOwner,
    cov_table_for_purge: Arc<RwLock<CovSubscriptionTable>>,
) -> JoinHandle<()> {
    spawn_owned(audit_owner, move || async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            let mut table = cov_table_for_purge.write().await;
            let purged = table.purge_expired();
            if purged > 0 {
                debug!(purged, "Purged expired COV subscriptions");
            }
        }
    })
}

/// The opt-in fault-detection task (`background::fault_detection`).
pub(super) fn spawn_fault_detection<T: TransportPort + 'static>(
    audit_owner: AuditOwner,
    fanout: CovFanout<T>,
) -> JoinHandle<()> {
    spawn_owned(audit_owner, move || background::fault_detection(fanout))
}

/// The Trend Log, Trend Log Multiple and Event Log task (`crate::trend_log::run`).
pub(super) fn spawn_trend_log(
    audit_owner: AuditOwner,
    db: &Arc<RwLock<ObjectDatabase>>,
) -> JoinHandle<()> {
    spawn_owned(audit_owner, || crate::trend_log::run(Arc::clone(db)))
}

/// The 60-second Schedule pass, and the work an application's own
/// `add` or `remove` queued on the database (`crate::membership`).
pub(super) fn spawn_schedules<T: TransportPort + 'static>(
    audit_owner: AuditOwner,
    schedule_fanout: CovFanout<T>,
    membership: Arc<tokio::sync::Notify>,
    command_runs: super::super::command_runs::CommandRunner<T>,
) -> JoinHandle<()> {
    spawn_owned(audit_owner, move || {
        background::schedules(schedule_fanout, membership, command_runs)
    })
}

/// Intrinsic-reporting task: once a second, and when a pending
/// transition's `Time_Delay` ends, it fires the transitions that are due
/// and sends their EventNotifications. The per-write path only *seeds* a
/// pending transition, due its delay after the write (see
/// `fire_event_notifications`); this task is the sole confirmer, so
/// repeated writes cannot shorten the delay (ASHRAE 135-2020 Clause
/// 13.3). Runs unconditionally like the trend-log task — a no-pending
/// sweep is a cheap empty iteration.
///
/// It is also what carries Reliability into event-state-detection when
/// no writer evaluated the change. Per Clause 13.2.2 the FAULT
/// determination is a standing condition, so each sweep re-derives it
/// from the object's current `Reliability`. A network or local write and
/// the fault detector's object-owned hooks above commit the transition
/// with the change (#2535), so the sweep finds it done; a setter that
/// bypasses them reaches detection here. `enable_fault_detection`
/// governs only whether those hooks run every 10 seconds, never whether
/// an existing Reliability is honored.
///
/// Six of the nine wired object types have no route that can set
/// Reliability, so the fault path is correct but inert on them (#218).
pub(super) fn spawn_intrinsic_reporting<T: TransportPort + 'static>(
    audit_owner: AuditOwner,
    cov_fanout: &CovFanout<T>,
) -> JoinHandle<()> {
    spawn_owned(audit_owner, || intrinsic::run(cov_fanout.clone()))
}

/// Reports what changed while a confirmed report was outstanding, once
/// it is acknowledged (#896).
pub(super) fn spawn_cov_revisits<T: TransportPort + 'static>(
    audit_owner: AuditOwner,
    cov_fanout: &CovFanout<T>,
) -> JoinHandle<()> {
    spawn_owned(audit_owner, || cov_fanout.clone().run_revisits())
}
