use super::*;
#[path = "intrinsic_reporting_lifecycle.rs"]
mod intrinsic;
use crate::committed_cov::BackgroundCommit;

#[path = "lifecycle_period.rs"]
mod period;
use super::heap_futures::boxed;
use super::{audit_recipient::spawn_owned, audit_recipient_routes::AuditRoutes};
pub(super) use period::{event_enrollment_period, MonotonicClocks};

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Start a server: every public start and build path ends here. The
    /// startup future is on the heap, which keeps theirs small (#953).
    pub(super) fn start_with_clock_mode_and_bindings(
        config: ServerConfig,
        db: ObjectDatabase,
        transport: T,
        clock_config: Option<ClockConfig>,
        configured_device_bindings: Vec<DeviceBinding>,
    ) -> std::pin::Pin<Box<impl std::future::Future<Output = Result<Self, Error>>>> {
        boxed(|| {
            Self::start_on_heap(
                config,
                db,
                transport,
                clock_config,
                configured_device_bindings,
            )
        })
    }

    async fn start_on_heap(
        mut config: ServerConfig,
        mut db: ObjectDatabase,
        transport: T,
        clock_config: Option<ClockConfig>,
        configured_device_bindings: Vec<DeviceBinding>,
    ) -> Result<Self, Error> {
        if let Some(profile) = &mut config.audit_reporters {
            profile.canonicalize()?;
        }
        // Validate every configured route against the concrete transport before
        // mutating the database or starting network work. No binding takes a
        // group address (#1493).
        let is_group = |mac: &[u8]| transport.is_group_destination(mac);
        let device_bindings =
            DeviceBindingTable::from_configured(configured_device_bindings, is_group)?;
        let audit_routes = AuditRoutes::prepare(&mut db, &config, &device_bindings, &transport)?;
        super::audit_forwarder::initialize(&db, &config, &device_bindings, &transport);
        super::network_port::validate_apdu_capacity(&mut config, &transport)?;
        crate::local_device::validate_apdu_declaration(&db, config.max_apdu_length)?;
        let receive_timeout = super::segmented_receive::validate_segment_timeout(&config, &db)?;
        // Settled now: every owner from here on shares it (#1521).
        let config = Arc::new(config);
        let request_tasks = super::request_tasks::RequestTasks::for_server(&config)?;

        if config.vendor_id == 0 {
            warn!("vendor_id is 0 (ASHRAE reserved); set a valid vendor ID for production use");
        }

        let (clock, monotonic) = period::install_database_clocks(&mut db, clock_config);
        let membership = crate::membership::install_waker(&mut db);

        let (network, mut apdu_rx, audit_routes, network_controls) =
            boxed(|| super::network_port::start(&mut db, &config, transport, audit_routes)).await?;
        let local_mac = MacAddr::from_slice(network.local_mac());

        let network = Arc::new(network);
        // The limiter keeps this startup identity; changing Device membership
        // later does not rebind it.
        let device_instance = db.selected_device().map(|oid| oid.instance_number());
        let (discovery_limiter, time_sync_limiter) = request_limiters(&config, device_instance);
        let db = Arc::new(RwLock::new(db));
        let cov_counters = Arc::new(crate::cov::AtomicCovCounters::default());
        let cov_table = Arc::new(RwLock::new(
            CovSubscriptionTable::with_policy(config.cov_policy.clone(), Arc::clone(&cov_counters))
                .with_max_apdu_length(config.max_apdu_length as usize),
        ));
        let seg_ack_senders = Arc::new(segmented_send::SegmentedSendRegistry::default());
        let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));

        let cov_in_flight = Arc::new(Semaphore::new(255));
        let learned_routers = Arc::new(Mutex::new(LearnedRouterCache::new()));
        let notification_transactions = NotificationTransactions::new();
        let confirmed_request_tracker = Arc::new(ConfirmedRequestTracker::default());
        let device_bindings = Arc::new(RwLock::new(device_bindings));
        let comm_state = Arc::new(CommState::default()); // ENABLE at every start
        let dcc_timer: Arc<Mutex<crate::server::dcc_timer::TimerSlot>> =
            Arc::new(Mutex::new(Default::default()));
        let dcc_outcomes = Arc::new(dcc_outcomes::DccOutcomes::default());
        let event_suppressions = Arc::new(super::event_suppression::EventSuppressions::default());
        let mutation_decisions = Arc::new(crate::mutation::MutationDecisions::default());

        let target_audit = super::audit_recipient::TargetAudit::install(
            &mut *db.write().await,
            &config,
            audit_routes,
            &network,
            &notification_transactions,
        );
        let target_audit = match target_audit {
            Ok(runtime) => runtime,
            Err(error) => {
                let network = Arc::try_unwrap(network).map_err(|_| {
                    Error::Encoding("startup cleanup has an unexpected network owner".into())
                })?;
                boxed(|| super::network_port::StartingNetwork::from_network(network).cleanup())
                    .await?;
                return Err(error);
            }
        };
        let network_number_task = network_controls.map(|controls| {
            super::network_port::spawn_number_worker(
                &network,
                &db,
                config.registered_network_port,
                controls,
                target_audit.as_ref().map(Arc::downgrade),
            )
        });
        let network_dispatch = Arc::clone(&network);
        let config_dispatch = Arc::clone(&config);
        let notification_transactions_dispatch = Arc::clone(&notification_transactions);

        let audit_owner = target_audit
            .as_ref()
            .map(|runtime| Arc::clone(&runtime.owner));
        if let Some(owner) = &audit_owner {
            request_tasks.set_audit_owner(owner);
        }
        let requests = Arc::clone(&request_tasks);
        let dispatch_context = DispatchContext {
            services: RequestServices {
                db: Arc::clone(&db),
                network: Arc::clone(&network_dispatch),
                cov_table: Arc::clone(&cov_table),
                seg_ack_senders: Arc::clone(&seg_ack_senders),
                seg_send_permits: Arc::clone(&seg_send_permits),
                cov_in_flight: Arc::clone(&cov_in_flight),
                learned_routers: Arc::clone(&learned_routers),
                notification_transactions: Arc::clone(&notification_transactions_dispatch),
                device_bindings: Arc::clone(&device_bindings),
                comm_state: Arc::clone(&comm_state),
                dcc_timer: Arc::clone(&dcc_timer),
                dcc_outcomes: Arc::clone(&dcc_outcomes),
                event_suppressions: Arc::clone(&event_suppressions),
                confirmed_event_repeats: Arc::default(),
                received_event_log: Arc::default(),
                mutation_decisions: Arc::clone(&mutation_decisions),
                config: Arc::clone(&config_dispatch),
            },
            confirmed_request_tracker: Arc::clone(&confirmed_request_tracker),
            clock: clock.clone(),
            discovery_limiter: discovery_limiter.clone(),
            time_sync_limiter: time_sync_limiter.clone(),
            request_tasks: Arc::clone(&request_tasks),
        };
        // A Schedule tick that writes a Command's Present_Value starts its list.
        let schedule_runner = super::command_runs::CommandRunner::new(
            &dispatch_context.services,
            &request_tasks.spawner(),
        );
        let dispatch_task = spawn_owned(audit_owner.clone(), move || async move {
            let mut seg_receivers: HashMap<SegRecvKey, SegmentedRequestState> = HashMap::new();
            let mut notifications_open = true;
            let mut ingress_open = true;

            loop {
                // Drain all selected ownership before the first asynchronous Abort.
                Self::reap_expired_requests(
                    &network_dispatch,
                    &mut seg_receivers,
                    runtime_clock::now(),
                    receive_timeout,
                )
                .await;
                let received = tokio::select! {
                    () = super::segmented_receive::wait_receive_deadline(&seg_receivers, receive_timeout), if !seg_receivers.is_empty() => continue,
                    result = notification_transactions_dispatch.join_next(), if notifications_open => {
                        notifications_open = result.is_some();
                        NotificationTransactions::observe(result);
                        continue;
                    }
                    result = requests.join_next(), if !requests.is_empty() => {
                        super::request_tasks::RequestTasks::observe(result);
                        continue;
                    }
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
                Self::reap_expired_requests(
                    &network_dispatch,
                    &mut seg_receivers,
                    runtime_clock::now(),
                    receive_timeout,
                )
                .await;

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
                        let source_mac = received.source_mac.clone();
                        let source_network = received.source_network.clone();

                        // Clause 5.4.5.2 AbortPDU_Received: a peer's Abort
                        // ('server' = FALSE) ends any reassembly session for
                        // its transaction. A side effect, not a short
                        // circuit — the PDU still reaches `dispatch`, whose
                        // Abort arm cancels in-flight segmented response
                        // senders and records server-TSM results (#377).
                        // Provenance snapshot for this ingress (RB-07, by value).
                        let provenance = received.provenance;
                        let route = received.response_route();
                        super::segmented_receive::remove_peer_aborted_request(
                            &mut seg_receivers,
                            &received,
                            &decoded,
                        );

                        let mut received = Some(received);
                        let handled = if let Apdu::ConfirmedRequest(ref req) = decoded {
                            if req.segmented {
                                let seq = req.sequence_number.unwrap_or(0);
                                let key = segmented_receive_key(
                                    source_mac.as_slice(),
                                    source_network.as_ref(),
                                    req.invoke_id,
                                    provenance,
                                );
                                if let Some(conflict) =
                                    super::segmented_receive::find_receive_provenance_conflict(
                                        &seg_receivers,
                                        &key,
                                    )
                                {
                                    seg_receivers.remove(&conflict);
                                    warn!(
                                        invoke_id = req.invoke_id,
                                        "Aborting segmented request on provenance mismatch (fail-closed)"
                                    );
                                    Self::send_server_abort(
                                        &network_dispatch,
                                        &source_mac,
                                        source_network.as_ref(),
                                        &route,
                                        req.invoke_id,
                                        AbortReason::INVALID_APDU_IN_THIS_STATE,
                                    )
                                    .await;
                                    continue;
                                }

                                // Clause 5.4.5.1
                                // ConfirmedSegmentedReceivedNotSupported: a
                                // device that does not support segmented
                                // reception answers segment traffic with this
                                // Abort instead of reassembling — the
                                // configured Segmentation value is the
                                // advertisement peers plan transfers around
                                // (#381).
                                let receives_segments = config_dispatch.segmentation_supported
                                    == Segmentation::BOTH
                                    || config_dispatch.segmentation_supported
                                        == Segmentation::RECEIVE;
                                if !receives_segments {
                                    Self::send_server_abort(
                                        &network_dispatch,
                                        &source_mac,
                                        source_network.as_ref(),
                                        &route,
                                        req.invoke_id,
                                        AbortReason::SEGMENTATION_NOT_SUPPORTED,
                                    )
                                    .await;
                                    continue;
                                }

                                let mut ack_to_send: Option<SegmentAckPdu> = None;
                                let mut final_total: Option<usize> = None;

                                // The live session is consulted before the
                                // `seq == 0` open path: Clause 20.1.2.7 wraps
                                // the sequence number modulo 256, so segment
                                // 256 of a long request arrives as another
                                // `seq == 0` — treating it as a fresh initial
                                // segment would silently replace the session
                                // and reassemble only the tail (#364).
                                let saved_bytes =
                                    super::segmented_receive::saved_request_payload_bytes(
                                        &seg_receivers,
                                    );
                                if let Some(state) = seg_receivers.get_mut(&key) {
                                    // Compat-mode live read: key isolates
                                    // contexts; snapshot must match the key.
                                    debug_assert_eq!(state.provenance, provenance);
                                    // Clause 5.4.5.2 restarts SegmentTimer
                                    // for accepted, duplicate and
                                    // out-of-order segments alike, so the
                                    // refresh precedes the ordering checks.
                                    state.last_activity = runtime_clock::now();
                                    if seq != state.expected_seq {
                                        ack_to_send =
                                            super::segmented_receive::classify_non_next_segment(
                                                state,
                                                req.invoke_id,
                                                seq,
                                            );
                                    } else {
                                        // In-order NEW segment: duplicates
                                        // and gaps returned above, so a
                                        // retransmission can never trip the
                                        // cap (Clause 5.4.5.2
                                        // DuplicateSegmentReceived requires
                                        // duplicates be discarded, not
                                        // punished).
                                        if state.accepted_segments >= MAX_REQUEST_SEGMENTS {
                                            // Clause 5.4.5.2 has no overflow
                                            // transition; SendAbort ('server'
                                            // = TRUE, reason a local matter)
                                            // is its one generic escape, and
                                            // Clause 18.10's BUFFER_OVERFLOW
                                            // fits a reassembly that exceeds
                                            // available buffer capacity (#364).
                                            warn!(
                                                invoke_id = req.invoke_id,
                                                accepted = state.accepted_segments,
                                                "Segmented request exceeds reassembly capacity, aborting"
                                            );
                                            seg_receivers.remove(&key);
                                            Self::send_server_abort(
                                                &network_dispatch,
                                                &source_mac,
                                                source_network.as_ref(),
                                                &route,
                                                req.invoke_id,
                                                AbortReason::BUFFER_OVERFLOW,
                                            )
                                            .await;
                                            continue;
                                        }
                                        if let Err(e) = state.payload.save_new(
                                            seq,
                                            req.service_request.clone(),
                                            saved_bytes,
                                        ) {
                                            // An unsaveable segment ends the
                                            // session the same way — leaving
                                            // it dangling told the peer
                                            // nothing while this side could
                                            // never complete (#364).
                                            warn!(error = %e, "Rejecting unsaveable segment");
                                            seg_receivers.remove(&key);
                                            Self::send_server_abort(
                                                &network_dispatch,
                                                &source_mac,
                                                source_network.as_ref(),
                                                &route,
                                                req.invoke_id,
                                                AbortReason::BUFFER_OVERFLOW,
                                            )
                                            .await;
                                            continue;
                                        }
                                        state.last_progress = runtime_clock::now();
                                        state.accepted_segments += 1;
                                        state.expected_seq = seq.wrapping_add(1);
                                        state.last_acked_seq = seq;
                                        state.window_pos += 1;
                                        let should_ack = !req.more_follows
                                            || state.window_pos >= state.actual_window_size;
                                        if should_ack {
                                            state.window_pos = 0;
                                            state.initial_sequence_number = state.last_acked_seq;
                                            state.duplicate_count = 0;
                                            ack_to_send = Some(SegmentAckPdu {
                                                negative_ack: false,
                                                sent_by_server: true,
                                                invoke_id: req.invoke_id,
                                                sequence_number: seq,
                                                actual_window_size: state.actual_window_size,
                                            });
                                        }
                                        if !req.more_follows {
                                            // The count, not `seq + 1`: the
                                            // wire sequence number is modulo
                                            // 256 (Clause 20.1.2.7) and says
                                            // nothing about how many segments
                                            // were accepted (#364).
                                            final_total = Some(state.accepted_segments);
                                        }
                                    }
                                } else if seq == 0 {
                                    let proposed_window_size =
                                        req.proposed_window_size.unwrap_or(0);
                                    if !(1..=127).contains(&proposed_window_size) {
                                        warn!(
	                                            invoke_id = req.invoke_id,
	                                            proposed_window_size,
	                                            "Rejecting segmented request with invalid proposed window size"
	                                        );
                                        Self::send_server_abort(
                                            &network_dispatch,
                                            &source_mac,
                                            source_network.as_ref(),
                                            &route,
                                            req.invoke_id,
                                            AbortReason::WINDOW_SIZE_OUT_OF_RANGE,
                                        )
                                        .await;
                                        continue;
                                    }

                                    // New sessions only; global capacity precedes peer quota.
                                    if let Some(reason) =
                                        super::segmented_receive::segmented_request_admission_error(
                                            &seg_receivers,
                                            &key,
                                        )
                                    {
                                        Self::send_server_abort(
                                            &network_dispatch,
                                            &source_mac,
                                            source_network.as_ref(),
                                            &route,
                                            req.invoke_id,
                                            reason,
                                        )
                                        .await;
                                        continue;
                                    }

                                    let mut payload =
                                        super::segmented_receive::RequestPayload::new(req);
                                    if let Err(e) = payload.save_new(
                                        seq,
                                        req.service_request.clone(),
                                        saved_bytes,
                                    ) {
                                        // No session exists to drop on this
                                        // path; the Abort is what tells the
                                        // peer instead of leaving it to time
                                        // out (#364).
                                        warn!(error = %e, "Rejecting unsaveable segment");
                                        Self::send_server_abort(
                                            &network_dispatch,
                                            &source_mac,
                                            source_network.as_ref(),
                                            &route,
                                            req.invoke_id,
                                            AbortReason::BUFFER_OVERFLOW,
                                        )
                                        .await;
                                        continue;
                                    }
                                    let (state, initial_ack) =
                                        super::segmented_receive::initial_state(
                                            payload,
                                            provenance,
                                            received
                                                .as_ref()
                                                .and_then(|r| r.direct_response.clone()),
                                            source_mac.clone(),
                                            source_network.clone(),
                                            req,
                                        );
                                    ack_to_send = initial_ack;
                                    if !req.more_follows {
                                        final_total = Some(1);
                                    }
                                    seg_receivers.insert(key.clone(), state);
                                } else {
                                    warn!(
	                                        invoke_id = req.invoke_id,
	                                        seq = seq,
	                                        "Received non-initial segment without prior segment 0, aborting"
	                                    );
                                    Self::send_server_abort(
                                        &network_dispatch,
                                        &source_mac,
                                        source_network.as_ref(),
                                        &route,
                                        req.invoke_id,
                                        AbortReason::INVALID_APDU_IN_THIS_STATE,
                                    )
                                    .await;
                                    continue;
                                }

                                if let Some(seg_ack) = ack_to_send {
                                    let seg_ack = Apdu::SegmentAck(seg_ack);
                                    let mut ack_buf = BytesMut::new();
                                    encode_apdu(&mut ack_buf, &seg_ack)
                                        .expect("valid APDU encoding");
                                    if let Err(e) = Self::send_confirmed_response_apdu(
                                        &network_dispatch,
                                        &ack_buf,
                                        &source_mac,
                                        source_network.as_ref(),
                                        &route,
                                    )
                                    .await
                                    {
                                        warn!(
                                            error = %e,
                                            "Failed to send SegmentAck for segmented request"
                                        );
                                    }
                                }

                                if let Some(total) = final_total {
                                    if let Some(state) = seg_receivers.remove(&key) {
                                        if let Some(envelope) = received.as_mut() {
                                            envelope.provenance = state.provenance;
                                            envelope.direct_response = state.direct_response;
                                        }
                                        match state.payload.complete(total) {
                                            Ok(reassembled) => {
                                                debug!(
                                                    invoke_id = reassembled.invoke_id,
                                                    segments = total,
                                                    payload_len = reassembled.service_request.len(),
                                                    "Reassembled segmented ConfirmedRequest"
                                                );
                                                Self::dispatch(
                                                    &dispatch_context,
                                                    &source_mac,
                                                    Apdu::ConfirmedRequest(reassembled),
                                                    received.take().unwrap_or_else(|| {
                                                        warn!("received consumed twice - using empty fallback");
                                                        bacnet_network::layer::ReceivedApdu {
                                                            direct_response: None,
                                                            apdu: bytes::Bytes::new(),
                                                            source_mac: bacnet_types::MacAddr::new(),
                                                            ingress_network: None,
                                                            source_network: None,
                                                            link_layer_group: false,
                                                            is_group: false,
                                                            global_broadcast: false,
                                                            data_attributes: Vec::new(),
                                                            provenance: bacnet_transport::port::TransportProvenance::unverified(),
                                                            reply_tx: None,
                                                        }
                                                    }),
                                                )
                                                .await;
                                            }
                                            Err(e) => {
                                                warn!(
                                                    error = %e,
                                                    "Failed to reassemble segmented request"
                                                );
                                            }
                                        }
                                    }
                                }

                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        if !handled {
                            Self::dispatch(
                                &dispatch_context,
                                &source_mac,
                                decoded,
                                received.take().unwrap_or_else(|| {
                                    warn!("received consumed twice — using empty fallback");
                                    bacnet_network::layer::ReceivedApdu {
                                        direct_response: None,
                                        apdu: bytes::Bytes::new(),
                                        source_mac: bacnet_types::MacAddr::new(),
                                        ingress_network: None,
                                        source_network: None,
                                        link_layer_group: false,
                                        is_group: false,
                                        global_broadcast: false,
                                        data_attributes: Vec::new(),
                                        provenance:
                                            bacnet_transport::port::TransportProvenance::unverified(
                                            ),
                                        reply_tx: None,
                                    }
                                }),
                            )
                            .await;
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Server failed to decode received APDU");
                    }
                }
            }
        });

        let cov_table_for_purge = Arc::clone(&cov_table);
        let cov_purge_task = spawn_owned(audit_owner.clone(), move || async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                let mut table = cov_table_for_purge.write().await;
                let purged = table.purge_expired();
                if purged > 0 {
                    debug!(purged, "Purged expired COV subscriptions");
                }
            }
        });

        // Background commits fan COV out like a network write (#889).
        let cov_fanout = super::cov_fanout::CovFanout::new(
            &super::cov_notify_context::CovNotifyContext {
                db: &db,
                network: &network,
                cov_table: &cov_table,
                cov_in_flight: &cov_in_flight,
                notification_transactions: &notification_transactions,
                comm_state: &comm_state,
                config: &config,
            },
            &event_suppressions,
        );

        let fault_detection_task = if config.enable_fault_detection {
            let fanout = cov_fanout.clone();
            Some(spawn_owned(audit_owner.clone(), move || async move {
                let detector = crate::fault_detection::FaultDetector::default();
                let mut interval = tokio::time::interval(Duration::from_secs(10));
                loop {
                    interval.tick().await;
                    let committed = {
                        let mut db_guard = fanout.db.write().await;
                        // The detector mutates objects in place, so snapshot
                        // Life Safety state up front.
                        let mut commit = BackgroundCommit::snapshot_all(&db_guard);
                        for change in detector.evaluate(&mut db_guard) {
                            debug!(
                                object = %change.object_id,
                                old = %change.old_reliability,
                                new = %change.new_reliability,
                                "Fault detection: reliability changed"
                            );
                            commit.changed(change.object_id);
                        }
                        commit
                            .finish(&fanout.db, &mut db_guard, &fanout.cov_table)
                            .await
                    };
                    fanout.fire(&committed).await;
                }
            }))
        } else {
            None
        };

        let event_enrollment_task = if config.enable_event_enrollment {
            let ee_period = event_enrollment_period(config.event_enrollment_interval_secs);
            // The delay countdown converts seconds to passes with
            // `ceil(delay / period)`, so the evaluator needs the actual,
            // clamped interval — not the raw config value.
            Some(
                super::event_enrollment_lifecycle::spawn_event_enrollment_task(
                    super::event_enrollment_lifecycle::EventEnrollmentTask {
                        db: Arc::clone(&db),
                        network: Arc::clone(&network),
                        comm_state: Arc::clone(&comm_state),
                        learned_routers: Arc::clone(&learned_routers),
                        notification_transactions: Arc::clone(&notification_transactions),
                        device_bindings: Arc::clone(&device_bindings),
                        suppressions: Arc::clone(&event_suppressions),
                        period: ee_period,
                        retry_ms: config.cov_retry_timeout_ms,
                        local_apdu_capacity: config.max_apdu_length,
                    },
                ),
            )
        } else {
            None
        };

        let trend_log_task = Some(spawn_owned(audit_owner.clone(), || {
            crate::trend_log::run(Arc::clone(&db))
        }));

        let schedule_fanout = cov_fanout.clone();
        // The 60-second Schedule pass, and the work an application's own
        // `add` or `remove` queued on the database (`crate::membership`).
        let schedule_tick_task = Some(spawn_owned(audit_owner.clone(), move || async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                let (db, cov_table) = (&schedule_fanout.db, &schedule_fanout.cov_table);
                let committed = tokio::select! {
                    _ = interval.tick() => {
                        crate::schedule::tick_schedules_committed(db, cov_table).await
                    }
                    () = membership.notified() => {
                        crate::membership::settle_committed(db, cov_table).await
                    }
                };
                schedule_fanout.fire(&committed).await;
                schedule_runner.start(committed.command_runs);
            }
        }));

        // One-second intrinsic-reporting task: advances the `Time_Delay`
        // countdown for any object with a pending delayed transition and sends
        // the EventNotification when the delay elapses. The per-write path
        // only *seeds* a pending transition (see `fire_event_notifications`);
        // this task is the sole confirmer, so repeated writes cannot shorten
        // the delay (ASHRAE 135-2020 Clause 13.3). Runs unconditionally like the
        // trend-log task — a no-pending tick is a cheap empty iteration.
        //
        // It is also what carries Reliability into event-state-detection. Per
        // Clause 13.2.2 the FAULT determination is a standing condition, so each
        // tick re-derives it from the object's current `Reliability` rather than
        // reacting to a change event. Whoever writes Reliability — an object's
        // opt-in evaluation hook, a local write, or a network write — reaches
        // detection through this tick. The fault detector above only fans COV
        // out for the Status_Flags change it causes (#889); it signals nothing
        // to event detection. `enable_fault_detection` therefore governs only
        // whether those object-owned hooks run every 10 seconds, never whether
        // an existing Reliability is honored.
        //
        // Six of the nine wired object types have no route that can set
        // Reliability, so the fault path is correct but inert on them (#218).
        let intrinsic_reporting_task = Some(spawn_owned(audit_owner.clone(), || {
            intrinsic::run(
                cov_fanout.clone(),
                Arc::clone(&learned_routers),
                Arc::clone(&device_bindings),
            )
        }));

        // Reports what changed while a confirmed report was outstanding, once
        // it is acknowledged (#896).
        let cov_revisit_task = Some(spawn_owned(audit_owner.clone(), || {
            cov_fanout.clone().run_revisits()
        }));

        let binary_lighting_operation_task = Some(
            super::binary_lighting_lifecycle::spawn_binary_lighting_operation_task(
                cov_fanout, monotonic,
            ),
        );

        let broadcaster = super::broadcaster::BroadcasterState::new(
            &network,
            &request_tasks,
            &config,
            &db,
            &comm_state,
        );
        let server = Self {
            target_audit,
            config,
            discovery_limiter,
            time_sync_limiter,
            _clock: clock,
            network: Some(network),
            broadcaster,
            transport_cleanup: None,
            transport_cleanup_error: None,
            network_number_task,
            db,
            cov_table,
            cov_counters,
            seg_ack_senders,
            seg_send_permits,
            cov_in_flight,
            learned_routers,
            notification_transactions,
            confirmed_request_tracker,
            device_bindings,
            comm_state,
            dcc_timer,
            dcc_outcomes,
            event_suppressions,
            mutation_decisions,
            dispatch_task: Some(dispatch_task),
            request_tasks,
            cov_purge_task: Some(cov_purge_task),
            fault_detection_task,
            event_enrollment_task,
            trend_log_task,
            schedule_tick_task,
            intrinsic_reporting_task,
            binary_lighting_operation_task,
            cov_revisit_task,
            local_mac,
        };
        boxed(|| server.execute_initial_staging_plans()).await;
        Ok(server)
    }
}
