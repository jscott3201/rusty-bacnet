//! Admission and delivery of one confirmed COV notification, ordinary or
//! Multiple. Its references' baselines advance only on the subscriber's Ack
//! (#896); see `crate::cov::confirmed` for the outstanding-report policy.
use super::super::cov_notify_context::CovFanoutHandles;
use super::*;
use crate::cov::timed_store::{ClaimGuard, LodgedClaim};
use bacnet_device_core::cov::{BeginRefusal, Delivery, PreparedCovCompletion};
use bacnet_device_core::cov::{CovObservation, CovSubscriptionKey};

/// A confirmed notification ready for admission.
pub(in crate::server) struct ConfirmedReport {
    pub(in crate::server) service: ConfirmedServiceChoice,
    /// Reference whose recipient and current route receive the notification.
    pub(in crate::server) route: CovSubscriptionSnapshot,
    /// The report's one ticket, under which every carried reference completes.
    pub(in crate::server) completion: PreparedCovCompletion,
    /// Each carried reference with the observation its Ack completes.
    pub(in crate::server) observations: Vec<(CovSubscriptionSnapshot, CovObservation)>,
    /// Timestamped history conveyed; it retires on the Ack and otherwise
    /// returns to its references.
    pub(in crate::server) claim: Option<ClaimGuard>,
    /// Later parts of a report too large for one notification (#986). They
    /// return to their queue, their untimestamped references owed (#1038),
    /// once this report holds its coordinate, so no other report can carry
    /// them first, and the Ack's follow-up sends them.
    pub(in crate::server) deferred: Vec<ClaimGuard>,
}

impl ConfirmedReport {
    fn label(&self) -> &'static str {
        if self.service == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION {
            "COV notification"
        } else {
            "COVNotificationMultiple"
        }
    }

    fn keys(&self) -> impl Iterator<Item = CovSubscriptionKey> + '_ {
        self.observations.iter().map(|(sub, _)| sub.key().clone())
    }
}

/// What a confirmed report holds once admitted: its peer's in-flight slot
/// and the transaction of its invoke ID.
pub(in crate::server) struct ConfirmedLease {
    guard: crate::cov::CovInFlightGuard,
    operation: crate::server::notification_transactions::NotificationOperation,
    result_rx: tokio::sync::oneshot::Receiver<CovAckResult>,
}

impl ConfirmedLease {
    /// The invoke ID the report goes out under.
    pub(in crate::server) fn invoke_id(&self) -> u8 {
        self.operation.invoke_id()
    }
}

/// A begun confirmed report, ready for its worker.
pub(in crate::server) struct ConfirmedSend {
    pub(in crate::server) lease: ConfirmedLease,
    pub(in crate::server) flight: crate::cov::ConfirmedFlight,
    /// The encoded APDU.
    pub(in crate::server) buf: BytesMut,
    /// Reference whose recipient and current route receive the notification.
    pub(in crate::server) route: CovSubscriptionSnapshot,
    /// Timestamped history conveyed, lodged with the histories until the
    /// report's deadline: the worker commits it on the Ack, and it returns
    /// to its references otherwise.
    pub(in crate::server) claim: Option<LodgedClaim>,
    pub(in crate::server) label: &'static str,
    pub(in crate::server) revisits: Arc<crate::cov::CovRevisits>,
    /// When the table began the report: its deadline, and the worker's
    /// cutoff, run from it.
    pub(in crate::server) begun: tokio::time::Instant,
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Admit a confirmed report to `route` against the peer and global
    /// in-flight limits and an invoke ID. A refusal is counted or logged,
    /// and the report isn't sent. Synchronous: it takes leaf mutexes only,
    /// so the Single fanout calls it under its database and table guards.
    pub(in crate::server) fn admit_confirmed(
        handles: &CovFanoutHandles<'_, '_, T>,
        route: &CovSubscriptionSnapshot,
        service: ConfirmedServiceChoice,
        label: &'static str,
    ) -> Option<ConfirmedLease> {
        let &CovFanoutHandles {
            ctx,
            in_flight_tracker,
            counters,
        } = handles;
        let guard = match in_flight_tracker.try_acquire(
            route.recipient(),
            ctx.config.cov_policy.max_confirmed_in_flight_per_peer,
            ctx.cov_in_flight,
        ) {
            Ok(guard) => guard,
            Err(InFlightAcquireError::PeerLimitExceeded) => {
                counters
                    .notifications_throttled_peer
                    .fetch_add(1, Ordering::Relaxed);
                return None;
            }
            Err(InFlightAcquireError::GlobalPoolExhausted) => {
                warn!(
                    object = ?route.monitored_object_identifier,
                    "255 confirmed COV notifications in-flight, skipping {label}"
                );
                return None;
            }
        };
        let (operation, result_rx) = match ctx
            .notification_transactions
            .reserve(Self::canonical_cov_peer(route), service)
        {
            Ok(reservation) => reservation,
            Err(error) => {
                warn!(%error, "No free invoke ID for confirmed {label}");
                return None;
            }
        };
        Some(ConfirmedLease {
            guard,
            operation,
            result_rx,
        })
    }

    /// Admit `report` against the peer and global in-flight limits, an invoke
    /// ID, the event budget and its coordinate's outstanding-report mark, then
    /// deliver it with the usual retries. `encode` builds the APDU for the
    /// reserved invoke ID.
    pub(in crate::server) async fn send_confirmed_cov(
        handles: &CovFanoutHandles<'_, '_, T>,
        budget: &mut EventBudget,
        mut report: ConfirmedReport,
        encode: impl FnOnce(u8) -> Result<BytesMut, Error>,
    ) {
        let counters = handles.counters;
        let label = report.label();
        let Some(lease) = Self::admit_confirmed(handles, &report.route, report.service, label)
        else {
            return;
        };
        let buf = match encode(lease.invoke_id()) {
            Ok(buf) => buf,
            Err(error) => {
                warn!(%error, "Failed to encode confirmed {label}");
                return;
            }
        };
        if !budget.try_consume(buf.len()) {
            counters
                .notifications_throttled_fanout
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        let (flight, revisits) = {
            let mut table = handles.ctx.cov_table.write().await;
            let flight = table.begin_confirmed(
                report.completion,
                report
                    .observations
                    .iter()
                    .map(|(sub, observation)| (sub, observation.clone())),
            );
            (flight, Arc::clone(table.revisits()))
        };
        // The instant the table began the report, near enough: its deadline
        // runs from it, and the worker's cutoff too.
        let begun = tokio::time::Instant::now();
        let flight = match flight {
            Ok(flight) => {
                // The coordinate is marked busy: the later parts can queue,
                // and their untimestamped references are owed (#1038). A
                // change this part sends only some values of keeps the rest
                // queued until it is delivered (#1163).
                if let Some(claim) = &report.claim {
                    claim.going_out();
                }
                for deferred in std::mem::take(&mut report.deferred) {
                    drop(deferred.owing());
                }
                flight
            }
            Err(refusal) => {
                budget.refund(buf.len());
                let keys: Vec<_> = report.keys().collect();
                // Put drained history back before any follow-up can drain the
                // same references again, so it stays in capture order.
                drop(report);
                if refusal == BeginRefusal::NotCurrent {
                    // A fence moved a reference since it was captured; look at
                    // its live state again. A busy coordinate needs nothing: its
                    // outstanding report, Ack or hold-off owns the follow-up.
                    revisits.request(keys);
                }
                return;
            }
        };
        // The worker's claim ends with its report, at the one instant the
        // table gave the report: the histories keep it until then, and a
        // live worker settles ahead of that, as it does the report (its
        // cutoff is half a timeout earlier).
        let claim = report.claim.map(|claim| claim.lodge(flight.deadline()));
        Self::spawn_confirmed(
            handles,
            ConfirmedSend {
                lease,
                flight,
                buf,
                route: report.route,
                claim,
                label,
                revisits,
                begun,
            },
        );
    }

    /// Count a begun confirmed report as sent and deliver it with the usual
    /// retries on a worker of its own, which ends it in the table.
    pub(in crate::server) fn spawn_confirmed(
        handles: &CovFanoutHandles<'_, '_, T>,
        send: ConfirmedSend,
    ) {
        let &CovFanoutHandles { ctx, counters, .. } = handles;
        let ConfirmedSend {
            lease:
                ConfirmedLease {
                    guard,
                    operation,
                    result_rx,
                },
            flight,
            buf,
            route,
            claim,
            label,
            revisits,
            begun,
        } = send;
        counters.notifications_sent.fetch_add(1, Ordering::Relaxed);
        counters
            .notifications_confirmed
            .fetch_add(1, Ordering::Relaxed);
        counters
            .notification_bytes_sent
            .fetch_add(buf.len() as u64, Ordering::Relaxed);

        let id = operation.invoke_id();
        let (counted, buf_len) = (Arc::clone(counters), buf.len() as u64);
        let network = Arc::clone(ctx.network);
        let comm_state = Arc::clone(ctx.comm_state);
        let cov_table = Arc::clone(ctx.cov_table);
        let apdu_timeout = Duration::from_millis(ctx.config.cov_retry_timeout_ms);
        let apdu_retries = DEFAULT_APDU_RETRIES;
        // After a failure the coordinate waits one full retry cycle, the first
        // attempt and every retry, before it may report again. That bounds a
        // dead or refusing subscriber to half the in-flight time at most.
        let hold_off = apdu_timeout * (u32::from(apdu_retries) + 1);
        ctx.notification_transactions.spawn(async move {
            // Whether an attempt got past the DCC check, so went to the link.
            let attempted = std::sync::atomic::AtomicBool::new(false);
            let delivery = run_notification_under_dcc(
                operation,
                result_rx,
                apdu_timeout,
                apdu_retries,
                &comm_state,
                |attempt| {
                    attempted.store(true, Ordering::Relaxed);
                    let network = Arc::clone(&network);
                    let buf = buf.clone();
                    let route = route.clone();
                    async move {
                        let result = Self::send_cov_apdu(&network, &buf, &route, true).await;
                        match &result {
                            Ok(()) => debug!(invoke_id = id, attempt, "Confirmed {label} sent"),
                            Err(error) => warn!(%error, attempt, "{label} send failed"),
                        }
                        result
                    }
                },
            );
            // However long a send stalls (a BACnet/SC send can wait on a hub
            // or a direct dial), the report ends half a timeout before the
            // table's deadline, as one whose retries ran out. Slow but working
            // sends still end the usual way, by their retries, with the last
            // answer window whole; the deadline itself is for a worker that
            // can't report. Past it a stale retry could follow the
            // coordinate's next report (#2303). The cutoff takes no lock, and
            // cancels the transaction as a fence does.
            let cutoff = begun + hold_off + apdu_timeout / 2;
            let delivery = tokio::time::timeout_at(cutoff, delivery);
            // A renewal or route change replaced this report's incarnation: its
            // retries would only land after the replacement's own report, and
            // its Ack could complete nothing. Dropping the delivery cancels the
            // transaction; there is no hold-off on the orphaned marker.
            // A fence already ended the report in the table; nothing is
            // owed back.
            let result = tokio::select! {
                // Logged once, below, as a report whose retries ran out.
                result = delivery => result.unwrap_or_else(|_| {
                    debug!(invoke_id = id, "{label} cut off before its deadline");
                    Ok(NotificationWorkerResult::Exhausted)
                }),
                () = flight.fenced() => {
                    debug!(invoke_id = id, "{label} superseded by a new incarnation");
                    drop(claim);
                    drop(guard);
                    return;
                }
            };
            let revisit = match result {
                Ok(NotificationWorkerResult::Ack) => {
                    debug!(invoke_id = id, "{label} acknowledged");
                    let mut table = cov_table.write().await;
                    if let Some(claim) = claim {
                        claim.commit();
                    }
                    // Changes to the references were held while the report was
                    // outstanding: to a context's every reference, not only to
                    // those it carried, and still if every carried one has gone.
                    table.delivered(&flight, Delivery::Acknowledged)
                }
                Ok(NotificationWorkerResult::Error(_) | NotificationWorkerResult::Exhausted) => {
                    if matches!(result, Ok(NotificationWorkerResult::Error(_))) {
                        warn!(invoke_id = id, "{label} rejected by subscriber");
                    } else {
                        warn!(
                            invoke_id = id,
                            "{label} failed after {} retries", apdu_retries
                        );
                    }
                    // Return unacknowledged history before the mark clears, so
                    // the next report conveys it again in capture order.
                    drop(claim);
                    let mut table = cov_table.write().await;
                    table.delivered(&flight, Delivery::Failed);
                    // Its timestamped history goes out once the hold-off ends.
                    if let Some(context) = route.key().multiple_context() {
                        let until = runtime_clock::core_now()
                            + bacnet_time::Duration::saturating_from_core(hold_off);
                        table.timed().hold_until(context, until);
                    }
                    Vec::new()
                }
                Ok(NotificationWorkerResult::Closed) => {
                    drop(claim);
                    cov_table
                        .write()
                        .await
                        .delivered(&flight, Delivery::NotSent);
                    Vec::new()
                }
                // DCC ended the report at an attempt. The subscriber did not
                // fail, so there is no hold-off: the report is owed as one DCC
                // held back before its first send. Its history returns to the
                // queue before the mark clears, the backstop sends it once
                // initiation is enabled again, and the next fanout reports
                // untimestamped values still away from their baselines. A
                // change the report began sending value by value stays in
                // delivery (#1163), as after a report whose sends all failed.
                Err(InitiationRestricted) => {
                    debug!(
                        invoke_id = id,
                        "{label} withdrawn: DCC restricts initiation"
                    );
                    if !attempted.load(Ordering::Relaxed) {
                        // DCC took effect after the fanout counted the report
                        // but before its first attempt: nothing went out.
                        counted.notifications_sent.fetch_sub(1, Ordering::Relaxed);
                        counted
                            .notifications_confirmed
                            .fetch_sub(1, Ordering::Relaxed);
                        counted
                            .notification_bytes_sent
                            .fetch_sub(buf_len, Ordering::Relaxed);
                    }
                    drop(claim);
                    cov_table
                        .write()
                        .await
                        .delivered(&flight, Delivery::NotSent);
                    Vec::new()
                }
            };
            // Free the peer slot before the follow-up needs it.
            drop(guard);
            revisits.request(revisit);
        });
    }
}

#[cfg(test)]
mod tests;
