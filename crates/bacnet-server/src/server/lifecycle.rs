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
        let seg_ack_senders: Arc<Mutex<HashMap<SegKey, Arc<SegmentedSendHandle>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));

        let cov_in_flight = Arc::new(Semaphore::new(255));
        let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));
        let comm_state = Arc::new(AtomicU8::new(0)); // 0 = Enable (default)
        let dcc_timer: Arc<Mutex<Option<JoinHandle<()>>>> = Arc::new(Mutex::new(None));

        let network_dispatch = Arc::clone(&network);
        let db_dispatch = Arc::clone(&db);
        let cov_dispatch = Arc::clone(&cov_table);
        let seg_ack_dispatch = Arc::clone(&seg_ack_senders);
        let seg_send_permits_dispatch = Arc::clone(&seg_send_permits);
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
                        let source_network = received.source_network.clone();
                        let mut received = Some(received);
                        let handled = if let Apdu::ConfirmedRequest(ref req) = decoded {
                            if req.segmented {
                                let seq = req.sequence_number.unwrap_or(0);
                                let key: SegKey =
                                    (source_mac.clone(), source_network.clone(), req.invoke_id);

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
                                        let _ = Self::send_confirmed_response_apdu(
                                            &network_dispatch,
                                            &abort_buf,
                                            &source_mac,
                                            source_network.as_ref(),
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
                                        let _ = Self::send_confirmed_response_apdu(
                                            &network_dispatch,
                                            &abort_buf,
                                            &source_mac,
                                            source_network.as_ref(),
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
                                    let _ = Self::send_confirmed_response_apdu(
                                        &network_dispatch,
                                        &abort_buf,
                                        &source_mac,
                                        source_network.as_ref(),
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

        // One-second intrinsic-reporting task: advances the `Time_Delay`
        // countdown for any object with a pending delayed transition and sends
        // the EventNotification when the delay elapses. The per-write path
        // only *seeds* a pending transition (see `fire_event_notifications`);
        // this task is the sole confirmer, so repeated writes cannot shorten
        // the delay (ASHRAE 135-2020 §13.2.4). Runs unconditionally like the
        // trend-log task — a no-pending tick is a cheap empty iteration.
        let db_intrinsic = Arc::clone(&db);
        let network_intrinsic = Arc::clone(&network);
        let comm_state_intrinsic = Arc::clone(&comm_state);
        let server_tsm_intrinsic = Arc::clone(&server_tsm);
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
                if comm_state_intrinsic.load(Ordering::Acquire) >= 1 {
                    continue;
                }
                // Collect confirmed transitions under a brief write lock, then
                // drop it before sending (never hold the db lock across a
                // network send — matches the per-write notification path).
                let fired: Vec<(ObjectIdentifier, bacnet_objects::event::EventStateChange)> = {
                    let mut db = db_intrinsic.write().await;
                    let mut out = Vec::new();
                    db.for_each_object_mut(|oid, object| {
                        if let Some(change) = object.tick_intrinsic_reporting() {
                            out.push((oid, change));
                        }
                    });
                    out
                };
                for (oid, change) in fired {
                    Self::build_and_send_event_notification(
                        &db_intrinsic,
                        &network_intrinsic,
                        &comm_state_intrinsic,
                        &server_tsm_intrinsic,
                        &oid,
                        change,
                        intrinsic_retry_ms,
                    )
                    .await;
                }
            }
        }));

        Ok(Self {
            config,
            network,
            db,
            cov_table,
            seg_ack_senders,
            seg_send_permits,
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
            intrinsic_reporting_task,
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
        Self::fire_event_notifications(
            &self.db,
            &self.network,
            &self.comm_state,
            &self.server_tsm,
            oid,
            self.config.cov_retry_timeout_ms,
        )
        .await;
        Self::fire_cov_notifications(
            &self.db,
            &self.network,
            &self.cov_table,
            &self.cov_in_flight,
            &self.server_tsm,
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
        if let Some(task) = self.intrinsic_reporting_task.take() {
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
