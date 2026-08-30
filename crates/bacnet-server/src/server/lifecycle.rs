use super::event_notifications::ResolvedIntrinsicTransition;
use super::*;

/// Resolve the configured Event Enrollment interval into a tick period.
///
/// `tokio::time::interval` panics on a zero period, and that panic would land
/// inside a spawned task — `start` would still return `Ok` while enrollment
/// evaluation was silently dead. A configured `0` is clamped to one second
/// instead, matching how an invalid `vendor_id` is handled: warn loudly and
/// keep the device running. Use `enable_event_enrollment(false)` to actually
/// disable evaluation.
pub(super) fn event_enrollment_period(secs: u64) -> Duration {
    if secs == 0 {
        warn!(
            "event_enrollment_interval_secs is 0; clamping to 1s. \
             Use enable_event_enrollment(false) to disable Event Enrollment evaluation"
        );
        return Duration::from_secs(1);
    }
    Duration::from_secs(secs)
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    pub(super) async fn start_with_clock_mode_and_bindings(
        mut config: ServerConfig,
        mut db: ObjectDatabase,
        transport: T,
        clock_config: Option<ClockConfig>,
        configured_device_bindings: Vec<DeviceBinding>,
    ) -> Result<Self, Error> {
        // Validate every configured route against the concrete transport before
        // mutating the database or starting network work.
        let device_bindings =
            DeviceBindingTable::from_configured(configured_device_bindings, |mac| {
                transport.is_broadcast_mac(mac)
            })?;
        let transport_max = transport.max_apdu_length() as u32;
        config.max_apdu_length = config.max_apdu_length.min(transport_max);
        let max_apdu = u16::try_from(config.max_apdu_length).map_err(|_| {
            Error::Encoding(format!(
                "invalid max_apdu_length {}; expected one of 50, 128, 206, 480, 1024, 1476",
                config.max_apdu_length
            ))
        })?;
        validate_max_apdu_length(max_apdu)?;

        if config.vendor_id == 0 {
            warn!("vendor_id is 0 (ASHRAE reserved); set a valid vendor ID for production use");
        }

        let clock = clock_config.map(|config| Arc::new(ServerClock::new(config)));
        let reader = clock
            .as_ref()
            .map(|clock| Arc::clone(clock) as Arc<dyn bacnet_objects::clock::ClockReader>);
        db.set_clock_reader(reader);

        let mut network = NetworkLayer::new(transport);
        let apdu_rx = network.start().await?;
        let local_mac = MacAddr::from_slice(network.local_mac());

        let network = Arc::new(network);
        let db = Arc::new(RwLock::new(db));
        let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::new()));
        let seg_ack_senders: Arc<Mutex<HashMap<SegKey, Arc<SegmentedSendHandle>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));

        let cov_in_flight = Arc::new(Semaphore::new(255));
        let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));
        let notification_transactions = NotificationTransactions::new();
        let device_bindings = Arc::new(RwLock::new(device_bindings));
        let comm_state = Arc::new(AtomicU8::new(0)); // 0 = Enable (default)
        let dcc_timer: Arc<Mutex<Option<JoinHandle<()>>>> = Arc::new(Mutex::new(None));

        let network_dispatch = Arc::clone(&network);
        let db_dispatch = Arc::clone(&db);
        let cov_dispatch = Arc::clone(&cov_table);
        let seg_ack_dispatch = Arc::clone(&seg_ack_senders);
        let seg_send_permits_dispatch = Arc::clone(&seg_send_permits);
        let cov_in_flight_dispatch = Arc::clone(&cov_in_flight);
        let server_tsm_dispatch = Arc::clone(&server_tsm);
        let notification_transactions_dispatch = Arc::clone(&notification_transactions);
        let device_bindings_dispatch = Arc::clone(&device_bindings);
        let comm_state_dispatch = Arc::clone(&comm_state);
        let dcc_timer_dispatch = Arc::clone(&dcc_timer);
        let config_dispatch = Arc::new(config.clone());
        let clock_dispatch = clock.clone();

        let dispatch_task = tokio::spawn(async move {
            let mut apdu_rx = apdu_rx;
            let mut seg_receivers: HashMap<SegKey, SegmentedRequestState> = HashMap::new();

            while let Some(received) = apdu_rx.recv().await {
                let now = Instant::now();
                seg_receivers.retain(|_key, state| {
                    now.duration_since(state.last_activity) < SEG_RECEIVER_TIMEOUT
                });

                match apdu::decode_apdu(received.apdu.clone()) {
                    Ok(decoded) => {
                        let source_mac = received.source_mac.clone();
                        let source_network = received.source_network.clone();

                        // Clause 5.4.5.2 AbortPDU_Received: a peer's Abort
                        // ('server' = FALSE) ends any reassembly session for
                        // its transaction. A side effect, not a short
                        // circuit — the PDU still reaches `dispatch`, whose
                        // Abort arm cancels in-flight segmented response
                        // senders and records server-TSM results (#377).
                        if let Apdu::Abort(ref abt) = decoded {
                            if !abt.sent_by_server {
                                let key = segmented_transaction_key(
                                    source_mac.as_slice(),
                                    source_network.as_ref(),
                                    abt.invoke_id,
                                );
                                seg_receivers.remove(&key);
                            }
                        }

                        let mut received = Some(received);
                        let handled = if let Apdu::ConfirmedRequest(ref req) = decoded {
                            if req.segmented {
                                let seq = req.sequence_number.unwrap_or(0);
                                let key = segmented_transaction_key(
                                    source_mac.as_slice(),
                                    source_network.as_ref(),
                                    req.invoke_id,
                                );

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
                                if let Some(state) = seg_receivers.get_mut(&key) {
                                    // Clause 5.4.5.2 restarts SegmentTimer
                                    // for accepted, duplicate and
                                    // out-of-order segments alike, so the
                                    // refresh precedes the ordering checks.
                                    state.last_activity = Instant::now();
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
                                            // — "a buffer capacity has been
                                            // exceeded" — is the fit (#364).
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
                                                req.invoke_id,
                                                AbortReason::BUFFER_OVERFLOW,
                                            )
                                            .await;
                                            continue;
                                        }
                                        if let Err(e) =
                                            state.receiver.receive(seq, req.service_request.clone())
                                        {
                                            // An unsaveable segment ends the
                                            // session the same way — leaving
                                            // it dangling told the peer
                                            // nothing while this side could
                                            // never complete (#364).
                                            warn!(error = %e, "Rejecting oversized segment");
                                            seg_receivers.remove(&key);
                                            Self::send_server_abort(
                                                &network_dispatch,
                                                &source_mac,
                                                source_network.as_ref(),
                                                req.invoke_id,
                                                AbortReason::BUFFER_OVERFLOW,
                                            )
                                            .await;
                                            continue;
                                        }
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
                                            req.invoke_id,
                                            AbortReason::WINDOW_SIZE_OUT_OF_RANGE,
                                        )
                                        .await;
                                        continue;
                                    }

                                    // This path runs only when no session
                                    // exists for the key, so the map length
                                    // is the whole capacity check.
                                    if seg_receivers.len() >= MAX_SEG_RECEIVERS {
                                        Self::send_server_abort(
                                            &network_dispatch,
                                            &source_mac,
                                            source_network.as_ref(),
                                            req.invoke_id,
                                            AbortReason::BUFFER_OVERFLOW,
                                        )
                                        .await;
                                        continue;
                                    }

                                    let mut receiver = SegmentReceiver::new();
                                    if let Err(e) =
                                        receiver.receive(seq, req.service_request.clone())
                                    {
                                        // No session exists to drop on this
                                        // path; the Abort is what tells the
                                        // peer instead of leaving it to time
                                        // out (#364).
                                        warn!(error = %e, "Rejecting oversized segment");
                                        Self::send_server_abort(
                                            &network_dispatch,
                                            &source_mac,
                                            source_network.as_ref(),
                                            req.invoke_id,
                                            AbortReason::BUFFER_OVERFLOW,
                                        )
                                        .await;
                                        continue;
                                    }
                                    let actual_window_size = proposed_window_size;
                                    let mut state = SegmentedRequestState {
                                        receiver,
                                        first_req: req.clone(),
                                        last_activity: Instant::now(),
                                        expected_seq: 1,
                                        initial_sequence_number: 0,
                                        duplicate_count: 0,
                                        last_acked_seq: 0,
                                        window_pos: 1,
                                        actual_window_size,
                                        accepted_segments: 1,
                                    };
                                    let should_ack =
                                        !req.more_follows || state.window_pos >= actual_window_size;
                                    if should_ack {
                                        state.window_pos = 0;
                                        state.initial_sequence_number = state.last_acked_seq;
                                        state.duplicate_count = 0;
                                        ack_to_send = Some(SegmentAckPdu {
                                            negative_ack: false,
                                            sent_by_server: true,
                                            invoke_id: req.invoke_id,
                                            sequence_number: seq,
                                            actual_window_size,
                                        });
                                    }
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
                                        match state.receiver.reassemble(total) {
                                            Ok(full_data) => {
                                                let reassembled =
                                                    bacnet_encoding::apdu::ConfirmedRequest {
                                                        segmented: false,
                                                        more_follows: false,
                                                        sequence_number: None,
                                                        proposed_window_size: None,
                                                        service_request: Bytes::from(full_data),
                                                        invoke_id: state.first_req.invoke_id,
                                                        service_choice: state
                                                            .first_req
                                                            .service_choice,
                                                        max_apdu_length: state
                                                            .first_req
                                                            .max_apdu_length,
                                                        segmented_response_accepted: state
                                                            .first_req
                                                            .segmented_response_accepted,
                                                        max_segments: state.first_req.max_segments,
                                                    };
                                                debug!(
                                                    invoke_id = reassembled.invoke_id,
                                                    segments = total,
                                                    payload_len = reassembled.service_request.len(),
                                                    "Reassembled segmented ConfirmedRequest"
                                                );
                                                Self::dispatch(
	                                                    &db_dispatch,
	                                                    &network_dispatch,
                                    &cov_dispatch,
                                    &seg_ack_dispatch,
                                    &seg_send_permits_dispatch,
                                    &cov_in_flight_dispatch,
                                    &server_tsm_dispatch,
                                                    &notification_transactions_dispatch,
	                                                    &device_bindings_dispatch,
	                                                    &comm_state_dispatch,
	                                                    &dcc_timer_dispatch,
	                                                    &config_dispatch,
	                                                    &clock_dispatch,
	                                                    &source_mac,
	                                                    Apdu::ConfirmedRequest(reassembled),
	                                                    received.take().unwrap_or_else(|| {
	                                                        warn!("received consumed twice - using empty fallback");
	                                                        bacnet_network::layer::ReceivedApdu {
	                                                            apdu: bytes::Bytes::new(),
	                                                            source_mac: bacnet_types::MacAddr::new(),
	                                                            source_network: None,
	                                                            link_layer_group: false,
	                                                            is_group: false,
	                                                            data_attributes: Vec::new(),
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
                                &db_dispatch,
                                &network_dispatch,
                                &cov_dispatch,
                                &seg_ack_dispatch,
                                &seg_send_permits_dispatch,
                                &cov_in_flight_dispatch,
                                &server_tsm_dispatch,
                                &notification_transactions_dispatch,
                                &device_bindings_dispatch,
                                &comm_state_dispatch,
                                &dcc_timer_dispatch,
                                &config_dispatch,
                                &clock_dispatch,
                                &source_mac,
                                decoded,
                                received.take().unwrap_or_else(|| {
                                    warn!("received consumed twice — using empty fallback");
                                    bacnet_network::layer::ReceivedApdu {
                                        apdu: bytes::Bytes::new(),
                                        source_mac: bacnet_types::MacAddr::new(),
                                        source_network: None,
                                        link_layer_group: false,
                                        is_group: false,
                                        data_attributes: Vec::new(),
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
        let cov_purge_task = tokio::spawn(async move {
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

        let fault_detection_task = if config.enable_fault_detection {
            let db_fault = Arc::clone(&db);
            Some(tokio::spawn(async move {
                let detector = crate::fault_detection::FaultDetector::default();
                let mut interval = tokio::time::interval(Duration::from_secs(10));
                loop {
                    interval.tick().await;
                    let mut db_guard = db_fault.write().await;
                    let changes = detector.evaluate(&mut db_guard);
                    for change in &changes {
                        debug!(
                            object = %change.object_id,
                            old = change.old_reliability,
                            new = change.new_reliability,
                            "Fault detection: reliability changed"
                        );
                    }
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
                    Arc::clone(&db),
                    Arc::clone(&network),
                    Arc::clone(&comm_state),
                    Arc::clone(&server_tsm),
                    Arc::clone(&notification_transactions),
                    Arc::clone(&device_bindings),
                    ee_period,
                    config.cov_retry_timeout_ms,
                ),
            )
        } else {
            None
        };

        let db_trend = Arc::clone(&db);
        let trend_log_state: crate::trend_log::TrendLogState =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let trend_log_task = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                crate::trend_log::poll_trend_logs(&db_trend, &trend_log_state).await;
            }
        }));

        let db_schedule = Arc::clone(&db);
        let schedule_tick_task = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                crate::schedule::tick_schedules(&db_schedule, 0).await;
            }
        }));

        // One-second intrinsic-reporting task: advances the `Time_Delay`
        // countdown for any object with a pending delayed transition and sends
        // the EventNotification when the delay elapses. The per-write path
        // only *seeds* a pending transition (see `fire_event_notifications`);
        // this task is the sole confirmer, so repeated writes cannot shorten
        // the delay (ASHRAE 135-2020 §13.2.4). Runs unconditionally like the
        // trend-log task — a no-pending tick is a cheap empty iteration.
        //
        // It is also what carries Reliability into event-state-detection. Per
        // Clause 13.2.2 the FAULT determination is a standing condition, so each
        // tick re-derives it from the object's current `Reliability` rather than
        // reacting to a change event. That is why the fault detector above can
        // keep merely *logging* its `ReliabilityChange` records: whoever writes
        // Reliability — an object's opt-in evaluation hook, a local write, or a
        // network write — reaches detection through this tick, and no route
        // needs to notify anything. `enable_fault_detection` therefore governs
        // only whether those object-owned hooks run every 10 seconds, never
        // whether an existing Reliability is honored.
        //
        // Six of the nine wired object types have no route that can set
        // Reliability, so the fault path is correct but inert on them (#218).
        let db_intrinsic = Arc::clone(&db);
        let network_intrinsic = Arc::clone(&network);
        let comm_state_intrinsic = Arc::clone(&comm_state);
        let server_tsm_intrinsic = Arc::clone(&server_tsm);
        let notification_transactions_intrinsic = Arc::clone(&notification_transactions);
        let device_bindings_intrinsic = Arc::clone(&device_bindings);
        let intrinsic_retry_ms = config.cov_retry_timeout_ms;
        let intrinsic_reporting_task = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            // The countdown decrements exactly once per call, so a delayed wake
            // must NOT burst-deliver missed ticks (each would decrement
            // `remaining`, compressing the Time_Delay). `Delay` collapses a
            // missed deadline into a single tick, preserving per-second
            // granularity (ASHRAE 135-2020 §13.2.4).
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                // DCC gates the outbound sender, not event-state detection or
                // the local transition actions in Clause 13.2.2.1.4.
                // Collect resolved transitions under a brief write lock, then
                // drop it before sending (never hold the db lock across a
                // network send — matches the per-write notification path).
                //
                // Event_Enable gates distribution only (Clause 12.12), so every
                // built-in proposal is committed locally before a suppressed
                // transition is omitted from the outbound work list. Legacy
                // implementations already commit during their tick and bypass
                // the atomic hook explicitly.
                let fired = {
                    let mut db = db_intrinsic.write().await;
                    let mut out = Vec::new();
                    for oid in db.list_objects() {
                        let evaluated = db.get_mut(&oid).and_then(|object| {
                            let requires_atomic_commit =
                                object.intrinsic_reporting_requires_atomic_commit();
                            object
                                .tick_intrinsic_reporting()
                                .map(|outcome| (requires_atomic_commit, outcome))
                        });
                        let resolved = evaluated.and_then(|(requires_atomic_commit, outcome)| {
                            if requires_atomic_commit {
                                Self::commit_intrinsic_transition(&mut db, &oid, outcome)
                                    .map(ResolvedIntrinsicTransition::Committed)
                            } else {
                                Some(ResolvedIntrinsicTransition::Legacy(outcome))
                            }
                        });
                        if let Some(resolved) = resolved {
                            if resolved.distribute() && resolved.can_emit() {
                                out.push((oid, resolved));
                            }
                        }
                    }
                    out
                };
                for (oid, resolved) in fired {
                    Self::build_and_send_event_notification_with_bindings(
                        &db_intrinsic,
                        &network_intrinsic,
                        &comm_state_intrinsic,
                        &server_tsm_intrinsic,
                        &notification_transactions_intrinsic,
                        &device_bindings_intrinsic,
                        &oid,
                        resolved,
                        intrinsic_retry_ms,
                    )
                    .await;
                }
            }
        }));

        Ok(Self {
            config,
            _clock: clock,
            network,
            db,
            cov_table,
            seg_ack_senders,
            seg_send_permits,
            cov_in_flight,
            server_tsm,
            notification_transactions,
            device_bindings,
            comm_state,
            dcc_timer,
            dispatch_task: Some(dispatch_task),
            cov_purge_task: Some(cov_purge_task),
            fault_detection_task,
            event_enrollment_task,
            trend_log_task,
            schedule_tick_task,
            intrinsic_reporting_task,
            local_mac,
        })
    }

    /// Send a `'server' = TRUE` Abort back along the request's path.
    ///
    /// Every Abort this dispatch loop originates answers a client's request,
    /// so the flag is always TRUE — it names the sender's role, not the
    /// error (Clause 20.1.9.1: "TRUE when the Abort PDU is sent by a
    /// server").
    async fn send_server_abort(
        network: &Arc<NetworkLayer<T>>,
        source_mac: &MacAddr,
        source_network: Option<&NpduAddress>,
        invoke_id: u8,
        abort_reason: AbortReason,
    ) {
        let abort_pdu = Apdu::Abort(AbortPdu {
            sent_by_server: true,
            invoke_id,
            abort_reason,
        });
        let mut abort_buf = BytesMut::new();
        encode_apdu(&mut abort_buf, &abort_pdu).expect("valid APDU encoding");
        if let Err(e) =
            Self::send_confirmed_response_apdu(network, &abort_buf, source_mac, source_network)
                .await
        {
            warn!(error = %e, reason = abort_reason.to_raw(), "Failed to send Abort");
        }
    }

    /// Get the server's local MAC address.
    pub fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    /// Get a reference to the shared object database.
    pub fn database(&self) -> &Arc<RwLock<ObjectDatabase>> {
        &self.db
    }

    /// Create a cloneable handle for unsolicited I-Am announcements.
    pub fn i_am_broadcaster(&self) -> IAmBroadcaster<T> {
        IAmBroadcaster {
            config: self.config.clone(),
            network: Arc::clone(&self.network),
            db: Arc::clone(&self.db),
        }
    }

    /// Get the communication state per DeviceCommunicationControl.
    ///
    /// Returns 0 (Enable), 1 (Disable), or 2 (DisableInitiation).
    pub fn comm_state(&self) -> u8 {
        self.comm_state.load(Ordering::Acquire)
    }

    /// Arm or rearm a Life Safety object from trusted local application logic.
    ///
    /// This uses the object-internal state channel under the database write
    /// lock. Network WriteProperty and WritePropertyMultiple remain unable to
    /// forge `Operation_Expected`. Property-specific COV for this local state
    /// change remains follow-up #177; this method deliberately avoids the
    /// coarse whole-object COV/event path used by [`write_local`](Self::write_local).
    pub async fn set_life_safety_operation_expected_local(
        &self,
        oid: &ObjectIdentifier,
        operation: LifeSafetyOperation,
    ) -> Result<(), Error> {
        let mut db = self.db.write().await;
        let object = db.get_mut(oid).ok_or_else(|| Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
        })?;
        object.set_life_safety_operation_expected_internal(operation)
    }

    /// Write a property on a local object and fire the same post-write COV
    /// and event notifications that a network [`WriteProperty`] does.
    ///
    /// This is the server-owned local-mutation entry point: it performs the
    /// write under the database lock — routing `OBJECT_NAME` through the name
    /// uniqueness check and index refresh, exactly like the network handler —
    /// then releases the lock and runs the COV/event trigger path so a
    /// subscription observes a local mutation just as it would a network one.
    ///
    /// Low-level object setters (`set_present_value` and friends) deliberately
    /// bypass this path; they are building blocks below the high-level server
    /// surface and are not expected to emit notifications.
    ///
    /// [`WriteProperty`]: bacnet_services::write_property::WritePropertyRequest
    pub async fn write_local(
        &self,
        oid: &ObjectIdentifier,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        // Scoped write: mutate under the lock, then drop the guard before
        // firing notifications — matching the network dispatch path, which
        // releases the database lock before the post-write trigger loop.
        {
            let mut db = self.db.write().await;
            if db.get(oid).is_none() {
                return Err(Error::Protocol {
                    class: ErrorClass::OBJECT.to_raw() as u32,
                    code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
                });
            }
            if property == PropertyIdentifier::OBJECT_NAME {
                if let PropertyValue::CharacterString(ref new_name) = value {
                    db.check_name_available(oid, new_name)?;
                }
            }
            let object = db.get_mut(oid).expect("existence checked above");
            object.write_property(property, array_index, value, priority)?;
            if property == PropertyIdentifier::OBJECT_NAME {
                db.update_name_index(oid);
            }
        }

        // Post-write COV/event trigger, mirroring the network handler's loop.
        Self::fire_event_notifications_with_bindings(
            &self.db,
            &self.network,
            &self.comm_state,
            &self.server_tsm,
            &self.notification_transactions,
            &self.device_bindings,
            oid,
            self.config.cov_retry_timeout_ms,
        )
        .await;
        Self::fire_cov_notifications(
            &self.db,
            &self.network,
            &self.cov_table,
            &self.cov_in_flight,
            &self.notification_transactions,
            &self.comm_state,
            &self.config,
            oid,
        )
        .await;
        Ok(())
    }

    /// Generate a PICS document from the current object database and server configuration.
    ///
    /// The caller must supply a [`PicsConfig`] for fields not available from the server
    /// (vendor name, model, firmware revision, etc.).
    pub async fn generate_pics(&self, pics_config: &crate::pics::PicsConfig) -> crate::pics::Pics {
        let db = self.db.read().await;
        crate::pics::PicsGenerator::new(&db, &self.config, pics_config).generate()
    }

    /// Broadcast an I-Am for this server's Device object using the bound transport socket.
    pub async fn broadcast_i_am(&self) -> Result<(), Error> {
        broadcast_i_am_from(&self.config, &self.db, &self.network).await
    }
}

impl<T: TransportPort + 'static> IAmBroadcaster<T> {
    /// Broadcast an I-Am for this server's Device object using the bound transport socket.
    pub async fn broadcast_i_am(&self) -> Result<(), Error> {
        broadcast_i_am_from(&self.config, &self.db, &self.network).await
    }
}

async fn broadcast_i_am_from<T: TransportPort + 'static>(
    config: &ServerConfig,
    db: &Arc<RwLock<ObjectDatabase>>,
    network: &Arc<NetworkLayer<T>>,
) -> Result<(), Error> {
    let device_oid = {
        let db = db.read().await;
        db.list_objects()
            .into_iter()
            .find(|oid| oid.object_type() == ObjectType::DEVICE)
            .ok_or_else(|| Error::Encoding("no Device object in database".into()))?
    };

    let i_am = IAmRequest {
        object_identifier: device_oid,
        max_apdu_length: config.max_apdu_length,
        segmentation_supported: config.segmentation_supported,
        vendor_id: config.vendor_id,
    };

    let mut service_buf = BytesMut::new();
    i_am.encode(&mut service_buf);

    let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
        service_choice: UnconfirmedServiceChoice::I_AM,
        service_request: service_buf.freeze(),
    });

    let mut buf = BytesMut::new();
    encode_apdu(&mut buf, &pdu)?;

    network
        .broadcast_apdu(&buf, false, NetworkPriority::NORMAL)
        .await
}
