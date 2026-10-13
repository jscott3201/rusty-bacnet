use super::cov_notify_context::{CovFanoutHandles, CovNotifyContext};
use super::*;
use crate::cov::InFlightAcquireError;
use confirmed::ConfirmedReport;

mod confirmed;
mod life_safety;
mod multiple;
mod multiple_chunks;
mod multiple_items;
mod revisit;

#[derive(Debug)]
pub(super) struct EventBudget {
    max_notifications: usize,
    max_bytes: usize,
    notifications_sent: usize,
    bytes_sent: usize,
}

impl EventBudget {
    pub(super) fn new(policy: &CovPolicy) -> Self {
        Self {
            max_notifications: policy.max_notifications_per_event,
            max_bytes: policy.max_notification_bytes_per_event,
            notifications_sent: 0,
            bytes_sent: 0,
        }
    }

    pub(super) fn with_limits(max_notifications: usize, max_bytes: usize) -> Self {
        Self {
            max_notifications,
            max_bytes,
            notifications_sent: 0,
            bytes_sent: 0,
        }
    }

    pub(super) fn remaining_notifications(&self) -> usize {
        self.max_notifications
            .saturating_sub(self.notifications_sent)
    }

    pub(super) fn remaining_bytes(&self) -> usize {
        self.max_bytes.saturating_sub(self.bytes_sent)
    }

    pub(super) fn is_exhausted(&self) -> bool {
        self.notifications_sent >= self.max_notifications || self.bytes_sent >= self.max_bytes
    }

    pub(super) fn try_consume(&mut self, bytes: usize) -> bool {
        if self.notifications_sent >= self.max_notifications {
            return false;
        }
        if self.bytes_sent.saturating_add(bytes) > self.max_bytes {
            return false;
        }
        self.notifications_sent += 1;
        self.bytes_sent = self.bytes_sent.saturating_add(bytes);
        true
    }

    pub(super) fn refund(&mut self, bytes: usize) {
        self.notifications_sent = self.notifications_sent.saturating_sub(1);
        self.bytes_sent = self.bytes_sent.saturating_sub(bytes);
    }

    pub(super) fn consume_sub_budget(&mut self, sub_budget: &EventBudget) {
        self.notifications_sent = self
            .notifications_sent
            .saturating_add(sub_budget.notifications_sent);
        self.bytes_sent = self.bytes_sent.saturating_add(sub_budget.bytes_sent);
    }
}

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
                    NetworkPriority::NORMAL,
                )
                .await
        } else {
            network
                .send_apdu(
                    apdu,
                    &sub.subscriber_mac,
                    expecting_reply,
                    NetworkPriority::NORMAL,
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

        let device_oid = {
            let db = db.read().await;
            db.selected_device()
                .unwrap_or_else(|| ObjectIdentifier::new(ObjectType::DEVICE, 0).unwrap())
        };
        let ordinary = if subs.iter().any(|sub| sub.monitored_property.is_none()) {
            let db = if snapshot.is_none() {
                Some(db.read().await)
            } else {
                None
            };
            let object = match snapshot.or_else(|| db.as_deref()?.get(oid)) {
                Some(o) => o,
                None => return,
            };

            // Present_Value leads, except on an Access Point (Access_Event).
            let lead = crate::cov::reported::lead(oid.object_type());
            let prepared = (|| {
                let leading = object.read_property(lead.property(), None).ok()?;
                let sample = crate::cov::CovSample::new(&leading).ok()?;
                let flags = crate::cov::flags::PreparedFlags::read(object).ok()?;
                let reported = crate::cov::reported::PreparedReported::read(object).ok()?;
                let mut buf = BytesMut::new();
                encode_property_value(&mut buf, sample.value()).ok()?;
                let mut values = vec![BACnetPropertyValue {
                    property_identifier: lead.property(),
                    property_array_index: None,
                    value: buf.to_vec(),
                    priority: None,
                }];
                if let Some(encoded) = &flags.encoded {
                    values.push(BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::STATUS_FLAGS,
                        property_array_index: None,
                        value: encoded.clone(),
                        priority: None,
                    });
                }
                // Table 13-1 extras follow the leading value and flags, in the
                // object's order.
                values.extend(reported.values);
                let observation = flags.observation(sample).with_triggers(reported.triggers);
                let increment = object.cov_increment();
                // Reserve every ordinary reference while the shared observation
                // is still guarded, before a preceding property send can await.
                let completions = subs
                    .iter()
                    .enumerate()
                    .filter_map(|(index, sub)| {
                        if sub.monitored_property.is_some()
                            || (!force
                                && !(lead.triggers()
                                    && CovSubscriptionTable::should_notify(
                                        sub,
                                        Some(observation.sample()),
                                        sub.cov_increment.map(f64::from).or(increment),
                                    ))
                                && !observation
                                    .flags_changed(sub.last_notified_observation.as_ref())
                                && !observation
                                    .triggers_changed(sub.last_notified_observation.as_ref()))
                        {
                            return None;
                        }
                        sub.prepare_completion()
                            .map(|completion| (index, completion))
                    })
                    .collect::<HashMap<_, _>>();
                Some((values, observation, completions))
            })();
            prepared
        } else {
            None
        };

        for (index, sub) in subs.iter().enumerate() {
            let (notification_values, current_observation, completion) = if let Some(property) =
                sub.monitored_property
            {
                let db = if snapshot.is_none() {
                    Some(db.read().await)
                } else {
                    None
                };
                let Some(object) = snapshot.or_else(|| db.as_deref()?.get(oid)) else {
                    continue;
                };
                let Ok(flags) = crate::cov::flags::PreparedFlags::read(object) else {
                    continue;
                };
                let (values, observation) = if crate::cov::value_source::applies(object, property) {
                    if sub.monitored_property_array_index.is_some() {
                        continue;
                    }
                    let Ok(prepared) =
                        crate::cov::value_source::PreparedValueSource::read(object, &flags)
                    else {
                        continue;
                    };
                    if !force && !prepared.reports(sub.last_notified_observation.as_ref()) {
                        continue;
                    }
                    (prepared.values(), prepared.observation)
                } else {
                    let prepared = if property == PropertyIdentifier::STATUS_FLAGS {
                        flags.selected(object, sub.monitored_property_array_index)
                    } else {
                        object
                            .read_property(property, sub.monitored_property_array_index)
                            .and_then(|value| {
                                crate::cov::prepare::prepare_value(
                                    object,
                                    property,
                                    sub.monitored_property_array_index,
                                    sub.cov_increment,
                                    &value,
                                )
                            })
                    };
                    let Ok(prepared) = prepared else {
                        continue;
                    };
                    let observation = flags.observation(prepared.sample.clone());
                    if !force
                        && !prepared
                            .reports(sub.last_notified_observation.as_ref().map(|o| o.sample()))
                        && !observation.flags_changed(sub.last_notified_observation.as_ref())
                    {
                        continue;
                    }
                    let mut values = vec![BACnetPropertyValue {
                        property_identifier: property,
                        property_array_index: sub.monitored_property_array_index,
                        value: prepared.encoded,
                        priority: None,
                    }];
                    if property != PropertyIdentifier::STATUS_FLAGS {
                        if let Some(encoded) = flags.encoded {
                            values.push(BACnetPropertyValue {
                                property_identifier: PropertyIdentifier::STATUS_FLAGS,
                                property_array_index: None,
                                value: encoded,
                                priority: None,
                            });
                        }
                    }
                    (values, observation)
                };
                let Some(completion) = sub.prepare_completion() else {
                    continue;
                };
                (values, observation, completion)
            } else {
                let Some((values, observation, completions)) = &ordinary else {
                    continue;
                };
                let Some(completion) = completions.get(&index) else {
                    continue;
                };
                (values.clone(), observation.clone(), *completion)
            };

            // Resolve after every awaited read/callback and before fresh admission.
            // A confirmed reference with an outstanding report waits for it (#896).
            let time_remaining = {
                let table = cov_table.read().await;
                table
                    .remaining_lifetime(sub, runtime_clock::now())
                    .and_then(crate::cov::CovTimeRemaining::wire_seconds)
                    .filter(|_| table.confirmed_idle(sub))
            };
            let Some(time_remaining) = time_remaining else {
                continue;
            };
            if budget.is_exhausted() {
                counters
                    .notifications_throttled_fanout
                    .fetch_add(1, Ordering::Relaxed);
                continue;
            }

            let notification = COVNotificationRequest {
                subscriber_process_identifier: sub.subscriber_process_identifier,
                initiating_device_identifier: device_oid,
                monitored_object_identifier: *oid,
                time_remaining,
                list_of_values: notification_values,
            };

            let mut service_buf = BytesMut::new();
            notification.encode(&mut service_buf);

            if sub.issue_confirmed_notifications {
                let max_apdu_length = apdu::max_apdu_header_at_or_below(config.max_apdu_length)
                    .expect("validated local APDU capacity");
                let service_request = service_buf.freeze();
                Self::send_confirmed_cov(
                    handles,
                    budget,
                    ConfirmedReport {
                        service: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
                        route: sub.clone(),
                        completion,
                        observations: vec![(sub.clone(), current_observation)],
                        claim: None,
                        deferred: Vec::new(),
                    },
                    |invoke_id| {
                        let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                            segmented: false,
                            more_follows: false,
                            segmented_response_accepted: false,
                            max_segments: None,
                            max_apdu_length,
                            invoke_id,
                            sequence_number: None,
                            proposed_window_size: None,
                            service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
                            service_request,
                        });
                        let mut buf = BytesMut::new();
                        encode_apdu(&mut buf, &pdu)?;
                        Ok(buf)
                    },
                )
                .await;
            } else {
                let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                    service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
                    service_request: service_buf.freeze(),
                });

                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

                if !budget.try_consume(buf.len()) {
                    counters
                        .notifications_throttled_fanout
                        .fetch_add(1, Ordering::Relaxed);
                    continue;
                }

                counters.notifications_sent.fetch_add(1, Ordering::Relaxed);
                counters
                    .notifications_unconfirmed
                    .fetch_add(1, Ordering::Relaxed);
                counters
                    .notification_bytes_sent
                    .fetch_add(buf.len() as u64, Ordering::Relaxed);

                if let Err(e) = Self::send_cov_apdu(network, &buf, sub, false).await {
                    warn!(error = %e, "Failed to send COV notification");
                } else {
                    let mut table = cov_table.write().await;
                    table.complete_observation(sub, completion, current_observation);
                }
            }
        }
    }
}
