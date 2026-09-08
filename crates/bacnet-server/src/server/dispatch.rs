use super::request_admission::{Class, Rejection};
use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    async fn admit_notification_terminal(
        server_tsm: &Arc<Mutex<ServerTsm>>,
        notification_transactions: &Arc<NotificationTransactions>,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        apdu: &Apdu,
    ) -> bool {
        if !notification_transactions.admit_terminal(source_mac, source_network, apdu) {
            return false;
        }

        if let Some(source) = source_network.filter(|source| !source.mac_address.is_empty()) {
            server_tsm
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
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn dispatch(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        seg_ack_senders: &Arc<segmented_send::SegmentedSendRegistry>,
        seg_send_permits: &Arc<Semaphore>,
        cov_in_flight: &Arc<Semaphore>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        notification_transactions: &Arc<NotificationTransactions>,
        confirmed_request_tracker: &Arc<ConfirmedRequestTracker>,
        device_bindings: &Arc<RwLock<DeviceBindingTable>>,
        comm_state: &Arc<AtomicU8>,
        dcc_timer: &Arc<Mutex<Option<JoinHandle<()>>>>,
        config: &Arc<ServerConfig>,
        clock: &Option<Arc<ServerClock>>,
        discovery_limiter: &Arc<DiscoveryLimiter>,
        request_tasks: &Arc<super::request_tasks::RequestTasks>,
        source_mac: &[u8],
        apdu: Apdu,
        mut received: bacnet_network::layer::ReceivedApdu,
    ) {
        match apdu {
            Apdu::ConfirmedRequest(req) => {
                let pending = match confirmed_request_tracker.begin(
                    source_mac,
                    received.source_network.as_ref(),
                    req.clone(),
                ) {
                    ConfirmedRequestAdmission::Duplicate => return,
                    ConfirmedRequestAdmission::New(pending) => pending,
                };
                if comm_state.load(Ordering::Acquire) == 1
                    && req.service_choice != ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL
                    && req.service_choice != ConfirmedServiceChoice::REINITIALIZE_DEVICE
                {
                    // Preserve the existing DCC discard and duplicate retention.
                    pending.complete();
                    return;
                }
                let invoke_id = req.invoke_id;
                let service_choice = req.service_choice;
                let abort_comm_state = Arc::clone(comm_state);
                let abort_network = Arc::clone(network);
                let abort_mac = MacAddr::from_slice(source_mac);
                let abort_source = received.source_network.clone();
                let mut reply_tx = received.reply_tx.take();
                let db = Arc::clone(db);
                let network = Arc::clone(network);
                let cov_table = Arc::clone(cov_table);
                let seg_ack_senders = Arc::clone(seg_ack_senders);
                let seg_send_permits = Arc::clone(seg_send_permits);
                let cov_in_flight = Arc::clone(cov_in_flight);
                let server_tsm = Arc::clone(server_tsm);
                let notification_transactions = Arc::clone(notification_transactions);
                let device_bindings = Arc::clone(device_bindings);
                let comm_state = Arc::clone(comm_state);
                let dcc_timer = Arc::clone(dcc_timer);
                let config = Arc::clone(config);
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
                            &db,
                            &network,
                            &cov_table,
                            &seg_ack_senders,
                            &seg_send_permits,
                            &cov_in_flight,
                            &server_tsm,
                            &notification_transactions,
                            &device_bindings,
                            &comm_state,
                            &dcc_timer,
                            &config,
                            &descendants,
                            &source_mac,
                            source_network,
                            req,
                            reply_tx,
                        )
                        .await;
                        pending.complete();
                    }
                });
                if result == Err(Rejection::Overloaded) {
                    // ASHRAE 135-2020 §§5.4.5.3, 18.10: resource exhaustion
                    // before service execution is reported by a server Abort.
                    // Eight owned sends bound this response work. If all are
                    // busy, the counted silent drop is a known local limitation.
                    let _ = request_tasks.try_spawn(Class::Abort, peer, || async move {
                        // Match the handler's first-poll DCC check as well as
                        // dispatch's precheck if DCC changed after registration.
                        if abort_comm_state.load(Ordering::Acquire) == 1
                            && service_choice
                                != ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL
                            && service_choice != ConfirmedServiceChoice::REINITIALIZE_DEVICE
                        {
                            return;
                        }
                        requests::confirmed_response::send_overload_response(
                            &abort_network,
                            &Apdu::Abort(AbortPdu {
                                sent_by_server: true,
                                invoke_id,
                                abort_reason: AbortReason::OUT_OF_RESOURCES,
                            }),
                            &abort_mac,
                            abort_source.as_ref(),
                            reply_tx,
                        )
                        .await;
                    });
                }
            }
            Apdu::UnconfirmedRequest(req) => {
                let comm = comm_state.load(Ordering::Acquire);
                if comm == 1 {
                    debug!("Dropping unconfirmed service: DCC is DISABLE");
                    return;
                }

                let now = Instant::now();
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
                        PreCheckDecision::DecodeError => {}
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
                        PreCheckDecision::DecodeError => {}
                    }
                }

                let db = Arc::clone(db);
                let network = Arc::clone(network);
                let config = Arc::clone(config);
                let clock = clock.clone();
                let comm_state = Arc::clone(comm_state);
                let device_bindings = Arc::clone(device_bindings);
                let discovery_limiter = Arc::clone(discovery_limiter);
                let peer = super::request_peer::canonical_requester(
                    source_mac,
                    received.source_network.as_ref(),
                );
                let _ = request_tasks.try_spawn(Class::Unconfirmed, peer, || async move {
                    Self::handle_unconfirmed_request(
                        &db,
                        &network,
                        &config,
                        clock.as_ref(),
                        &comm_state,
                        &device_bindings,
                        &discovery_limiter,
                        req,
                        &received,
                    )
                    .await;
                });
            }
            // Fast paths — remain inline (bounded synchronous admission)
            Apdu::SimpleAck(sa) => {
                let invoke_id = sa.invoke_id;
                let admitted = Self::admit_notification_terminal(
                    server_tsm,
                    notification_transactions,
                    source_mac,
                    received.source_network.as_ref(),
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
                    server_tsm,
                    notification_transactions,
                    source_mac,
                    received.source_network.as_ref(),
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
                    server_tsm,
                    notification_transactions,
                    source_mac,
                    received.source_network.as_ref(),
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
                    );
                    Self::route_segmented_send_event(
                        seg_ack_senders,
                        key,
                        SegmentedSendEvent::Abort(abort.clone()),
                    )
                    .await
                };
                let admitted = Self::admit_notification_terminal(
                    server_tsm,
                    notification_transactions,
                    source_mac,
                    received.source_network.as_ref(),
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
                    server_tsm,
                    notification_transactions,
                    source_mac,
                    received.source_network.as_ref(),
                    &Apdu::ComplexAck(ack),
                )
                .await;
                debug!(
                    invoke_id,
                    admitted, "Server ignoring ComplexAck for confirmed notification"
                );
            }
        }
    }
}
