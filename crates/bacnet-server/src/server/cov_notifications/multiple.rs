use super::super::cov_notify_context::{CovFanoutHandles, CovNotifyContext};
use super::confirmed::ConfirmedReport;
use super::*;
use crate::cov::multiple_report::{Gathering, Reading, Source, by_context, carried};
use crate::cov::timed_store::{ClaimGuard, SendTurn};
use std::collections::BTreeSet;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Fire the initial COVNotificationMultiple for a newly accepted
    /// SubscribeCOVPropertyMultiple request.
    pub(in crate::server) async fn fire_initial_cov_notification_multiple(
        ctx: &CovNotifyContext<'_, T>,
        subscriptions: &[CovSubscriptionSnapshot],
    ) {
        let (counters, in_flight_tracker, subscriptions) = {
            let table = ctx.cov_table.read().await;
            (
                Arc::clone(table.counters()),
                Arc::clone(table.in_flight_tracker()),
                subscriptions
                    .iter()
                    .filter(|sub| table.is_current(sub))
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        };
        let mut budget = EventBudget::new(&ctx.config.cov_policy);
        Self::fire_cov_notification_multiple_for_subscriptions(
            &CovFanoutHandles {
                ctx,
                in_flight_tracker: &in_flight_tracker,
                counters: &counters,
            },
            &subscriptions,
            None,
            true,
            &mut budget,
        )
        .await;
    }

    pub(in crate::server) async fn fire_cov_notification_multiple_for_subscriptions(
        handles: &CovFanoutHandles<'_, '_, T>,
        subscriptions: &[CovSubscriptionSnapshot],
        snapshot: Option<&dyn bacnet_objects::traits::BACnetObject>,
        force: bool,
        budget: &mut EventBudget,
    ) {
        if handles.ctx.comm_state.initiation_restricted() || subscriptions.is_empty() {
            return;
        }

        if budget.is_exhausted() {
            return;
        }

        // Contexts report in the order of their keys: by recipient, then
        // process identifier, unconfirmed before confirmed. A hash map
        // gave an order that varied from run to run (#2320).
        for subs in by_context(subscriptions.iter().cloned()).values() {
            Self::send_cov_notification_multiple(handles, subs, snapshot, force, budget).await;
        }
    }

    async fn send_cov_notification_multiple(
        handles: &CovFanoutHandles<'_, '_, T>,
        subscriptions: &[CovSubscriptionSnapshot],
        snapshot: Option<&dyn bacnet_objects::traits::BACnetObject>,
        force: bool,
        budget: &mut EventBudget,
    ) {
        let &CovFanoutHandles {
            ctx:
                &CovNotifyContext {
                    db,
                    network,
                    cov_table,
                    config,
                    ..
                },
            counters,
            ..
        } = handles;
        if subscriptions.is_empty() {
            return;
        }

        // `turn` is the context's send turn, if it is unconfirmed (#986,
        // #1038). It is declared before any claim guard so that, on any
        // early return, the claims' changes are back in their queue before
        // the turn hands the context to a follow-up.
        let (report, store, turn) = {
            // One DB borrow, released before any send, supplies the clock
            // sample for any current-state fallback and every value; the
            // initiating Device comes from the frozen identity (#2217).
            // Producers capture under the database write guard, so while this
            // read guard is held no capture can move a field record past the
            // values read here. A producer snapshot carries its own captured
            // changes, so that path takes no clock (#987).
            let db = if snapshot.is_none() {
                Some(db.read().await)
            } else {
                None
            };
            let source = match (snapshot, db.as_deref()) {
                (Some(object), _) => Source::Snapshot(object),
                (None, Some(db)) => Source::Database(db),
                (None, None) => unreachable!("the database is read without a snapshot"),
            };
            let reading = Reading::read(source, subscriptions, force);
            // Established lock order: DB read -> table write -> timed store.
            // No object callback runs under the table guard, and the tickets
            // are issued under both guards, so they follow the order of the
            // observations (#826). The step holds the store for one
            // synchronous section and drops no claim guard or turn of it.
            let (gathered, store, turn) = {
                let mut table = cov_table.write().await;
                let mut gathered = match table.gather_multiple(reading) {
                    Gathering::Ready(gathered) => gathered,
                    Gathering::FollowUp(keys) => {
                        table.revisits().request(keys);
                        return;
                    }
                    Gathering::Nothing { unclocked } => {
                        warn_unclocked(unclocked);
                        return;
                    }
                    Gathering::Busy | Gathering::StoodBack => return,
                };
                let store = table.timed().clone();
                // The turn the step took for an unconfirmed context is
                // guarded from here on: every early return ends it.
                let turn = gathered
                    .take_turn()
                    .map(|keys| SendTurn::held(&store, gathered.context(), table.revisits(), keys));
                (gathered, store, turn)
            };
            warn_unclocked(gathered.unclocked());
            // The split works on the claim as plain data, under one hold of
            // the store, and tells the histories what it gave up. From the
            // step above until every part is guarded again, below, nothing
            // awaits and nothing returns except on an exhausted budget, which
            // guards the parts to give them back. A panic in between is the
            // one way out with no guard: the plain claim unwinds with its
            // changes, where a guard gave them back. Nothing there is known
            // to panic; the claimed changes would be lost, not sent twice.
            let report = gathered.split(
                &mut store.lock(),
                crate::local_device::initiating_device(&config.device),
                u32::from(config.device.max_apdu_length()),
            );
            for key in &report.left_out {
                let property = match key {
                    bacnet_device_core::cov::CovSubscriptionKey::Multiple { property, .. } => {
                        Some(*property)
                    }
                    _ => None,
                };
                warn!(
                    object = ?key.object(),
                    property = ?property,
                    limit = report.limit,
                    "Untimestamped COV-multiple values exceed the notification size on \
                     their own; left out"
                );
            }
            // Untimestamped values a single notification carries need no
            // turn; reports of them may go out side by side, as before.
            let turn = turn.filter(|_| report.needs_turn);
            (report, store, turn)
        };

        // From the final live decision through fresh admission there is no await.
        if budget.is_exhausted() {
            counters
                .notifications_throttled_fanout
                .fetch_add(1, Ordering::Relaxed);
            // Every part goes back to its references, in order.
            for part in report.parts {
                drop(ClaimGuard::of(&store, part.claim));
            }
            return;
        }

        if report.route.issue_confirmed_notifications {
            let max_apdu_length =
                apdu::max_apdu_header_at_or_below(u32::from(config.device.max_apdu_length()))
                    .expect("validated local APDU capacity");
            // Only the oldest part goes now; the rest is deferred to the
            // follow-up its Ack owes (#986). Nothing to send when every
            // timestamped value was too large and nothing else changed.
            let Some(first) = report.into_first() else {
                return;
            };
            let notification = first.notification;
            Self::send_confirmed_cov(
                handles,
                budget,
                ConfirmedReport {
                    service: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
                    route: first.route,
                    completion: first.completion,
                    observations: first.observations,
                    claim: Some(ClaimGuard::of(&store, first.claim)),
                    deferred: first
                        .deferred
                        .into_iter()
                        .map(|claim| ClaimGuard::of(&store, claim))
                        .collect(),
                },
                |invoke_id| {
                    Self::encode_confirmed_cov_multiple_apdu(
                        &notification,
                        invoke_id,
                        max_apdu_length,
                    )
                },
            )
            .await;
        } else {
            // Every part goes out now, oldest changes first, each retired as
            // it is sent. A failure or an exhausted budget stops the rest,
            // which returns to its queue for a later notification (#986).
            let representative = report.route;
            let last_notified = report.observations;
            // Every claim is guarded before the first send is awaited.
            let parts: Vec<GuardedPart> = report
                .parts
                .into_iter()
                .map(|part| GuardedPart {
                    notification: part.notification,
                    claim: ClaimGuard::of(&store, part.claim),
                    finishes: part.finishes,
                })
                .collect();
            let mut delivered = Vec::new();
            let mut parts = parts.into_iter();
            let (mut stopped, mut sent) = (None, false);
            for part in parts.by_ref() {
                // Communication may have been restricted since the fanout began
                // (Clause 16.1). Stop; re-enabling it rearms the backstop, which
                // sends the parts left queued.
                if handles.ctx.comm_state.initiation_restricted() {
                    stopped = Some(part);
                    break;
                }
                let buf = match Self::encode_unconfirmed_cov_multiple_apdu(&part.notification) {
                    Ok(buf) => buf,
                    Err(e) => {
                        warn!(error = %e, "Failed to encode unconfirmed COVNotificationMultiple");
                        stopped = Some(part);
                        break;
                    }
                };

                if !budget.try_consume(buf.len()) {
                    counters
                        .notifications_throttled_fanout
                        .fetch_add(1, Ordering::Relaxed);
                    stopped = Some(part);
                    break;
                }

                counters.notifications_sent.fetch_add(1, Ordering::Relaxed);
                counters
                    .notifications_unconfirmed
                    .fetch_add(1, Ordering::Relaxed);
                counters
                    .notification_bytes_sent
                    .fetch_add(buf.len() as u64, Ordering::Relaxed);

                if let Err(e) = Self::send_cov_apdu(network, &buf, &representative, false).await {
                    warn!(error = %e, "Failed to send COVNotificationMultiple");
                    stopped = Some(part);
                    break;
                }
                // A reference whose latest change, or untimestamped values,
                // this part carried is complete once it is sent, whatever
                // becomes of the parts after it (#1008, #1038).
                delivered.extend(carried(
                    &part.finishes,
                    part.claim.plain(),
                    &last_notified,
                    false,
                ));
                part.claim.commit();
                sent = true;
            }
            // A report that began going out owes the untimestamped values of
            // the parts it did not send, for the backstop (#1038).
            if sent {
                for part in stopped.into_iter().chain(parts) {
                    drop(part.claim.owing());
                }
            }
            if !delivered.is_empty() {
                let mut table = cov_table.write().await;
                for (snapshot, pv, completion) in delivered {
                    table.complete_observation(&snapshot, completion, pv);
                }
            }
            drop(turn);
        }
    }
}

/// One notification of a report, its claim under the server's guard: the
/// claim goes back to its references unless the part is delivered and the
/// claim committed.
struct GuardedPart {
    notification: bacnet_services::cov_multiple::COVNotificationMultipleRequest,
    claim: ClaimGuard,
    /// References whose observation this part's delivery completes.
    finishes: BTreeSet<bacnet_device_core::cov::CovSubscriptionKey>,
}

/// Log each timestamped current-state change a fanout skipped for want of
/// a valid Device clock.
fn warn_unclocked(skipped: usize) {
    for _ in 0..skipped {
        warn!("Skipping timestamped COV-multiple change without a valid Device clock");
    }
}
