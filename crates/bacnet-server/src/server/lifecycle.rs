use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    pub async fn start(
        mut config: ServerConfig,
        db: ObjectDatabase,
        transport: T,
    ) -> Result<Self, Error> {
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

        let mut network = NetworkLayer::new(transport);
        let apdu_rx = network.start().await?;
        let local_mac = MacAddr::from_slice(network.local_mac());

        let network = Arc::new(network);
        let db = Arc::new(RwLock::new(db));
        let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::new()));
        let seg_ack_senders: Arc<Mutex<HashMap<SegKey, mpsc::Sender<SegmentAckPdu>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let cov_in_flight = Arc::new(Semaphore::new(255));
        let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));
        let comm_state = Arc::new(AtomicU8::new(0)); // 0 = Enable (default)
        let dcc_timer: Arc<Mutex<Option<JoinHandle<()>>>> = Arc::new(Mutex::new(None));

        let network_dispatch = Arc::clone(&network);
        let db_dispatch = Arc::clone(&db);
        let cov_dispatch = Arc::clone(&cov_table);
        let seg_ack_dispatch = Arc::clone(&seg_ack_senders);
        let cov_in_flight_dispatch = Arc::clone(&cov_in_flight);
        let server_tsm_dispatch = Arc::clone(&server_tsm);
        let comm_state_dispatch = Arc::clone(&comm_state);
        let dcc_timer_dispatch = Arc::clone(&dcc_timer);
        let config_dispatch = Arc::new(config.clone());

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
                        let mut received = Some(received);
                        let handled = if let Apdu::ConfirmedRequest(ref req) = decoded {
                            if req.segmented {
                                let seq = req.sequence_number.unwrap_or(0);
                                let key: SegKey = (source_mac.clone(), req.invoke_id);

                                let mut ack_to_send: Option<SegmentAckPdu> = None;
                                let mut final_total: Option<usize> = None;

                                if seq == 0 {
                                    let proposed_window_size =
                                        req.proposed_window_size.unwrap_or(0);
                                    if !(1..=127).contains(&proposed_window_size) {
                                        warn!(
	                                            invoke_id = req.invoke_id,
	                                            proposed_window_size,
	                                            "Rejecting segmented request with invalid proposed window size"
	                                        );
                                        let abort_pdu = Apdu::Abort(AbortPdu {
                                            sent_by_server: true,
                                            invoke_id: req.invoke_id,
                                            abort_reason: AbortReason::WINDOW_SIZE_OUT_OF_RANGE,
                                        });
                                        let mut abort_buf = BytesMut::new();
                                        encode_apdu(&mut abort_buf, &abort_pdu)
                                            .expect("valid APDU encoding");
                                        let _ = network_dispatch
                                            .send_apdu(
                                                &abort_buf,
                                                &source_mac,
                                                false,
                                                NetworkPriority::NORMAL,
                                            )
                                            .await;
                                        continue;
                                    }

                                    if !seg_receivers.contains_key(&key)
                                        && seg_receivers.len() >= MAX_SEG_RECEIVERS
                                    {
                                        let abort_pdu = Apdu::Abort(AbortPdu {
                                            sent_by_server: true,
                                            invoke_id: req.invoke_id,
                                            abort_reason: AbortReason::BUFFER_OVERFLOW,
                                        });
                                        let mut abort_buf = BytesMut::new();
                                        encode_apdu(&mut abort_buf, &abort_pdu)
                                            .expect("valid APDU encoding");
                                        let _ = network_dispatch
                                            .send_apdu(
                                                &abort_buf,
                                                &source_mac,
                                                false,
                                                NetworkPriority::NORMAL,
                                            )
                                            .await;
                                        continue;
                                    }

                                    let mut receiver = SegmentReceiver::new();
                                    if let Err(e) =
                                        receiver.receive(seq, req.service_request.clone())
                                    {
                                        warn!(error = %e, "Rejecting oversized segment");
                                        continue;
                                    }
                                    let actual_window_size = proposed_window_size;
                                    let mut state = SegmentedRequestState {
                                        receiver,
                                        first_req: req.clone(),
                                        last_activity: Instant::now(),
                                        expected_seq: 1,
                                        last_acked_seq: 0,
                                        window_pos: 1,
                                        actual_window_size,
                                    };
                                    let should_ack =
                                        !req.more_follows || state.window_pos >= actual_window_size;
                                    if should_ack {
                                        state.window_pos = 0;
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
                                } else if let Some(state) = seg_receivers.get_mut(&key) {
                                    state.last_activity = Instant::now();
                                    if seq != state.expected_seq {
                                        warn!(
                                            invoke_id = req.invoke_id,
                                            expected = state.expected_seq,
                                            received = seq,
                                            "Segment gap detected, sending negative SegmentAck"
                                        );
                                        ack_to_send = Some(SegmentAckPdu {
                                            negative_ack: true,
                                            sent_by_server: true,
                                            invoke_id: req.invoke_id,
                                            sequence_number: state.last_acked_seq,
                                            actual_window_size: state.actual_window_size,
                                        });
                                    } else {
                                        if let Err(e) =
                                            state.receiver.receive(seq, req.service_request.clone())
                                        {
                                            warn!(error = %e, "Rejecting oversized segment");
                                            continue;
                                        }
                                        state.expected_seq = seq.wrapping_add(1);
                                        state.last_acked_seq = seq;
                                        state.window_pos += 1;
                                        let should_ack = !req.more_follows
                                            || state.window_pos >= state.actual_window_size;
                                        if should_ack {
                                            state.window_pos = 0;
                                            ack_to_send = Some(SegmentAckPdu {
                                                negative_ack: false,
                                                sent_by_server: true,
                                                invoke_id: req.invoke_id,
                                                sequence_number: seq,
                                                actual_window_size: state.actual_window_size,
                                            });
                                        }
                                        if !req.more_follows {
                                            final_total = Some(seq as usize + 1);
                                        }
                                    }
                                } else {
                                    warn!(
	                                        invoke_id = req.invoke_id,
	                                        seq = seq,
	                                        "Received non-initial segment without prior segment 0, aborting"
	                                    );
                                    let abort_pdu = Apdu::Abort(AbortPdu {
                                        sent_by_server: true,
                                        invoke_id: req.invoke_id,
                                        abort_reason: AbortReason::INVALID_APDU_IN_THIS_STATE,
                                    });
                                    let mut abort_buf = BytesMut::new();
                                    encode_apdu(&mut abort_buf, &abort_pdu)
                                        .expect("valid APDU encoding");
                                    let _ = network_dispatch
                                        .send_apdu(
                                            &abort_buf,
                                            &source_mac,
                                            false,
                                            NetworkPriority::NORMAL,
                                        )
                                        .await;
                                    continue;
                                }

                                if let Some(seg_ack) = ack_to_send {
                                    let seg_ack = Apdu::SegmentAck(seg_ack);
                                    let mut ack_buf = BytesMut::new();
                                    encode_apdu(&mut ack_buf, &seg_ack)
                                        .expect("valid APDU encoding");
                                    if let Err(e) = network_dispatch
                                        .send_apdu(
                                            &ack_buf,
                                            &source_mac,
                                            false,
                                            NetworkPriority::NORMAL,
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
	                                                    &cov_in_flight_dispatch,
	                                                    &server_tsm_dispatch,
	                                                    &comm_state_dispatch,
	                                                    &dcc_timer_dispatch,
	                                                    &config_dispatch,
	                                                    &source_mac,
	                                                    Apdu::ConfirmedRequest(reassembled),
	                                                    received.take().unwrap_or_else(|| {
	                                                        warn!("received consumed twice - using empty fallback");
	                                                        bacnet_network::layer::ReceivedApdu {
	                                                            apdu: bytes::Bytes::new(),
	                                                            source_mac: bacnet_types::MacAddr::new(),
	                                                            source_network: None,
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
                                &cov_in_flight_dispatch,
                                &server_tsm_dispatch,
                                &comm_state_dispatch,
                                &dcc_timer_dispatch,
                                &config_dispatch,
                                &source_mac,
                                decoded,
                                received.take().unwrap_or_else(|| {
                                    warn!("received consumed twice — using empty fallback");
                                    bacnet_network::layer::ReceivedApdu {
                                        apdu: bytes::Bytes::new(),
                                        source_mac: bacnet_types::MacAddr::new(),
                                        source_network: None,
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

        let event_enrollment_task = if config.enable_fault_detection {
            let db_ee = Arc::clone(&db);
            Some(tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(10));
                loop {
                    interval.tick().await;
                    let mut db_guard = db_ee.write().await;
                    let transitions =
                        crate::event_enrollment::evaluate_event_enrollments(&mut db_guard);
                    for t in &transitions {
                        debug!(
                            enrollment = %t.enrollment_oid,
                            monitored = %t.monitored_oid,
                            from = ?t.change.from,
                            to = ?t.change.to,
                            "Event enrollment: state changed"
                        );
                    }
                }
            }))
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
                // TODO: Read UTC_Offset from Device object for local time
                crate::schedule::tick_schedules(&db_schedule, 0).await;
            }
        }));

        Ok(Self {
            config,
            network,
            db,
            cov_table,
            seg_ack_senders,
            cov_in_flight,
            server_tsm,
            comm_state,
            dcc_timer,
            dispatch_task: Some(dispatch_task),
            cov_purge_task: Some(cov_purge_task),
            fault_detection_task,
            event_enrollment_task,
            trend_log_task,
            schedule_tick_task,
            local_mac,
        })
    }

    /// Get the server's local MAC address.
    pub fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    /// Get a reference to the shared object database.
    pub fn database(&self) -> &Arc<RwLock<ObjectDatabase>> {
        &self.db
    }

    /// Get the communication state per DeviceCommunicationControl.
    ///
    /// Returns 0 (Enable), 1 (Disable), or 2 (DisableInitiation).
    pub fn comm_state(&self) -> u8 {
        self.comm_state.load(Ordering::Acquire)
    }

    /// Generate a PICS document from the current object database and server configuration.
    ///
    /// The caller must supply a [`PicsConfig`] for fields not available from the server
    /// (vendor name, model, firmware revision, etc.).
    pub async fn generate_pics(&self, pics_config: &crate::pics::PicsConfig) -> crate::pics::Pics {
        let db = self.db.read().await;
        crate::pics::PicsGenerator::new(&db, &self.config, pics_config).generate()
    }

    /// Stop the server.
    pub async fn stop(&mut self) -> Result<(), Error> {
        if let Some(task) = self.fault_detection_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.event_enrollment_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.trend_log_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.schedule_tick_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.cov_purge_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.dispatch_task.take() {
            task.abort();
            let _ = task.await;
        }
        Ok(())
    }
}
