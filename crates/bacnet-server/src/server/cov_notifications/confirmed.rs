//! Admission and delivery of one confirmed COV notification, ordinary or
//! Multiple. Its references' baselines advance only on the subscriber's Ack
//! (#896); see `crate::cov::confirmed` for the outstanding-report policy.
use super::super::cov_notify_context::CovFanoutHandles;
use super::*;
use crate::cov::timed::TimedClaim;
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
    pub(in crate::server) claim: Option<TimedClaim>,
    /// Later parts of a report too large for one notification (#986). They
    /// return to their queue, their untimestamped references owed (#1038),
    /// once this report holds its coordinate, so no other report can carry
    /// them first, and the Ack's follow-up sends them.
    pub(in crate::server) deferred: Vec<TimedClaim>,
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

impl<T: TransportPort + 'static> BACnetServer<T> {
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
        let &CovFanoutHandles {
            ctx,
            in_flight_tracker,
            counters,
        } = handles;
        let label = report.label();
        let guard = match in_flight_tracker.try_acquire(
            report.route.recipient(),
            ctx.config.cov_policy.max_confirmed_in_flight_per_peer,
            ctx.cov_in_flight,
        ) {
            Ok(guard) => guard,
            Err(InFlightAcquireError::PeerLimitExceeded) => {
                counters
                    .notifications_throttled_peer
                    .fetch_add(1, Ordering::Relaxed);
                return;
            }
            Err(InFlightAcquireError::GlobalPoolExhausted) => {
                warn!(
                    object = ?report.route.monitored_object_identifier,
                    "255 confirmed COV notifications in-flight, skipping {label}"
                );
                return;
            }
        };
        let (operation, result_rx) = match ctx
            .notification_transactions
            .reserve(Self::canonical_cov_peer(&report.route), report.service)
        {
            Ok(reservation) => reservation,
            Err(error) => {
                warn!(%error, "No free invoke ID for confirmed {label}");
                return;
            }
        };
        let buf = match encode(operation.invoke_id()) {
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
            let mut table = ctx.cov_table.write().await;
            let flight = table.begin_confirmed(
                report.completion,
                report.observations.iter().map(|(sub, _)| sub),
            );
            (flight, Arc::clone(table.revisits()))
        };
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
            let ConfirmedReport {
                route,
                completion,
                observations,
                claim,
                ..
            } = report;
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
            // A renewal or route change replaced this report's incarnation: its
            // retries would only land after the replacement's own report, and
            // its Ack could complete nothing. Dropping the delivery cancels the
            // transaction; there is no hold-off on the orphaned marker.
            // A fence already ended the report in the table; nothing is
            // owed back.
            let result = tokio::select! {
                result = delivery => result,
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
                    let completed: Vec<_> = observations
                        .into_iter()
                        .filter(|(sub, observation)| {
                            table.complete_observation(sub, completion, observation.clone())
                        })
                        .map(|(sub, _)| sub.key().clone())
                        .collect();
                    table.delivered(&flight, Delivery::Acknowledged);
                    // Changes to any reference of a context were held while the
                    // report was outstanding, not only to those it carried, and
                    // they still are if every carried reference has gone since.
                    match route.key().multiple_context() {
                        Some(context) => table
                            .multiple_context_references(context)
                            .map(|sub| sub.key().clone())
                            .collect(),
                        None => completed,
                    }
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
                        let until = tokio::time::Instant::now() + hold_off;
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
