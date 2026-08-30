use super::*;

mod confirmed_response;
mod endpoint_responder;
#[cfg(test)]
#[path = "endpoint_shared_runtime_tests.rs"]
mod endpoint_shared_runtime_tests;
mod unconfirmed;
#[cfg(test)]
mod unconfirmed_tests;
#[cfg(test)]
pub(crate) use unconfirmed::EXECUTED_UNCONFIRMED;

#[cfg(test)]
/// Every confirmed service choice with an inbound execution arm in
/// `handle_confirmed_request` below. Keep in lockstep with the `match` —
/// the `executed_services_match_dispatch_table` test compares this list
/// (mapped through [`ServiceSupported::from_confirmed_choice`]) against
/// [`bacnet_objects::device::EXECUTED_SERVICES`], which is what the Device
/// object advertises in `Protocol_Services_Supported`.
///
/// [`ServiceSupported::from_confirmed_choice`]: bacnet_types::enums::ServiceSupported::from_confirmed_choice
pub(crate) const EXECUTED_CONFIRMED: &[ConfirmedServiceChoice] = &[
    ConfirmedServiceChoice::ACKNOWLEDGE_ALARM,
    ConfirmedServiceChoice::GET_ALARM_SUMMARY,
    ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY,
    ConfirmedServiceChoice::SUBSCRIBE_COV,
    ConfirmedServiceChoice::ATOMIC_READ_FILE,
    ConfirmedServiceChoice::ATOMIC_WRITE_FILE,
    ConfirmedServiceChoice::ADD_LIST_ELEMENT,
    ConfirmedServiceChoice::REMOVE_LIST_ELEMENT,
    ConfirmedServiceChoice::CREATE_OBJECT,
    ConfirmedServiceChoice::DELETE_OBJECT,
    ConfirmedServiceChoice::READ_PROPERTY,
    ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
    ConfirmedServiceChoice::WRITE_PROPERTY,
    ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
    ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
    ConfirmedServiceChoice::CONFIRMED_TEXT_MESSAGE,
    ConfirmedServiceChoice::REINITIALIZE_DEVICE,
    ConfirmedServiceChoice::READ_RANGE,
    ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
    ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
    ConfirmedServiceChoice::GET_EVENT_INFORMATION,
    ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
    ConfirmedServiceChoice::AUDIT_LOG_QUERY,
];

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Handle a confirmed request.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn handle_confirmed_request(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        seg_ack_senders: &Arc<Mutex<HashMap<SegKey, Arc<SegmentedSendHandle>>>>,
        seg_send_permits: &Arc<Semaphore>,
        cov_in_flight: &Arc<Semaphore>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        notification_transactions: &Arc<NotificationTransactions>,
        device_bindings: &Arc<RwLock<DeviceBindingTable>>,
        comm_state: &Arc<AtomicU8>,
        dcc_timer: &Arc<Mutex<Option<JoinHandle<()>>>>,
        config: &ServerConfig,
        source_mac: &[u8],
        source_network: Option<NpduAddress>,
        req: bacnet_encoding::apdu::ConfirmedRequest,
        reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
    ) {
        enum InitialCovNotification {
            Single(CovSubscription),
            Multiple(Vec<CovSubscription>),
        }

        let invoke_id = req.invoke_id;
        let service_choice = req.service_choice;
        let client_max_apdu = req.max_apdu_length;
        let client_accepts_segmented = req.segmented_response_accepted;
        let client_max_segments = req.max_segments;
        let mut written_oids: Vec<ObjectIdentifier> = Vec::new();
        let mut initial_cov_notifications: Vec<InitialCovNotification> = Vec::new();

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
                confirmed_response::read_property_response(db, &req).await
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
                let (result, residual_oids) = {
                    let mut db = db.write().await;
                    handlers::handle_write_property_multiple_with_residuals(
                        &mut db,
                        &req.service_request,
                    )
                };
                written_oids = residual_oids;
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
                match handlers::handle_subscribe_cov_with_initial_endpoint(
                    &mut table,
                    &db,
                    source_mac,
                    source_network.as_ref(),
                    &req.service_request,
                ) {
                    Ok(subscriptions) => {
                        initial_cov_notifications.extend(
                            subscriptions
                                .into_iter()
                                .map(InitialCovNotification::Single),
                        );
                        simple_ack()
                    }
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY => {
                let db = db.read().await;
                let mut table = cov_table.write().await;
                match handlers::handle_subscribe_cov_property_with_initial_endpoint(
                    &mut table,
                    &db,
                    source_mac,
                    source_network.as_ref(),
                    &req.service_request,
                ) {
                    Ok(subscriptions) => {
                        initial_cov_notifications.extend(
                            subscriptions
                                .into_iter()
                                .map(InitialCovNotification::Single),
                        );
                        simple_ack()
                    }
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
            s if s == ConfirmedServiceChoice::AUDIT_LOG_QUERY => {
                // Query under the read guard, then release it before ACK
                // construction/encoding and the generic segmentation path.
                let query_result = {
                    let db = db.read().await;
                    handlers::handle_audit_log_query(&db, &req.service_request)
                };
                match query_result {
                    Ok((audit_log, page)) => {
                        let ack = bacnet_services::audit::AuditLogQueryAck {
                            audit_log,
                            records: page.records,
                            no_more_items: page.no_more_items,
                        };
                        match ack.try_encode(&mut ack_buf) {
                            Ok(()) => complex_ack(ack_buf),
                            Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                        }
                    }
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
                let request = bacnet_services::life_safety::LifeSafetyOperationRequest::decode(
                    &req.service_request,
                );
                match request {
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                    Ok(request) => {
                        let validation = handlers::validate_life_safety_operation(request.request);
                        let execution = if let Err(e) = validation {
                            Err(e)
                        } else {
                            let target_exists = match request.object_identifier {
                                Some(oid) => db.read().await.get(&oid).is_some(),
                                None => true,
                            };

                            if !target_exists {
                                Err(handlers::life_safety_error(
                                    ErrorClass::OBJECT,
                                    ErrorCode::UNKNOWN_OBJECT,
                                ))
                            } else {
                                let context = LifeSafetyOperationAuthorizationContext {
                                    source_mac: MacAddr::from_slice(source_mac),
                                    source_network: source_network.clone(),
                                    invoke_id,
                                    request: request.clone(),
                                };
                                let authorized = config
                                    .life_safety_operation_authorizer
                                    .as_ref()
                                    .is_some_and(|authorizer| {
                                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                                            || authorizer(&context),
                                        ))
                                        .unwrap_or(false)
                                    });
                                if !authorized {
                                    Err(handlers::life_safety_error(
                                        ErrorClass::SERVICES,
                                        ErrorCode::SERVICE_REQUEST_DENIED,
                                    ))
                                } else {
                                    let mut db = db.write().await;
                                    handlers::handle_life_safety_operation(&mut db, &request)
                                }
                            }
                        };

                        match execution {
                            // LifeSafetyOperation changes Silenced and
                            // Operation_Expected, not the whole-object COV and
                            // event inputs represented by written_oids. A
                            // property-aware COV path remains follow-up #177.
                            Ok(_changed) => simple_ack(),
                            Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                        }
                    }
                }
            }
            s if s == ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE => {
                let decoded =
                    bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleRequest::decode(
                        &req.service_request,
                    );
                match decoded {
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                    Ok(request) => {
                        let db = db.read().await;
                        let mut table = cov_table.write().await;
                        match handlers::handle_subscribe_cov_property_multiple_request_endpoint(
                            &mut table,
                            &db,
                            source_mac,
                            source_network.as_ref(),
                            request,
                        ) {
                            Ok(subscriptions) => {
                                if !subscriptions.is_empty() {
                                    initial_cov_notifications
                                        .push(InitialCovNotification::Multiple(subscriptions));
                                }
                                simple_ack()
                            }
                            Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                        }
                    }
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
                // Clause 5.4.5.3 CannotSendSegmentedComplexACK reads both
                // sides of the exchange: case (a) — "this device does not
                // support the transmission of segmented messages" — and case
                // (b), the client not accepting one. Either way the response
                // "cannot be sent as one PDU or multiple PDUs" and draws the
                // same Abort; SendSegmentedComplexACK is available only when
                // the device supports transmitting segments (#381).
                let device_transmits_segments = config.segmentation_supported == Segmentation::BOTH
                    || config.segmentation_supported == Segmentation::TRANSMIT;
                if !client_accepts_segmented || !device_transmits_segments {
                    let abort = Apdu::Abort(AbortPdu {
                        sent_by_server: true,
                        invoke_id,
                        abort_reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
                    });
                    let mut buf = BytesMut::new();
                    encode_apdu(&mut buf, &abort).expect("valid APDU encoding");
                    if let Err(e) = Self::send_confirmed_response_apdu(
                        network,
                        &buf,
                        source_mac,
                        source_network.as_ref(),
                    )
                    .await
                    {
                        warn!(error = %e, "Failed to send Abort for segmentation-not-supported");
                    }
                } else {
                    let network = Arc::clone(network);
                    let seg_ack_senders = Arc::clone(seg_ack_senders);
                    let seg_send_permits = Arc::clone(seg_send_permits);
                    let source_mac = MacAddr::from_slice(source_mac);
                    let service_ack_data = ack.service_ack.clone();
                    tokio::spawn(async move {
                        Self::send_segmented_complex_ack(
                            &network,
                            &seg_ack_senders,
                            &seg_send_permits,
                            &source_mac,
                            source_network.as_ref(),
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
                    Self::fire_event_notifications_with_bindings(
                        db,
                        network,
                        comm_state,
                        server_tsm,
                        notification_transactions,
                        device_bindings,
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
                        notification_transactions,
                        comm_state,
                        config,
                        oid,
                    )
                    .await;
                }
                for notification in &initial_cov_notifications {
                    match notification {
                        InitialCovNotification::Single(subscription) => {
                            Self::fire_initial_cov_notification(
                                db,
                                network,
                                cov_table,
                                cov_in_flight,
                                notification_transactions,
                                comm_state,
                                config,
                                subscription,
                            )
                            .await;
                        }
                        InitialCovNotification::Multiple(subscriptions) => {
                            Self::fire_initial_cov_notification_multiple(
                                db,
                                network,
                                cov_table,
                                cov_in_flight,
                                notification_transactions,
                                comm_state,
                                config,
                                subscriptions,
                            )
                            .await;
                        }
                    }
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
                destination: source_network.clone(),
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
                    if let Err(e) = Self::send_confirmed_response_apdu(
                        network,
                        &apdu_bytes,
                        source_mac,
                        source_network.as_ref(),
                    )
                    .await
                    {
                        warn!(error = %e, "Failed to send response");
                    }
                }
            }
        } else if let Err(e) =
            Self::send_confirmed_response_apdu(network, &buf, source_mac, source_network.as_ref())
                .await
        {
            warn!(error = %e, "Failed to send response");
        }

        for oid in &written_oids {
            Self::fire_event_notifications_with_bindings(
                db,
                network,
                comm_state,
                server_tsm,
                notification_transactions,
                device_bindings,
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
                notification_transactions,
                comm_state,
                config,
                oid,
            )
            .await;
        }

        for notification in &initial_cov_notifications {
            match notification {
                InitialCovNotification::Single(subscription) => {
                    Self::fire_initial_cov_notification(
                        db,
                        network,
                        cov_table,
                        cov_in_flight,
                        notification_transactions,
                        comm_state,
                        config,
                        subscription,
                    )
                    .await;
                }
                InitialCovNotification::Multiple(subscriptions) => {
                    Self::fire_initial_cov_notification_multiple(
                        db,
                        network,
                        cov_table,
                        cov_in_flight,
                        notification_transactions,
                        comm_state,
                        config,
                        subscriptions,
                    )
                    .await;
                }
            }
        }
    }
    /// Convert an error into its protocol response APDU.
    pub(super) fn error_apdu_from_error(
        invoke_id: u8,
        service_choice: ConfirmedServiceChoice,
        error: &Error,
    ) -> Apdu {
        confirmed_response::error_apdu_from_error(invoke_id, service_choice, error)
    }
}
