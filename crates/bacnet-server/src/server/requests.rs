use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Handle a confirmed request.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn handle_confirmed_request(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        seg_ack_senders: &Arc<Mutex<HashMap<SegKey, mpsc::Sender<SegmentAckPdu>>>>,
        cov_in_flight: &Arc<Semaphore>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        comm_state: &Arc<AtomicU8>,
        dcc_timer: &Arc<Mutex<Option<JoinHandle<()>>>>,
        config: &ServerConfig,
        source_mac: &[u8],
        req: bacnet_encoding::apdu::ConfirmedRequest,
        reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
    ) {
        let invoke_id = req.invoke_id;
        let service_choice = req.service_choice;
        let client_max_apdu = req.max_apdu_length;
        let client_accepts_segmented = req.segmented_response_accepted;
        let client_max_segments = req.max_segments;
        let mut written_oids: Vec<ObjectIdentifier> = Vec::new();

        let state = comm_state.load(Ordering::Acquire);
        if state == 1
            && service_choice != ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL
            && service_choice != ConfirmedServiceChoice::REINITIALIZE_DEVICE
        {
            debug!(
                service = service_choice.to_raw(),
                "DCC DISABLE: dropping confirmed request"
            );
            return;
        }

        let complex_ack = |ack_buf: BytesMut| -> Apdu {
            Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice,
                service_ack: ack_buf.freeze(),
            })
        };
        let simple_ack = || -> Apdu {
            Apdu::SimpleAck(SimpleAck {
                invoke_id,
                service_choice,
            })
        };

        let mut ack_buf = BytesMut::with_capacity(512);
        let response = match service_choice {
            s if s == ConfirmedServiceChoice::READ_PROPERTY => {
                let db = db.read().await;
                match handlers::handle_read_property(&db, &req.service_request, &mut ack_buf) {
                    Ok(()) => complex_ack(ack_buf),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::WRITE_PROPERTY => {
                let result = {
                    let mut db = db.write().await;
                    handlers::handle_write_property(&mut db, &req.service_request)
                };
                match result {
                    Ok(oid) => {
                        written_oids.push(oid);
                        simple_ack()
                    }
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE => {
                let db = db.read().await;
                match handlers::handle_read_property_multiple(
                    &db,
                    &req.service_request,
                    &mut ack_buf,
                ) {
                    Ok(()) => complex_ack(ack_buf),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE => {
                let result = {
                    let mut db = db.write().await;
                    handlers::handle_write_property_multiple(&mut db, &req.service_request)
                };
                match result {
                    Ok(oids) => {
                        written_oids = oids;
                        simple_ack()
                    }
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::SUBSCRIBE_COV => {
                let db = db.read().await;
                let mut table = cov_table.write().await;
                match handlers::handle_subscribe_cov(
                    &mut table,
                    &db,
                    source_mac,
                    &req.service_request,
                ) {
                    Ok(()) => simple_ack(),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY => {
                let db = db.read().await;
                let mut table = cov_table.write().await;
                match handlers::handle_subscribe_cov_property(
                    &mut table,
                    &db,
                    source_mac,
                    &req.service_request,
                ) {
                    Ok(()) => simple_ack(),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::CREATE_OBJECT => {
                let result = {
                    let mut db = db.write().await;
                    handlers::handle_create_object(&mut db, &req.service_request, &mut ack_buf)
                };
                match result {
                    Ok(()) => complex_ack(ack_buf),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::DELETE_OBJECT => {
                let deleted_oid =
                    bacnet_services::object_mgmt::DeleteObjectRequest::decode(&req.service_request)
                        .ok()
                        .map(|r| r.object_identifier);

                let result = {
                    let mut db = db.write().await;
                    handlers::handle_delete_object(&mut db, &req.service_request)
                };
                match result {
                    Ok(()) => {
                        // Clean up COV subscriptions for the deleted object
                        if let Some(oid) = deleted_oid {
                            let mut table = cov_table.write().await;
                            table.remove_for_object(oid);
                        }
                        simple_ack()
                    }
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL => {
                match handlers::handle_device_communication_control(
                    &req.service_request,
                    comm_state,
                    &config.dcc_password,
                ) {
                    Ok((_state, duration)) => {
                        if let Some(prev) = dcc_timer.lock().await.take() {
                            prev.abort();
                        }
                        if let Some(minutes) = duration {
                            let comm = Arc::clone(comm_state);
                            let handle = tokio::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_secs(
                                    minutes as u64 * 60,
                                ))
                                .await;
                                comm.store(0, Ordering::Release);
                                tracing::debug!(
                                    "DCC timer expired after {} min, state reverted to ENABLE",
                                    minutes
                                );
                            });
                            *dcc_timer.lock().await = Some(handle);
                        }
                        simple_ack()
                    }
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::REINITIALIZE_DEVICE => {
                match handlers::handle_reinitialize_device(
                    &req.service_request,
                    &config.reinit_password,
                ) {
                    Ok(()) => simple_ack(),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::GET_EVENT_INFORMATION => {
                let db = db.read().await;
                match handlers::handle_get_event_information(
                    &db,
                    &req.service_request,
                    &mut ack_buf,
                ) {
                    Ok(()) => complex_ack(ack_buf),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::ACKNOWLEDGE_ALARM => {
                let mut db = db.write().await;
                match handlers::handle_acknowledge_alarm(&mut db, &req.service_request) {
                    Ok(()) => simple_ack(),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::READ_RANGE => {
                let db = db.read().await;
                match handlers::handle_read_range(&db, &req.service_request, &mut ack_buf) {
                    Ok(()) => complex_ack(ack_buf),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::ATOMIC_READ_FILE => {
                let db = db.read().await;
                match handlers::handle_atomic_read_file(&db, &req.service_request, &mut ack_buf) {
                    Ok(()) => complex_ack(ack_buf),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::ATOMIC_WRITE_FILE => {
                let result = {
                    let mut db = db.write().await;
                    handlers::handle_atomic_write_file(&mut db, &req.service_request, &mut ack_buf)
                };
                match result {
                    Ok(()) => complex_ack(ack_buf),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::ADD_LIST_ELEMENT => {
                let mut db = db.write().await;
                match handlers::handle_add_list_element(&mut db, &req.service_request) {
                    Ok(()) => simple_ack(),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::REMOVE_LIST_ELEMENT => {
                let mut db = db.write().await;
                match handlers::handle_remove_list_element(&mut db, &req.service_request) {
                    Ok(()) => simple_ack(),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::GET_ALARM_SUMMARY => {
                let mut buf = BytesMut::new();
                let db = db.read().await;
                match handlers::handle_get_alarm_summary(&db, &mut buf) {
                    Ok(()) => complex_ack(buf),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY => {
                let mut buf = BytesMut::new();
                let db = db.read().await;
                match handlers::handle_get_enrollment_summary(&db, &req.service_request, &mut buf) {
                    Ok(()) => complex_ack(buf),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::CONFIRMED_TEXT_MESSAGE => {
                match handlers::handle_text_message(&req.service_request) {
                    Ok(_msg) => simple_ack(),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::LIFE_SAFETY_OPERATION => {
                match handlers::handle_life_safety_operation(&req.service_request) {
                    Ok(()) => simple_ack(),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE => {
                let db = db.read().await;
                let mut table = cov_table.write().await;
                match handlers::handle_subscribe_cov_property_multiple(
                    &mut table,
                    &db,
                    source_mac,
                    &req.service_request,
                ) {
                    Ok(()) => simple_ack(),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            _ => {
                debug!(
                    service = service_choice.to_raw(),
                    "Unsupported confirmed service"
                );
                Apdu::Reject(RejectPdu {
                    invoke_id,
                    reject_reason: RejectReason::UNRECOGNIZED_SERVICE,
                })
            }
        };

        if let Apdu::ComplexAck(ref ack) = response {
            let mut full_buf = BytesMut::new();
            encode_apdu(&mut full_buf, &response).expect("valid APDU encoding");

            if full_buf.len() > client_max_apdu as usize {
                if !client_accepts_segmented {
                    let abort = Apdu::Abort(AbortPdu {
                        sent_by_server: true,
                        invoke_id,
                        abort_reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
                    });
                    let mut buf = BytesMut::new();
                    encode_apdu(&mut buf, &abort).expect("valid APDU encoding");
                    if let Err(e) = network
                        .send_apdu(&buf, source_mac, false, NetworkPriority::NORMAL)
                        .await
                    {
                        warn!(error = %e, "Failed to send Abort for segmentation-not-supported");
                    }
                } else {
                    let network = Arc::clone(network);
                    let seg_ack_senders = Arc::clone(seg_ack_senders);
                    let source_mac = MacAddr::from_slice(source_mac);
                    let service_ack_data = ack.service_ack.clone();
                    tokio::spawn(async move {
                        Self::send_segmented_complex_ack(
                            &network,
                            &seg_ack_senders,
                            &source_mac,
                            invoke_id,
                            service_choice,
                            &service_ack_data,
                            client_max_apdu,
                            client_max_segments,
                        )
                        .await;
                    });
                }

                for oid in &written_oids {
                    Self::fire_event_notifications(
                        db,
                        network,
                        comm_state,
                        server_tsm,
                        oid,
                        config.cov_retry_timeout_ms,
                    )
                    .await;
                }
                for oid in &written_oids {
                    Self::fire_cov_notifications(
                        db,
                        network,
                        cov_table,
                        cov_in_flight,
                        server_tsm,
                        comm_state,
                        config,
                        oid,
                    )
                    .await;
                }
                return;
            }
        }

        let mut buf = BytesMut::new();
        encode_apdu(&mut buf, &response).expect("valid APDU encoding");

        if let Some(tx) = reply_tx {
            use bacnet_encoding::npdu::{encode_npdu, Npdu};
            let apdu_bytes = buf.freeze();
            let npdu = Npdu {
                is_network_message: false,
                expecting_reply: false,
                priority: NetworkPriority::NORMAL,
                destination: None,
                source: None,
                payload: apdu_bytes.clone(),
                ..Npdu::default()
            };
            let mut npdu_buf = BytesMut::with_capacity(2 + apdu_bytes.len());
            match encode_npdu(&mut npdu_buf, &npdu) {
                Ok(()) => {
                    let _ = tx.send(npdu_buf.freeze());
                }
                Err(e) => {
                    warn!(error = %e, "Failed to encode NPDU for MS/TP reply");
                    if let Err(e) = network
                        .send_apdu(&apdu_bytes, source_mac, false, NetworkPriority::NORMAL)
                        .await
                    {
                        warn!(error = %e, "Failed to send response");
                    }
                }
            }
        } else if let Err(e) = network
            .send_apdu(&buf, source_mac, false, NetworkPriority::NORMAL)
            .await
        {
            warn!(error = %e, "Failed to send response");
        }

        for oid in &written_oids {
            Self::fire_event_notifications(
                db,
                network,
                comm_state,
                server_tsm,
                oid,
                config.cov_retry_timeout_ms,
            )
            .await;
        }

        for oid in &written_oids {
            Self::fire_cov_notifications(
                db,
                network,
                cov_table,
                cov_in_flight,
                server_tsm,
                comm_state,
                config,
                oid,
            )
            .await;
        }
    }
    /// Handle an unconfirmed request (e.g., WhoIs).
    pub(super) async fn handle_unconfirmed_request(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        config: &ServerConfig,
        comm_state: &Arc<AtomicU8>,
        req: UnconfirmedRequestPdu,
        received: &bacnet_network::layer::ReceivedApdu,
    ) {
        let comm = comm_state.load(Ordering::Acquire);
        if comm == 1 {
            tracing::debug!("Dropping unconfirmed service: DCC is DISABLE");
            return;
        }

        if req.service_choice == UnconfirmedServiceChoice::WHO_IS {
            let who_is = match WhoIsRequest::decode(&req.service_request) {
                Ok(r) => r,
                Err(e) => {
                    warn!(error = %e, "Failed to decode WhoIs");
                    return;
                }
            };

            let db = db.read().await;
            let device_oid = db
                .list_objects()
                .into_iter()
                .find(|oid| oid.object_type() == ObjectType::DEVICE);

            if let Some(device_oid) = device_oid {
                let instance = device_oid.instance_number();

                let in_range = match (who_is.low_limit, who_is.high_limit) {
                    (Some(low), Some(high)) => instance >= low && instance <= high,
                    _ => true,
                };

                if in_range {
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
                    encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

                    if let Some(ref source_net) = received.source_network {
                        if let Err(e) = network
                            .send_apdu_routed(
                                &buf,
                                source_net.network,
                                &source_net.mac_address,
                                &received.source_mac,
                                false,
                                NetworkPriority::NORMAL,
                            )
                            .await
                        {
                            warn!(error = %e, "Failed to route IAm back to remote requester");
                        }
                    } else if let Err(e) = network
                        .broadcast_apdu(&buf, false, NetworkPriority::NORMAL)
                        .await
                    {
                        warn!(error = %e, "Failed to send IAm broadcast");
                    }
                }
            }
        } else if req.service_choice == UnconfirmedServiceChoice::WHO_HAS {
            let db = db.read().await;
            let device_oid = db
                .list_objects()
                .into_iter()
                .find(|oid| oid.object_type() == ObjectType::DEVICE);

            if let Some(device_oid) = device_oid {
                match handlers::handle_who_has(&db, &req.service_request, device_oid) {
                    Ok(Some(i_have)) => {
                        let mut service_buf = BytesMut::new();
                        if let Err(e) = i_have.encode(&mut service_buf) {
                            warn!(error = %e, "Failed to encode IHave");
                        } else {
                            let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                                service_choice: UnconfirmedServiceChoice::I_HAVE,
                                service_request: service_buf.freeze(),
                            });

                            let mut buf = BytesMut::new();
                            encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

                            if let Err(e) = network
                                .broadcast_apdu(&buf, false, NetworkPriority::NORMAL)
                                .await
                            {
                                warn!(error = %e, "Failed to send IHave broadcast");
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        warn!(error = %e, "Failed to decode WhoHas");
                    }
                }
            }
        } else if req.service_choice == UnconfirmedServiceChoice::TIME_SYNCHRONIZATION
            || req.service_choice == UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION
        {
            debug!("Received time synchronization request");
            if let Some(ref callback) = config.on_time_sync {
                let data = TimeSyncData {
                    raw_service_data: req.service_request.clone(),
                    is_utc: req.service_choice
                        == UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION,
                };
                callback(data);
            }
        } else if req.service_choice == UnconfirmedServiceChoice::WRITE_GROUP {
            match handlers::handle_write_group(&req.service_request) {
                Ok(write_group) => {
                    debug!(
                        group = write_group.group_number,
                        priority = write_group.write_priority,
                        values = write_group.change_list.len(),
                        "WriteGroup received"
                    );
                }
                Err(e) => {
                    debug!(error = %e, "WriteGroup decode failed");
                }
            }
        } else if req.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_TEXT_MESSAGE {
            match handlers::handle_text_message(&req.service_request) {
                Ok(msg) => {
                    debug!(
                        source = ?msg.source_device,
                        priority = ?msg.message_priority,
                        "UnconfirmedTextMessage: {}",
                        msg.message
                    );
                }
                Err(e) => {
                    debug!(error = %e, "UnconfirmedTextMessage decode failed");
                }
            }
        } else {
            debug!(
                service = req.service_choice.to_raw(),
                "Ignoring unsupported unconfirmed service"
            );
        }
    }
    /// Convert an Error into an Error APDU.
    fn error_apdu_from_error(
        invoke_id: u8,
        service_choice: ConfirmedServiceChoice,
        error: &Error,
    ) -> Apdu {
        let (class, code) = match error {
            Error::Protocol { class, code } => (*class, *code),
            _ => (
                ErrorClass::SERVICES.to_raw() as u32,
                ErrorCode::OTHER.to_raw() as u32,
            ),
        };
        Apdu::Error(ErrorPdu {
            invoke_id,
            service_choice,
            error_class: ErrorClass::from_raw(class as u16),
            error_code: ErrorCode::from_raw(code as u16),
            error_data: Bytes::new(),
        })
    }
}
