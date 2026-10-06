use super::request_admission::{Class, Rejection};
use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Admit an answer to a confirmed request this server sent. With
    /// `local_network`, this network's number, known, an answer relayed with
    /// it as SNET answers a request sent to its SADR (#1465), and teaches no
    /// next hop: sends to this network go out locally.
    async fn admit_notification_terminal(
        learned_routers: &Arc<Mutex<LearnedRouterCache>>,
        notification_transactions: &Arc<NotificationTransactions>,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        local_network: Option<u16>,
        apdu: &Apdu,
    ) -> bool {
        if !notification_transactions.admit_terminal(
            source_mac,
            source_network,
            local_network,
            apdu,
        ) {
            return false;
        }

        if let Some(source) = source_network.filter(|source| {
            !source.mac_address.is_empty() && Some(source.network) != local_network
        }) {
            learned_routers
                .lock()
                .await
                .learn_router(source.network, &MacAddr::from_slice(source_mac));
        }
        true
    }

    async fn route_segmented_send_event(
        seg_ack_senders: &Arc<segmented_send::SegmentedSendRegistry>,
        key: SegKey,
        event: SegmentedSendEvent,
    ) -> bool {
        let handle = {
            let senders = seg_ack_senders.lock();
            senders.get(&key).cloned()
        };

        let Some(handle) = handle else {
            return false;
        };

        match event {
            SegmentedSendEvent::Abort(abort) => {
                handle.send_control(SegmentedSendControlEvent::Abort(abort));
                true
            }
            SegmentedSendEvent::SegmentAck(ack) => {
                let invoke_id = ack.invoke_id;
                let seq = ack.sequence_number;
                if !handle.accepts_segment_ack(&ack) {
                    debug!(
                        invoke_id,
                        seq,
                        negative = ack.negative_ack,
                        "Ignoring SegmentAck outside current segmented-send window"
                    );
                    return true;
                }

                match handle.segment_ack_tx.try_send(ack) {
                    Ok(()) => true,
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        warn!(
                            invoke_id,
                            seq, "Dropping SegmentAck because segmented-send queue is full"
                        );
                        true
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => false,
                }
            }
        }
    }

    /// Dispatch a received APDU.
    ///
    /// ConfirmedRequest and UnconfirmedRequest handlers are spawned as
    /// independent tasks so the dispatch loop can immediately process the
    /// next incoming APDU.  This allows the server to handle multiple
    /// client requests concurrently (e.g. concurrent ReadProperty via
    /// the RwLock on ObjectDatabase).
    ///
    /// Fast-path APDU types (SimpleAck, Error, Reject, Abort, SegmentAck)
    /// remain inline since they are sub-microsecond TSM lookups.
    pub(super) async fn dispatch(
        ctx: &DispatchContext<T>,
        source_mac: &[u8],
        apdu: Apdu,
        mut received: bacnet_network::layer::ReceivedApdu,
    ) {
        let DispatchContext {
            services,
            confirmed_request_tracker,
            clock: _,
            discovery_limiter,
            time_sync_limiter: _,
            request_tasks,
        } = ctx;
        let RequestServices {
            network,
            seg_ack_senders,
            learned_routers,
            notification_transactions,
            ..
        } = services;
        if notification_transactions
            .application_sealed
            .load(Ordering::Acquire)
            && matches!(
                apdu,
                Apdu::ConfirmedRequest(_) | Apdu::UnconfirmedRequest(_)
            )
        {
            return;
        }
        // Ingress provenance travels with `received` into request diagnostics
        // and authorization. NotificationTransactions owns outgoing terminal
        // admission; learned_routers stores only next hops from admitted replies.
        let route = received.response_route();
        match apdu {
            Apdu::ConfirmedRequest(req) => {
                // LSO-only replay path (server level, separate budget).
                // Retransmitted already-executed confirmed LSO within the
                // window resends byte-identical bytes with a single execution.
                // Pending in-flight duplicates preserve DISCARD. This is a
                // local service-specific extension, not a Standard mandate.
                if req.service_choice == ConfirmedServiceChoice::LIFE_SAFETY_OPERATION {
                    let lso_pending = match confirmed_request_tracker.lso.begin(
                        source_mac,
                        received.source_network.as_ref(),
                        received.provenance,
                        req.clone(),
                    ) {
                        LsoAdmission::Replay(bytes) => {
                            let reply_tx = received.reply_tx.take();
                            let replay_mac = MacAddr::from_slice(source_mac);
                            let replay_network = received.source_network.clone();
                            requests::confirmed_response::send_replay_bytes(
                                network,
                                &bytes,
                                &replay_mac,
                                replay_network.as_ref(),
                                &route,
                                reply_tx,
                            )
                            .await;
                            return;
                        }
                        LsoAdmission::DuplicatePending => return,
                        LsoAdmission::New(pending) => pending,
                    };
                    let invoke_id = req.invoke_id;
                    let abort_network = Arc::clone(network);
                    let abort_mac = MacAddr::from_slice(source_mac);
                    let abort_source = received.source_network.clone();
                    let abort_route = route.clone();
                    let mut reply_tx = received.reply_tx.take();
                    let services = services.clone();
                    let source_mac = MacAddr::from_slice(source_mac);
                    let source_network = received.source_network.clone();
                    let descendants = request_tasks.spawner();
                    let peer = super::request_peer::canonical_requester(
                        &source_mac,
                        source_network.as_ref(),
                    );
                    let class = super::request_admission::confirmed_class(
                        req.service_choice,
                        &req.service_request,
                    );
                    let result = request_tasks.try_spawn(class, peer.clone(), || {
                        let reply_tx = reply_tx.take();
                        async move {
                            Self::handle_admitted_confirmed_request(
                                &services,
                                &descendants,
                                RequestOrigin {
                                    mac: &source_mac,
                                    network: source_network,
                                    route,
                                },
                                req,
                                reply_tx,
                                Some(ConfirmedRequestOwnership::LifeSafety(lso_pending)),
                            )
                            .await;
                        }
                    });
                    if result == Err(Rejection::Overloaded) {
                        let _ = request_tasks.try_spawn(Class::Abort, peer, || async move {
                            requests::confirmed_response::send_overload_response(
                                &abort_network,
                                &Apdu::Abort(AbortPdu {
                                    sent_by_server: true,
                                    invoke_id,
                                    abort_reason: AbortReason::OUT_OF_RESOURCES,
                                }),
                                &abort_mac,
                                abort_source.as_ref(),
                                &abort_route,
                                reply_tx,
                            )
                            .await;
                        });
                    }
                    return;
                }
                let pending = match confirmed_request_tracker.begin(
                    source_mac,
                    received.source_network.as_ref(),
                    received.provenance,
                    req.clone(),
                ) {
                    ConfirmedRequestAdmission::Duplicate => return,
                    ConfirmedRequestAdmission::New(pending) => pending,
                };
                let invoke_id = req.invoke_id;
                let abort_network = Arc::clone(network);
                let abort_mac = MacAddr::from_slice(source_mac);
                let abort_source = received.source_network.clone();
                let abort_route = route.clone();
                let mut reply_tx = received.reply_tx.take();
                let services = services.clone();
                let source_mac = MacAddr::from_slice(source_mac);
                let source_network = received.source_network.clone();
                let descendants = request_tasks.spawner();
                let peer =
                    super::request_peer::canonical_requester(&source_mac, source_network.as_ref());
                let class = super::request_admission::confirmed_class(
                    req.service_choice,
                    &req.service_request,
                );
                let result = request_tasks.try_spawn(class, peer.clone(), || {
                    let reply_tx = reply_tx.take();
                    async move {
                        Self::handle_admitted_confirmed_request(
                            &services,
                            &descendants,
                            RequestOrigin {
                                mac: &source_mac,
                                network: source_network,
                                route,
                            },
                            req,
                            reply_tx,
                            Some(ConfirmedRequestOwnership::Generic(pending)),
                        )
                        .await;
                    }
                });
                if result == Err(Rejection::Overloaded) {
                    // ASHRAE 135-2020 §§5.4.5.3, 18.10: resource exhaustion
                    // before service execution is reported by a server Abort.
                    // Eight owned sends bound this response work. If all are
                    // busy, the counted silent drop is a known local limitation.
                    let _ = request_tasks.try_spawn(Class::Abort, peer, || async move {
                        requests::confirmed_response::send_overload_response(
                            &abort_network,
                            &Apdu::Abort(AbortPdu {
                                sent_by_server: true,
                                invoke_id,
                                abort_reason: AbortReason::OUT_OF_RESOURCES,
                            }),
                            &abort_mac,
                            abort_source.as_ref(),
                            &abort_route,
                            reply_tx,
                        )
                        .await;
                    });
                }
            }
            Apdu::UnconfirmedRequest(req) => {
                let now = runtime_clock::now();
                if req.service_choice == UnconfirmedServiceChoice::WHO_IS {
                    match discovery_limiter.pre_check_who_is(&req.service_request, &received, now) {
                        PreCheckDecision::Admit => {}
                        PreCheckDecision::OutOfRange => {
                            debug!("WhoIs instance range does not include local device; dropped");
                            return;
                        }
                        PreCheckDecision::Coalesced => {
                            debug!("WhoIs duplicate coalesced within window");
                            return;
                        }
                        PreCheckDecision::ThrottledSource => {
                            debug!("WhoIs throttled: source rate/byte limit exceeded");
                            return;
                        }
                        PreCheckDecision::ThrottledGlobal => {
                            debug!("WhoIs throttled: global rate/byte limit exceeded");
                            return;
                        }
                        // Nothing can answer it: drop it here, before a
                        // task is spawned and it is decoded again.
                        PreCheckDecision::DecodeError => {
                            debug!("Malformed WhoIs dropped");
                            return;
                        }
                    }
                } else if req.service_choice == UnconfirmedServiceChoice::WHO_HAS {
                    match discovery_limiter.pre_check_who_has(&req.service_request, &received, now)
                    {
                        PreCheckDecision::Admit => {}
                        PreCheckDecision::OutOfRange => {
                            debug!("WhoHas instance range does not include local device; dropped");
                            return;
                        }
                        PreCheckDecision::Coalesced => {
                            debug!("WhoHas duplicate/negative coalesced within window");
                            return;
                        }
                        PreCheckDecision::ThrottledSource => {
                            debug!("WhoHas throttled: source rate/byte limit exceeded");
                            return;
                        }
                        PreCheckDecision::ThrottledGlobal => {
                            debug!("WhoHas throttled: global rate/byte limit exceeded");
                            return;
                        }
                        PreCheckDecision::DecodeError => {
                            debug!("Malformed WhoHas dropped");
                            return;
                        }
                    }
                }

                let services = ctx.unconfirmed_services();
                let peer = super::request_peer::canonical_requester(
                    source_mac,
                    received.source_network.as_ref(),
                );
                let _ = request_tasks.try_spawn(Class::Unconfirmed, peer, || async move {
                    Self::handle_unconfirmed_request(&services, req, &received).await;
                });
            }
            // Fast paths — remain inline (bounded synchronous admission)
            Apdu::SimpleAck(sa) => {
                let invoke_id = sa.invoke_id;
                let admitted = Self::admit_notification_terminal(
                    learned_routers,
                    notification_transactions,
                    source_mac,
                    received.source_network.as_ref(),
                    network.local_network_number().get(),
                    &Apdu::SimpleAck(sa),
                )
                .await;
                debug!(
                    invoke_id,
                    admitted, "SimpleAck received for outgoing confirmed notification"
                );
            }
            Apdu::Error(err) => {
                let invoke_id = err.invoke_id;
                let error_class = err.error_class.to_raw();
                let error_code = err.error_code.to_raw();
                let admitted = Self::admit_notification_terminal(
                    learned_routers,
                    notification_transactions,
                    source_mac,
                    received.source_network.as_ref(),
                    network.local_network_number().get(),
                    &Apdu::Error(err),
                )
                .await;
                debug!(
                    invoke_id,
                    error_class,
                    error_code,
                    admitted,
                    "Error received for outgoing confirmed notification"
                );
            }
            Apdu::Reject(rej) => {
                let invoke_id = rej.invoke_id;
                let admitted = Self::admit_notification_terminal(
                    learned_routers,
                    notification_transactions,
                    source_mac,
                    received.source_network.as_ref(),
                    network.local_network_number().get(),
                    &Apdu::Reject(rej),
                )
                .await;
                debug!(
                    invoke_id,
                    admitted, "Reject received for outgoing confirmed notification"
                );
            }
            Apdu::Abort(abort) => {
                let invoke_id = abort.invoke_id;
                let routed_segmented = if abort.sent_by_server {
                    false
                } else {
                    let key = segmented_transaction_key(
                        source_mac,
                        received.source_network.as_ref(),
                        invoke_id,
                        route.provenance(),
                    );
                    Self::route_segmented_send_event(
                        seg_ack_senders,
                        key,
                        SegmentedSendEvent::Abort(abort.clone()),
                    )
                    .await
                };
                let admitted = Self::admit_notification_terminal(
                    learned_routers,
                    notification_transactions,
                    source_mac,
                    received.source_network.as_ref(),
                    network.local_network_number().get(),
                    &Apdu::Abort(abort),
                )
                .await;
                debug!(
                    invoke_id,
                    routed_segmented,
                    admitted,
                    "Abort received for outgoing confirmed notification or segmented response"
                );
            }
            Apdu::SegmentAck(sa) => {
                let invoke_id = sa.invoke_id;
                if sa.sent_by_server {
                    debug!(
                        invoke_id,
                        seq = sa.sequence_number,
                        "Server ignoring SegmentAck with server bit set"
                    );
                    return;
                }

                let key = segmented_transaction_key(
                    source_mac,
                    received.source_network.as_ref(),
                    invoke_id,
                    route.provenance(),
                );
                if !Self::route_segmented_send_event(
                    seg_ack_senders,
                    key,
                    SegmentedSendEvent::SegmentAck(sa),
                )
                .await
                {
                    debug!(
                        invoke_id,
                        "Server ignoring SegmentAck for unknown transaction"
                    );
                }
            }
            Apdu::ComplexAck(ack) => {
                let invoke_id = ack.invoke_id;
                let admitted = Self::admit_notification_terminal(
                    learned_routers,
                    notification_transactions,
                    source_mac,
                    received.source_network.as_ref(),
                    network.local_network_number().get(),
                    &Apdu::ComplexAck(ack),
                )
                .await;
                if admitted {
                    debug!(invoke_id, "ComplexAck answered a read this server sent");
                } else {
                    debug!(invoke_id, "Server ignoring ComplexAck it has no read for");
                }
            }
        }
    }
}
