//! The one EventNotification send path: each notification this server
//! originates, and each copy its Notification Forwarders send on, reaches
//! its recipients through [`BACnetServer::send_event_notification`].

use super::event_forwarding::ForwardingBudget;
use super::event_recipient_route::{
    ConfirmedRecipientRoute, ConfirmedRouteRefusal, RecipientRoute, network_priority_for_event,
};
use super::event_suppression::EventSuppression;
use super::notification_transactions::{Attempt, NotificationReserveError, run_attempts};
use super::*;
use bacnet_types::constructed::BACnetRecipient;

#[path = "event_awaited_recipients.rs"]
mod awaited;
pub(super) use awaited::AwaitingRecipients;
use awaited::DeviceRoute;

/// Why a confirmed notification ended at an attempt with nothing sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Withdrawal {
    /// DeviceCommunicationControl restricts initiation (#1327): not counted.
    InitiationRestricted,
    /// The Device recipient's observed binding expired before this attempt
    /// (#1371): counted in `device_recipient_unbound`.
    BindingLapsed,
}

/// APDU header octets before the service request of an unsegmented
/// Confirmed-Request (type, segmentation limits, invoke ID, service choice)
/// and of an Unconfirmed-Request (type, service choice).
const CONFIRMED_HEADER_LEN: usize = 4;
const UNCONFIRMED_HEADER_LEN: usize = 2;

/// The service request a notification sends to one process identifier.
pub(super) type EncodeFor<'a> = &'a (dyn Fn(u32) -> Result<Bytes, Error> + Send + Sync);

/// Whether a notification may take a resolved route. Shared, so a
/// notification that waits for its device's I-Am keeps it (#1368).
pub(super) type AdmitsRoute = Arc<dyn Fn(&RecipientRoute) -> bool + Send + Sync>;

/// One event notification on its way to its recipients.
pub(super) struct OutboundNotification<'a> {
    /// The notification class, named in diagnostics.
    pub(super) notification_class: u32,
    /// The event priority, which sets the NPDU priority of every send and
    /// retry (Clause 13.2.5.4).
    pub(super) priority: u8,
    /// The request for each destination's process identifier.
    pub(super) encode_for: EncodeFor<'a>,
    /// Routes the notification may take. A refused route is skipped and not
    /// counted; the server's own notifications admit every route.
    pub(super) admits: AdmitsRoute,
    /// The forwarding cap a forwarded copy draws on, once it has passed
    /// every other check; `None` for the server's own notifications.
    pub(super) budget: Option<Arc<ForwardingBudget>>,
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Send `outbound` to each recipient, confirmed or not as the recipient
    /// asks. A recipient whose route cannot carry the notification is
    /// skipped and counted in [`EventNotificationCounters`]; the rest are
    /// still served. Nothing is sent while DeviceCommunicationControl
    /// restricts initiation, a confirmed notification's retries included.
    ///
    /// A Device recipient the server holds no fresh binding for is looked
    /// for with a targeted Who-Is (#1368), and its notification waits for the
    /// I-Am in its device's queue, which a task of its own drains, so the
    /// other recipients' notifications go out at once. While the queue holds
    /// anything, the device's next notifications join it, in order.
    pub(super) async fn send_event_notification(
        ctx: &EventDelivery<'_, T>,
        outbound: &OutboundNotification<'_>,
        recipients: &[(BACnetRecipient, u32, bool)],
    ) {
        if ctx.comm_state.initiation_restricted() {
            return;
        }
        let network = ctx.network;
        // One reading for every destination of this notification, taken
        // without the database lock (#1298).
        let local_network = network.local_network_number().get();
        // The link's own broadcast routes an Address recipient; any group
        // address takes no binding and no confirmed request (#1493).
        let is_link_broadcast = |mac: &[u8]| network.transport().is_broadcast_mac(mac);
        let is_group = |mac: &[u8]| network.transport().is_group_destination(mac);

        for &(ref recipient, process_id, confirmed) in recipients {
            let route = match recipient {
                BACnetRecipient::Address(address) => {
                    RecipientRoute::resolve_address(address, is_link_broadcast)
                }
                BACnetRecipient::Device(identifier) => {
                    match Self::device_route(ctx, outbound, *identifier, process_id, confirmed)
                        .await
                    {
                        DeviceRoute::Now(resolution) => {
                            RecipientRoute::from_device_resolution(resolution)
                        }
                        DeviceRoute::Queued | DeviceRoute::Done => continue,
                    }
                }
            }
            // A recipient on this network by number is sent to as a local
            // one (#1299).
            .localize(local_network, is_link_broadcast, is_group);
            Self::send_on_route(ctx, outbound, route, process_id, confirmed).await;
        }
    }

    /// Send `outbound` to one recipient, `process_id`, along `route`, its
    /// resolved and localized route, confirmed or not as it asks: the second
    /// half of [`Self::send_event_notification`], which a notification that
    /// waited for its device's I-Am takes once it has a route.
    pub(super) async fn send_on_route(
        ctx: &EventDelivery<'_, T>,
        outbound: &OutboundNotification<'_>,
        route: RecipientRoute,
        process_id: u32,
        confirmed: bool,
    ) {
        let &EventDelivery {
            db: _,
            network,
            comm_state,
            learned_routers,
            notification_transactions,
            device_bindings: _,
            suppressions,
            retry_timeout_ms,
            local_apdu_capacity,
        } = ctx;
        let notification_class = outbound.notification_class;
        let network_priority = network_priority_for_event(outbound.priority);
        let is_group = |mac: &[u8]| network.transport().is_group_destination(mac);

        // A route that can't carry this notification is skipped and
        // counted (#1160); the remaining destinations are still served.
        if let Some(skip) = route.skip(confirmed, notification_class) {
            suppressions.record(skip);
            return;
        }
        // A confirmed one goes to one device, never to a group address
        // such as a multicast one, where an unconfirmed one may still go.
        // It is counted with the confirmed broadcast recipients (#1493).
        let confirmed_route = if confirmed {
            match route.clone().into_confirmed(is_group) {
                Ok(confirmed_route) => Some(confirmed_route),
                Err(ConfirmedRouteRefusal::GroupNextHop) => {
                    suppressions.record(EventSuppression::ConfirmedBroadcastRecipient);
                    warn!(
                        notification_class,
                        "Recipient requests confirmed notifications at a group address; \
                         a confirmed request goes to one device, skipping"
                    );
                    return;
                }
                // `skip` let only unicast routes through, so this means
                // the route classification changed: fail closed.
                Err(ConfirmedRouteRefusal::NotOneDevice) => {
                    warn!(
                        notification_class,
                        "Confirmed notification route is unusable"
                    );
                    return;
                }
            }
        } else {
            None
        };
        // A forwarded notification keeps off the routes Clause 12.51's
        // loop rules close to it. That is configured behaviour, not a
        // failed delivery, so nothing is counted.
        if !(outbound.admits)(&route) {
            debug!(
                notification_class,
                "Forwarding rule keeps the notification off this route"
            );
            return;
        }

        // Cannot fail for a notification that reached this loop: a local
        // transition's payload and message text were validated when it
        // was committed, and a forwarded copy only swaps the process
        // identifier of a request that already decoded. See
        // `EventNotificationCounters` for why nothing counts it.
        let service_bytes = match (outbound.encode_for)(process_id) {
            Ok(bytes) => bytes,
            Err(e) => {
                warn!(error = %e, "Failed to encode EventNotification");
                return;
            }
        };
        // Notifications go unsegmented, so one longer than the local APDU
        // capacity is not sent; a forwarded copy of a notification that
        // arrived segmented is the usual case. It is checked before an
        // invoke ID is reserved.
        let header = if confirmed {
            CONFIRMED_HEADER_LEN
        } else {
            UNCONFIRMED_HEADER_LEN
        };
        let capacity = usize::try_from(local_apdu_capacity).unwrap_or(usize::MAX);
        if header + service_bytes.len() > capacity {
            suppressions.record(EventSuppression::ApduTooLarge);
            warn!(
                notification_class,
                length = header + service_bytes.len(),
                capacity,
                "EventNotification is longer than the local APDU capacity; not sent"
            );
            return;
        }
        // A forwarded copy that would go out draws on its notification's
        // cap (#1259); one past the cap is dropped and counted.
        if outbound
            .budget
            .as_ref()
            .is_some_and(|budget| !budget.take())
        {
            suppressions.record(EventSuppression::ForwardingCapDropped);
            return;
        }

        if let Some(ConfirmedRecipientRoute {
            canonical_peer,
            local_target,
            remote,
            freshness,
        }) = confirmed_route
        {
            let (operation, result_rx) = match notification_transactions.reserve(
                canonical_peer,
                ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
            ) {
                Ok(reservation) => reservation,
                // Stopping closes the adapter, which is not a delivery
                // failure, so it is not counted.
                Err(NotificationReserveError::Closed) => {
                    debug!("Server stopping; confirmed EventNotification not sent");
                    return;
                }
                Err(error) => {
                    suppressions.record(EventSuppression::ConfirmedNoInvokeId);
                    warn!(%error, "No free invoke ID for confirmed EventNotification");
                    return;
                }
            };
            let id = operation.invoke_id();

            let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: false,
                max_segments: None,
                max_apdu_length: apdu::max_apdu_header_at_or_below(local_apdu_capacity)
                    .expect("validated local APDU capacity"),
                invoke_id: id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
                service_request: service_bytes,
            });

            let mut buf = BytesMut::new();
            encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

            let network = Arc::clone(network);
            let comm_state = Arc::clone(comm_state);
            let learned_routers = Arc::clone(learned_routers);
            let suppressions = Arc::clone(suppressions);
            let timeout = Duration::from_millis(retry_timeout_ms);
            let apdu_retries = DEFAULT_APDU_RETRIES;
            notification_transactions.spawn(async move {
                let result = run_attempts(operation, result_rx, timeout, apdu_retries, |attempt| {
                    // DCC and the binding's lifetime are checked
                    // before every attempt, the first and each retry,
                    // and either ends the notification there, its
                    // invoke ID freed (#1327, #1371).
                    let withdrawn = if comm_state.initiation_restricted() {
                        Some(Withdrawal::InitiationRestricted)
                    } else if freshness.is_some_and(|freshness| {
                        !freshness.permits_attempt_at(tokio::time::Instant::now())
                    }) {
                        Some(Withdrawal::BindingLapsed)
                    } else {
                        None
                    };
                    let network = Arc::clone(&network);
                    let learned_routers = Arc::clone(&learned_routers);
                    let buf = buf.clone();
                    let local_target = local_target.clone();
                    let remote = remote.clone();
                    async move {
                        if let Some(withdrawal) = withdrawn {
                            return Attempt::Withdrawn(withdrawal);
                        }
                        let send_result = match (local_target, remote) {
                            (Some(target), None) => {
                                network
                                    .send_apdu(&buf, &target, true, network_priority)
                                    .await
                            }
                            (None, Some((dnet, dadr, configured_router))) => {
                                // A Device binding keeps its fixed next hop for
                                // each permitted attempt. Address recipients retain
                                // the learned-router/broadcast behavior.
                                let router = match configured_router {
                                    Some(router) => Some(router),
                                    None if attempt == 0 => {
                                        learned_routers.lock().await.cached_router(dnet)
                                    }
                                    None => None,
                                };
                                match router {
                                    Some(router_mac) => {
                                        network
                                            .send_apdu_routed(
                                                &buf,
                                                dnet,
                                                &dadr,
                                                &router_mac,
                                                true,
                                                network_priority,
                                            )
                                            .await
                                    }
                                    None => {
                                        network
                                            .send_apdu_routed_via_local_broadcast(
                                                &buf,
                                                dnet,
                                                &dadr,
                                                true,
                                                network_priority,
                                            )
                                            .await
                                    }
                                }
                            }
                            _ => unreachable!("confirmed route validated before spawn"),
                        };
                        match &send_result {
                            Ok(()) => {
                                debug!(invoke_id = id, attempt, "Confirmed EventNotification sent")
                            }
                            Err(error) => warn!(
                                %error,
                                attempt, "Confirmed EventNotification send failed"
                            ),
                        }
                        match send_result {
                            Ok(()) => Attempt::Sent,
                            Err(_) => Attempt::NotSent,
                        }
                    }
                })
                .await;
                match result.map(NotificationWorkerResult::from) {
                    Ok(NotificationWorkerResult::Ack) => {
                        debug!(invoke_id = id, "EventNotification acknowledged");
                    }
                    Ok(NotificationWorkerResult::Error(_)) => {
                        suppressions.record(EventSuppression::ConfirmedRejected);
                        warn!(invoke_id = id, "EventNotification rejected by recipient");
                    }
                    Ok(NotificationWorkerResult::Exhausted) => {
                        suppressions.record(EventSuppression::ConfirmedUnanswered);
                        warn!(
                            invoke_id = id,
                            "EventNotification failed after {} retries", apdu_retries
                        );
                    }
                    Ok(NotificationWorkerResult::Closed) => {}
                    // Held back by DCC, as a notification it stops before
                    // the first send is: not counted, and not sent again
                    // once initiation is enabled.
                    Err(Withdrawal::InitiationRestricted) => debug!(
                        invoke_id = id,
                        "EventNotification withdrawn: DCC restricts initiation"
                    ),
                    // The device's observed binding ran out before a
                    // retry: the server no longer holds an address for
                    // it, as when the first send finds none, so it counts
                    // where that skip does.
                    Err(Withdrawal::BindingLapsed) => {
                        suppressions.record(EventSuppression::DeviceRecipientUnbound);
                        warn!(
                            invoke_id = id,
                            "EventNotification withdrawn: the recipient's observed \
                             binding expired before a retry"
                        );
                    }
                }
            });
        } else {
            let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                service_choice: UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION,
                service_request: service_bytes,
            });

            let mut buf = BytesMut::new();
            encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

            let send_result = match &route {
                RecipientRoute::LocalUnicast(mac) => {
                    network.send_apdu(&buf, mac, false, network_priority).await
                }
                RecipientRoute::BoundLocalUnicast { mac, .. } => {
                    network.send_apdu(&buf, mac, false, network_priority).await
                }
                RecipientRoute::LocalBroadcast => {
                    network.broadcast_apdu(&buf, false, network_priority).await
                }
                // Carries DNET with DLEN zero, so routers forward it
                // onto the remote network as a broadcast there.
                RecipientRoute::RemoteBroadcast(net) => {
                    network
                        .broadcast_to_network(&buf, *net, false, network_priority)
                        .await
                }
                // Carries DNET 0xFFFF, which routers forward to every
                // reachable network. `broadcast_to_network` rejects that
                // DNET, so it needs its own send.
                RecipientRoute::GlobalBroadcast => {
                    network
                        .broadcast_global_apdu(&buf, false, network_priority)
                        .await
                }
                // DNET/DADR name the recipient; the link DA is the local
                // broadcast because this non-routing device keeps no
                // router table (Clause 6.5.3's unknown-router form).
                RecipientRoute::RemoteUnicast { network: net, mac } => {
                    network
                        .send_apdu_routed_via_local_broadcast(
                            &buf,
                            *net,
                            mac,
                            false,
                            network_priority,
                        )
                        .await
                }
                RecipientRoute::BoundRoutedUnicast {
                    network: net,
                    mac,
                    router,
                    ..
                } => {
                    network
                        .send_apdu_routed(&buf, *net, mac, router, false, network_priority)
                        .await
                }
                // Skipped by `RecipientRoute::skip` above.
                RecipientRoute::ContradictoryGlobal
                | RecipientRoute::UnknownDevice
                | RecipientRoute::StaleDevice
                | RecipientRoute::InvalidDevice => return,
            };

            if let Err(e) = send_result {
                suppressions.record(EventSuppression::UnconfirmedSendFailed);
                warn!(
                    error = %e,
                    "Failed to send unconfirmed EventNotification"
                );
            }
        }
    }
}
