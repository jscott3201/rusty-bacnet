use super::cov_notify_context::{CovFanoutHandles, CovNotifyContext};
use super::*;
use crate::cov::InFlightAcquireError;
use bacnet_device_core::cov::{CovReport, Delivery, Evaluation, EventBudget};
use confirmed::ConfirmedSend;

mod confirmed;
mod life_safety;
mod multiple;
mod multiple_chunks;
mod multiple_items;
mod revisit;

impl<T: TransportPort + 'static> BACnetServer<T> {
    pub(super) async fn send_cov_apdu(
        network: &NetworkLayer<T>,
        apdu: &[u8],
        sub: &CovSubscription,
        expecting_reply: bool,
    ) -> Result<(), Error> {
        if let Some(ref destination) = sub.subscriber_network {
            network
                .send_apdu_routed(
                    apdu,
                    destination.network,
                    &destination.mac_address,
                    &sub.subscriber_mac,
                    expecting_reply,
                    NetworkPriority::Normal,
                )
                .await
        } else {
            network
                .send_apdu(
                    apdu,
                    &sub.subscriber_mac,
                    expecting_reply,
                    NetworkPriority::Normal,
                )
                .await
        }
    }

    fn canonical_cov_peer(
        sub: &CovSubscription,
    ) -> bacnet_endpoint_core::coordinator::CanonicalPeer {
        match &sub.subscriber_network {
            Some(destination) => {
                canonical_routed_peer(destination.network, &destination.mac_address)
            }
            None => canonical_direct_peer(&sub.subscriber_mac),
        }
    }

    /// Fire COV notifications for all active subscriptions on the given object.
    /// Skipped while DCC restricts initiation.
    pub(super) async fn fire_cov_notifications(
        ctx: &CovNotifyContext<'_, T>,
        oid: &ObjectIdentifier,
    ) {
        Self::fire_cov_notifications_inner(ctx, oid, None).await;
    }

    pub(super) async fn fire_cov_notifications_inner(
        ctx: &CovNotifyContext<'_, T>,
        oid: &ObjectIdentifier,
        snapshot: Option<&dyn bacnet_objects::traits::BACnetObject>,
    ) {
        if ctx.comm_state.initiation_restricted() {
            return;
        }
        let (subs, counters, in_flight_tracker, dispatch_turn) = {
            let mut table = ctx.cov_table.write().await;
            (
                table
                    .subscriptions_for(oid)
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>(),
                Arc::clone(table.counters()),
                Arc::clone(table.in_flight_tracker()),
                table.next_dispatch_turn(),
            )
        };

        if subs.is_empty() {
            return;
        }

        let handles = CovFanoutHandles {
            ctx,
            in_flight_tracker: &in_flight_tracker,
            counters: &counters,
        };
        Self::fire_cov_by_kind(&handles, dispatch_turn, oid, subs, snapshot, false).await;
    }

    /// Split `subs` into Single and Multiple notifications and fire both under
    /// one shared event budget. When both kinds are present the kind that goes
    /// first alternates per event and is capped at half the budget.
    async fn fire_cov_by_kind(
        handles: &CovFanoutHandles<'_, '_, T>,
        dispatch_turn: usize,
        oid: &ObjectIdentifier,
        subs: Vec<CovSubscriptionSnapshot>,
        snapshot: Option<&dyn bacnet_objects::traits::BACnetObject>,
        force: bool,
    ) {
        let (single_subs, multiple_subs): (Vec<_>, Vec<_>) = subs
            .into_iter()
            .partition(|sub| sub.notification_kind == CovNotificationKind::Single);

        let mut budget = EventBudget::new(&handles.ctx.config.cov_policy);

        if single_subs.is_empty() {
            Self::fire_cov_notification_multiple_for_subscriptions(
                handles,
                &multiple_subs,
                snapshot,
                force,
                &mut budget,
            )
            .await;
        } else if multiple_subs.is_empty() {
            Self::fire_cov_notifications_for_subscriptions(
                handles,
                oid,
                &single_subs,
                snapshot,
                force,
                &mut budget,
            )
            .await;
        } else {
            let single_first = dispatch_turn.is_multiple_of(2);
            let rem_notifs = budget.remaining_notifications();
            let first_notif_cap = (rem_notifs / 2) + (rem_notifs % 2);
            let rem_bytes = budget.remaining_bytes();
            let first_bytes_cap = (rem_bytes / 2) + (rem_bytes % 2);
            let mut first_budget = EventBudget::with_limits(first_notif_cap, first_bytes_cap);

            if single_first {
                Self::fire_cov_notifications_for_subscriptions(
                    handles,
                    oid,
                    &single_subs,
                    snapshot,
                    force,
                    &mut first_budget,
                )
                .await;
                budget.consume_sub_budget(&first_budget);

                Self::fire_cov_notification_multiple_for_subscriptions(
                    handles,
                    &multiple_subs,
                    snapshot,
                    force,
                    &mut budget,
                )
                .await;
            } else {
                Self::fire_cov_notification_multiple_for_subscriptions(
                    handles,
                    &multiple_subs,
                    snapshot,
                    force,
                    &mut first_budget,
                )
                .await;
                budget.consume_sub_budget(&first_budget);

                Self::fire_cov_notifications_for_subscriptions(
                    handles,
                    oid,
                    &single_subs,
                    snapshot,
                    force,
                    &mut budget,
                )
                .await;
            }
        }
    }

    /// Fire the initial COV notification for a newly accepted subscription.
    /// Skipped while DCC restricts initiation.
    pub(super) async fn fire_initial_cov_notification(
        ctx: &CovNotifyContext<'_, T>,
        subscription: &CovSubscriptionSnapshot,
    ) {
        if ctx.comm_state.initiation_restricted() {
            return;
        }

        let (counters, in_flight_tracker) = {
            let table = ctx.cov_table.read().await;
            if !table.is_current(subscription) {
                return;
            }
            (
                Arc::clone(table.counters()),
                Arc::clone(table.in_flight_tracker()),
            )
        };
        let mut budget = EventBudget::new(&ctx.config.cov_policy);

        Self::fire_cov_notifications_for_subscriptions(
            &CovFanoutHandles {
                ctx,
                in_flight_tracker: &in_flight_tracker,
                counters: &counters,
            },
            &subscription.monitored_object_identifier,
            std::slice::from_ref(subscription),
            None,
            true,
            &mut budget,
        )
        .await;
    }

    /// Report a change of `oid` to its ordinary and Single subscriptions
    /// `subs`, as [`evaluate`](bacnet_device_core::cov::evaluate()) decides
    /// under the database and table guards, then send each report with
    /// both dropped.
    pub(super) async fn fire_cov_notifications_for_subscriptions(
        handles: &CovFanoutHandles<'_, '_, T>,
        oid: &ObjectIdentifier,
        subs: &[CovSubscriptionSnapshot],
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
        if budget.is_exhausted() {
            return;
        }
        let evaluation = Evaluation {
            device: crate::local_device::initiating_device(&config.device),
            force,
        };
        let (reports, revisits, begun) = {
            let db = if snapshot.is_none() {
                Some(db.read().await)
            } else {
                None
            };
            let Some(object) = snapshot.or_else(|| db.as_deref()?.get(oid)) else {
                return;
            };
            // Lock order: the database (read), then the table (write), for
            // one synchronous evaluation; both drop before any send (design
            // §4.5). Tokio's RwLock is fair, so waiting here for the table
            // while holding the database is safe only because no path holds
            // a table guard and then waits for the database: every
            // acquisition takes the database first, or takes the table
            // alone and awaits nothing else under it (#2303 PR 2a audit).
            // The admission below takes leaf mutexes only (in-flight slots,
            // invoke IDs) and awaits nothing, and nothing here runs beside
            // an unpolled future that holds a guard (#1494).
            let mut table = cov_table.write().await;
            // An unconfirmed report needs nothing of the host before its send.
            let reports = table.evaluate(object, subs, evaluation, budget, |sub, _| {
                if !sub.issue_confirmed_notifications {
                    return Some(None);
                }
                Self::admit_confirmed(
                    handles,
                    sub,
                    ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
                    "COV notification",
                )
                .map(Some)
            });
            (
                reports,
                Arc::clone(table.revisits()),
                tokio::time::Instant::now(),
            )
        };

        // Every confirmed worker starts before any unconfirmed send is
        // awaited: a report's retry window runs from `begun`, the instant
        // its flight began, so a stalled send ahead of it would eat into it.
        let mut unconfirmed = Vec::with_capacity(reports.len());
        for (report, flight) in reports {
            let CovReport {
                index,
                params,
                token,
                lease,
            } = report;
            let sub = &subs[index];
            if let Some(lease) = lease {
                let flight = flight.expect("a confirmed report's flight began with its lease");
                let max_apdu_length =
                    apdu::max_apdu_header_at_or_below(u32::from(config.device.max_apdu_length()))
                        .expect("validated local APDU capacity");
                let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                    segmented: false,
                    more_follows: false,
                    segmented_response_accepted: false,
                    max_segments: None,
                    max_apdu_length,
                    invoke_id: lease.invoke_id(),
                    sequence_number: None,
                    proposed_window_size: None,
                    service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
                    service_request: params,
                });
                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");
                Self::spawn_confirmed(
                    handles,
                    ConfirmedSend {
                        lease,
                        flight,
                        buf,
                        route: sub.clone(),
                        claim: None,
                        label: "COV notification",
                        revisits: Arc::clone(&revisits),
                        begun,
                    },
                );
            } else {
                unconfirmed.push((sub, params, token));
            }
        }
        for (sub, params, token) in unconfirmed {
            let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
                service_request: params,
            });
            let mut buf = BytesMut::new();
            encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

            counters.notifications_sent.fetch_add(1, Ordering::Relaxed);
            counters
                .notifications_unconfirmed
                .fetch_add(1, Ordering::Relaxed);
            counters
                .notification_bytes_sent
                .fetch_add(buf.len() as u64, Ordering::Relaxed);

            // The observation completes once the send succeeds (#826).
            let outcome = match Self::send_cov_apdu(network, &buf, sub, false).await {
                Ok(()) => Delivery::Sent,
                Err(e) => {
                    warn!(error = %e, "Failed to send COV notification");
                    Delivery::NotSent
                }
            };
            cov_table
                .write()
                .await
                .delivered_unconfirmed(token, outcome);
        }
    }
}
