use super::*;

mod acknowledge_alarm;
mod alarm_summary;
mod atomic_read_file;
mod atomic_write_file;
mod audit_notification;
mod confirmed;
pub(super) mod confirmed_response;
mod dcc;
mod endpoint_responder;
#[cfg(test)]
#[path = "endpoint_shared_runtime_tests.rs"]
mod endpoint_shared_runtime_tests;
mod enrollment_summary;
mod event_information;
mod mutations;
use mutations::InitialCovNotification;
#[cfg(test)]
mod executed;
#[cfg(test)]
mod mutation_boundary_tests;
#[cfg(test)]
mod mutation_tests;
#[cfg(test)]
mod mutation_wpm_tests;
mod read_range;
mod unconfirmed;
#[cfg(test)]
mod unconfirmed_tests;
#[cfg(test)]
pub(crate) use self::{executed::EXECUTED_CONFIRMED, unconfirmed::EXECUTED_UNCONFIRMED};

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Handle one admitted confirmed request.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::server) async fn handle_admitted_confirmed_request(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        seg_ack_senders: &Arc<segmented_send::SegmentedSendRegistry>,
        seg_send_permits: &Arc<Semaphore>,
        cov_in_flight: &Arc<Semaphore>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        notification_transactions: &Arc<NotificationTransactions>,
        device_bindings: &Arc<RwLock<DeviceBindingTable>>,
        comm_state: &Arc<AtomicU8>,
        dcc_timer: &Arc<Mutex<Option<JoinHandle<()>>>>,
        dcc_outcomes: &Arc<dcc_outcomes::DccOutcomes>,
        config: &ServerConfig,
        request_tasks: &super::request_tasks::RequestTaskSpawner,
        source_mac: &[u8],
        source_network: Option<NpduAddress>,
        req: bacnet_encoding::apdu::ConfirmedRequest,
        reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
    ) {
        let invoke_id = req.invoke_id;
        let service_choice = req.service_choice;
        let client_max_apdu = req.max_apdu_length;
        let client_accepts_segmented = req.segmented_response_accepted;
        let client_max_segments = req.max_segments;
        let effective_max_apdu = event_information::limit(client_max_apdu, config.max_apdu_length);
        let device_transmits_segments =
            event_information::can_segment(config.segmentation_supported);
        let segmented_response_available = client_accepts_segmented && device_transmits_segments;
        let (mut written_oids, mut coarse_cov_oids) = (Vec::new(), Vec::new());
        let mut life_safety_cov_changes = Vec::new();
        let mut staging_plans = Vec::new();
        let mut initial_cov_notifications: Vec<InitialCovNotification> = Vec::new();
        let mut accepted_acknowledgment = None;

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
        let mutation = mutations::Request {
            config,
            source_mac,
            source_network: source_network.as_ref(),
            req: &req,
        };
        let response = match service_choice {
            s if s == ConfirmedServiceChoice::READ_PROPERTY => {
                confirmed_response::read_property_response(db, &req).await
            }
            s if s == ConfirmedServiceChoice::WRITE_PROPERTY => {
                mutation
                    .write_property::<T>(
                        db,
                        &mut written_oids,
                        &mut coarse_cov_oids,
                        &mut life_safety_cov_changes,
                        &mut staging_plans,
                    )
                    .await
            }
            s if s == ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE => {
                let db = db.read().await;
                match handlers::handle_rpm_budgeted(
                    &db,
                    &req.service_request,
                    &mut ack_buf,
                    config.read_property_multiple_budget,
                ) {
                    Ok(()) => complex_ack(ack_buf),
                    Err(handlers::RpmFailure::Service(e)) => {
                        Self::error_apdu_from_error(invoke_id, service_choice, &e)
                    }
                    Err(failure) => Apdu::Abort(AbortPdu {
                        sent_by_server: true,
                        invoke_id,
                        abort_reason: match failure {
                            handlers::RpmFailure::Work => AbortReason::OUT_OF_RESOURCES,
                            handlers::RpmFailure::Bytes => AbortReason::BUFFER_OVERFLOW,
                            handlers::RpmFailure::Service(_) => unreachable!(),
                        },
                    }),
                }
            }
            s if s == ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE => {
                mutation
                    .write_property_multiple::<T>(
                        db,
                        &mut written_oids,
                        &mut coarse_cov_oids,
                        &mut life_safety_cov_changes,
                        &mut staging_plans,
                    )
                    .await
            }
            s if s == ConfirmedServiceChoice::SUBSCRIBE_COV => {
                mutation
                    .subscribe_cov::<T>(db, cov_table, &mut initial_cov_notifications)
                    .await
            }
            s if s == ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY => {
                mutation
                    .subscribe_cov_property::<T>(db, cov_table, &mut initial_cov_notifications)
                    .await
            }
            s if s == ConfirmedServiceChoice::CREATE_OBJECT => {
                mutation.create_object::<T>(db, ack_buf).await
            }
            s if s == ConfirmedServiceChoice::DELETE_OBJECT => {
                mutation.delete_object::<T>(db, cov_table).await
            }
            s if s == ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL => {
                dcc::response::<T>(
                    dcc_timer,
                    comm_state,
                    dcc_outcomes,
                    config,
                    &req,
                    source_mac,
                    source_network.as_ref(),
                    request_tasks,
                )
                .await
            }
            s if s == ConfirmedServiceChoice::REINITIALIZE_DEVICE => {
                let password = &config.reinit_password;
                let error = handlers::handle_reinitialize_device(&req.service_request, password)
                    .err()
                    .unwrap_or(Error::Protocol {
                        class: ErrorClass::SERVICES.to_raw() as u32,
                        code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
                    });
                Self::error_apdu_from_error(invoke_id, service_choice, &error)
            }
            s if s == ConfirmedServiceChoice::GET_EVENT_INFORMATION => {
                event_information::response(
                    db,
                    &req,
                    config.get_event_information_budget,
                    effective_max_apdu,
                    segmented_response_available,
                )
                .await
            }
            s if s == ConfirmedServiceChoice::ACKNOWLEDGE_ALARM => {
                acknowledge_alarm::response(db, &req, &mut accepted_acknowledgment).await
            }
            s if s == ConfirmedServiceChoice::READ_RANGE => {
                read_range::response(
                    db,
                    &req,
                    config.read_range_budget,
                    effective_max_apdu,
                    segmented_response_available,
                )
                .await
            }
            s if s == ConfirmedServiceChoice::ATOMIC_READ_FILE => {
                let db = db.read().await;
                Self::atomic_read_file_response(
                    &db,
                    invoke_id,
                    &req.service_request,
                    config.atomic_read_file_budget,
                )
            }
            s if s == ConfirmedServiceChoice::ATOMIC_WRITE_FILE => {
                mutation.atomic_write_file::<T>(db).await
            }
            s if s == ConfirmedServiceChoice::ADD_LIST_ELEMENT => {
                mutation.add_list_element::<T>(db).await
            }
            s if s == ConfirmedServiceChoice::REMOVE_LIST_ELEMENT => {
                mutation.remove_list_element::<T>(db).await
            }
            s if s == ConfirmedServiceChoice::GET_ALARM_SUMMARY => {
                let db = db.read().await;
                Self::alarm_summary_response(&db, invoke_id, config.get_alarm_summary_budget)
            }
            s if s == ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY => {
                let db = db.read().await;
                Self::enrollment_summary_response(
                    &db,
                    invoke_id,
                    &req.service_request,
                    config.get_enrollment_summary_budget,
                )
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
            s if s == ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION => {
                match audit_notification::receive_confirmed_audit_notification(
                    db,
                    config,
                    source_mac,
                    source_network.as_ref(),
                    &req,
                )
                .await
                {
                    Ok(audit_notification::Stored) => simple_ack(),
                    Ok(audit_notification::Duplicate) => return,
                    Err(error) => Self::error_apdu_from_error(invoke_id, service_choice, &error),
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
                                    handlers::handle_life_safety_operation_detailed(
                                        &mut db, &request,
                                    )
                                }
                            }
                        };

                        match execution {
                            Ok(result) => {
                                life_safety_cov_changes.extend(result.cov_changes);
                                simple_ack()
                            }
                            Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                        }
                    }
                }
            }
            s if s == ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE => {
                mutation
                    .subscribe_cov_property_multiple::<T>(
                        db,
                        cov_table,
                        &mut initial_cov_notifications,
                    )
                    .await
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

        Self::execute_staging_plans(
            db,
            network,
            cov_table,
            cov_in_flight,
            server_tsm,
            notification_transactions,
            device_bindings,
            comm_state,
            config,
            staging_plans,
        )
        .await;

        if let Apdu::ComplexAck(ref ack) = response {
            let mut full_buf = BytesMut::new();
            encode_apdu(&mut full_buf, &response).expect("valid APDU encoding");

            if full_buf.len() > effective_max_apdu as usize {
                // Clause 5.4.5.3 CannotSendSegmentedComplexACK reads both
                // sides of the exchange: case (a), no local capability to
                // transmit segmented messages, and case
                // (b), the client not accepting one. Either way the response
                // fits neither an unsegmented nor a segmented send and draws the
                // same Abort; SendSegmentedComplexACK is available only when
                // the device supports transmitting segments (#381).
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
                    Self::spawn_segmented_complex_ack(
                        network,
                        seg_ack_senders,
                        seg_send_permits,
                        request_tasks,
                        source_mac,
                        source_network,
                        invoke_id,
                        service_choice,
                        ack.service_ack.clone(),
                        effective_max_apdu,
                        client_max_segments,
                    );
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
                Self::fire_post_write_cov_notifications(
                    db,
                    network,
                    cov_table,
                    cov_in_flight,
                    notification_transactions,
                    comm_state,
                    config,
                    &coarse_cov_oids,
                    &life_safety_cov_changes,
                )
                .await;
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

        confirmed_response::send_unsegmented_response(
            network,
            &response,
            source_mac,
            source_network.as_ref(),
            reply_tx,
        )
        .await;

        if let Some(accepted) = accepted_acknowledgment {
            Self::send_acknowledgment_notification_with_bindings(
                db,
                network,
                comm_state,
                server_tsm,
                notification_transactions,
                device_bindings,
                accepted,
                config.cov_retry_timeout_ms,
            )
            .await;
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

        Self::fire_post_write_cov_notifications(
            db,
            network,
            cov_table,
            cov_in_flight,
            notification_transactions,
            comm_state,
            config,
            &coarse_cov_oids,
            &life_safety_cov_changes,
        )
        .await;

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
}
