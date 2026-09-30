use super::cov_notify_context::CovNotifyContext;
use super::event_forwarding::{ForwardOrigin, Reception};
use super::received_event_log::log_received_event_notification;
use super::*;
use bacnet_endpoint_core::coordinator::CanonicalPeer;
use bacnet_services::alarm_event::ForwardedEventNotification;

#[cfg(test)]
mod access_rights_durable_tests;
#[cfg(test)]
mod access_rights_mutation_tests;
mod acknowledge_alarm;
mod alarm_summary;
mod atomic_read_file;
mod atomic_write_file;
mod audit_notification;
pub(super) use audit_notification::fail_closed_authorize;
#[cfg(test)]
mod confirmed;
pub(super) mod confirmed_response;
mod dcc;
#[doc(hidden)]
pub mod endpoint_responder;
#[cfg(test)]
mod endpoint_shared_runtime_tests;
mod enrollment_summary;
mod event_information;
mod mutations;
use mutations::{InitialCovNotification, MutationEffects};
#[cfg(test)]
mod audit_log_buffer_wire_tests;
#[cfg(test)]
mod color_command_wire_tests;
#[cfg(test)]
mod durable_stop_tests;
#[cfg(test)]
mod durable_write_wire_tests;
#[cfg(test)]
mod executed;
#[cfg(test)]
mod lighting_command_wire_tests;
#[cfg(test)]
mod mutation_boundary_tests;
#[cfg(test)]
mod mutation_entry_tests;
#[cfg(test)]
mod mutation_list_element_number_tests;
#[cfg(test)]
mod mutation_list_wire_tests;
#[cfg(test)]
mod mutation_provenance_tests;
#[cfg(test)]
mod mutation_tests;
#[cfg(test)]
mod mutation_wpm_priority_tests;
#[cfg(test)]
mod mutation_wpm_tests;
#[cfg(test)]
mod notification_class_durable_tests;
mod read_range;
#[cfg(test)]
mod recipient_mac_bound_tests;
#[cfg(test)]
mod structured_error_wire_tests;
#[cfg(test)]
mod subscribed_recipients_wire_tests;
mod unconfirmed;
#[cfg(test)]
mod unconfirmed_tests;
#[cfg(test)]
pub(crate) use self::{executed::EXECUTED_CONFIRMED, unconfirmed::EXECUTED_UNCONFIRMED};

/// Distinct lifetimes: ordinary transaction ownership versus LSO's local replay.
pub(super) enum ConfirmedRequestOwnership {
    Generic(PendingConfirmedRequest),
    LifeSafety(PendingLsoReplay),
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Execute admitted work with its single response owner. Direct handler
    /// tests may omit ownership; production dispatch always supplies it.
    pub(in crate::server) async fn handle_admitted_confirmed_request(
        services: &RequestServices<T>,
        request_tasks: &super::request_tasks::RequestTaskSpawner,
        origin: RequestOrigin<'_>,
        req: bacnet_encoding::apdu::ConfirmedRequest,
        reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
        ownership: Option<ConfirmedRequestOwnership>,
    ) {
        let RequestServices {
            db,
            network,
            cov_table,
            seg_ack_senders,
            seg_send_permits,
            cov_in_flight,
            learned_routers: _,
            notification_transactions,
            device_bindings,
            comm_state,
            dcc_timer: _,
            dcc_outcomes: _,
            event_suppressions,
            confirmed_event_repeats,
            received_event_log,
            mutation_decisions,
            config,
        } = services;
        let RequestOrigin {
            mac: source_mac,
            network: source_network,
            route,
        } = origin;
        let provenance = route.provenance();
        let (pending, lso_pending) = match ownership {
            Some(ConfirmedRequestOwnership::Generic(pending)) => (Some(pending), None),
            Some(ConfirmedRequestOwnership::LifeSafety(pending)) => (None, Some(pending)),
            None => (None, None),
        };
        let invoke_id = req.invoke_id;
        let service_choice = req.service_choice;
        let client_max_apdu = req.max_apdu_length;
        let client_accepts_segmented = req.segmented_response_accepted;
        let client_max_segments = req.max_segments;
        let effective_max_apdu = event_information::limit(client_max_apdu, config.max_apdu_length);
        let effective_max_apdu = route
            .max_apdu_length(effective_max_apdu, source_network.as_ref())
            // Invalid reply authority must not revoke already-admitted service
            // execution. Keep its construction budget; issuance still fails
            // closed on the same invalid route without any fallback.
            .unwrap_or(effective_max_apdu);
        let device_transmits_segments =
            event_information::can_segment(config.segmentation_supported);
        let segmented_response_available = client_accepts_segmented && device_transmits_segments;
        let mut effects = MutationEffects::default();
        let mut initial_cov_notifications: Vec<InitialCovNotification> = Vec::new();
        let mut accepted_acknowledgment = None;
        let mut received_event = None;

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
        // One reading of this network's number for the command origin and the
        // Audit source, so both tie the request to the same Device (#1404).
        let local_network = network.local_network_number().get();
        // Snapshot actual original sender and correlation independently of Audit;
        // the binding guard is gone before any object mutation takes place.
        let command_origin = device_bindings::snapshot_command_origin(
            service_choice,
            source_mac,
            source_network.as_ref(),
            local_network,
            device_bindings,
            notification_transactions,
        )
        .await;
        let mutation = mutations::Request {
            config,
            decisions: mutation_decisions,
            source_mac,
            source_network: source_network.as_ref(),
            provenance,
            req: &req,
            command_origin: command_origin.as_ref(),
        };
        let mut audit = super::audit_reporter::WriteAudit::new(
            config,
            network,
            notification_transactions,
            device_bindings,
            super::audit_reporter::RequestSource {
                mac: source_mac,
                network: source_network.as_ref(),
                invoke_id,
                local_network,
            },
        )
        .await;
        let mut read_audits = Vec::new();
        let response = match service_choice {
            s if s == ConfirmedServiceChoice::READ_PROPERTY => {
                confirmed_response::read_property_response_observed(
                    db,
                    Some(services.live_tables()),
                    crate::device_view::DeviceExecution::FullServer,
                    config.registered_network_port,
                    config.read_property_multiple_budget.max_result_elements,
                    &req,
                    |db, oid, req, result| {
                        let result = match result {
                            Ok(()) => None,
                            Err(Error::Timeout(_) | Error::Reject { .. } | Error::Abort { .. }) => {
                                return
                            }
                            Err(error) => Some(confirmed_response::error_fields(error)),
                        };
                        read_audits.extend(audit.read_intent(
                            db,
                            oid,
                            req.property_identifier,
                            req.property_array_index,
                            result,
                        ));
                    },
                )
                .await
            }
            s if s == ConfirmedServiceChoice::WRITE_PROPERTY => {
                mutation
                    .write_property::<T>(db, cov_table, &mut effects, &mut audit)
                    .await
            }
            s if s == ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE => {
                let result = confirmed_response::read_property_multiple_observed(
                    db,
                    services.live_tables(),
                    &req.service_request,
                    &mut ack_buf,
                    config.read_property_multiple_budget,
                    config.registered_network_port,
                    |db, oid, property, index, result| {
                        read_audits.extend(audit.read_intent(db, oid, property, index, result));
                    },
                )
                .await;
                if result.is_err() {
                    // No audited prefix for decode/work/response-buffer failure.
                    read_audits.clear();
                }
                match result {
                    Ok(()) => complex_ack(ack_buf),
                    Err(handlers::ReadFailure::Service(e)) => {
                        Self::error_apdu_from_error(invoke_id, service_choice, &e)
                    }
                    Err(failure) => Apdu::Abort(AbortPdu {
                        sent_by_server: true,
                        invoke_id,
                        abort_reason: match failure {
                            handlers::ReadFailure::Work => AbortReason::OUT_OF_RESOURCES,
                            handlers::ReadFailure::Bytes => AbortReason::BUFFER_OVERFLOW,
                            handlers::ReadFailure::Service(_) => unreachable!(),
                        },
                    }),
                }
            }
            s if s == ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE => {
                mutation
                    .write_property_multiple::<T>(db, cov_table, &mut effects, &mut audit)
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
                mutation
                    .create_object::<T>(db, cov_table, &mut effects, ack_buf, &mut audit)
                    .await
            }
            s if s == ConfirmedServiceChoice::DELETE_OBJECT => {
                mutation
                    .delete_object::<T>(db, cov_table, &mut effects, &mut audit)
                    .await
            }
            s if s == ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL => {
                dcc::response(
                    services,
                    &req,
                    source_mac,
                    source_network.as_ref(),
                    local_network,
                    request_tasks,
                    &mut audit,
                )
                .await
            }
            s if s == ConfirmedServiceChoice::REINITIALIZE_DEVICE => {
                confirmed_response::reinitialize_response(
                    db,
                    &req,
                    &config.reinit_password,
                    config.on_reinitialize.as_ref(),
                )
                .await
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
                    services.live_tables(),
                    &req,
                    config,
                    effective_max_apdu,
                    segmented_response_available,
                    |db, target, property, index, result| {
                        read_audits.extend(audit.completed_read_intent(
                            db,
                            target,
                            Some((property, index)),
                            result,
                        ));
                    },
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
                    |target, result| {
                        read_audits.extend(audit.completed_read_intent(&db, target, None, result));
                    },
                )
            }
            s if s == ConfirmedServiceChoice::ATOMIC_WRITE_FILE => {
                mutation.atomic_write_file::<T>(db, &mut audit).await
            }
            s if s == ConfirmedServiceChoice::ADD_LIST_ELEMENT
                || s == ConfirmedServiceChoice::REMOVE_LIST_ELEMENT =>
            {
                let remove = s == ConfirmedServiceChoice::REMOVE_LIST_ELEMENT;
                mutation
                    .list_element::<T>(db, cov_table, &mut effects, &mut audit, remove)
                    .await
            }
            s if s == ConfirmedServiceChoice::GET_ALARM_SUMMARY => {
                let db = db.read().await;
                Self::alarm_summary_response(
                    &db,
                    invoke_id,
                    &req.service_request,
                    config.get_alarm_summary_budget,
                )
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
                    handlers::handle_audit_log_query_observed(
                        &db,
                        &req.service_request,
                        |target, result| {
                            read_audits
                                .extend(audit.completed_read_intent(&db, target, None, result));
                        },
                    )
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
                            Err(e) => {
                                // Execution alone is not a completed query response.
                                read_audits.clear();
                                Self::error_apdu_from_error(invoke_id, service_choice, &e)
                            }
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
                    provenance,
                    &req,
                )
                .await
                {
                    Ok((audit_notification::Stored, forward)) => {
                        if let Some(forward) = forward {
                            forward.start(
                                network,
                                notification_transactions,
                                device_bindings,
                                config.max_apdu_length,
                            );
                        }
                        simple_ack()
                    }
                    Ok((audit_notification::Duplicate, _)) => return,
                    Err(error) => Self::error_apdu_from_error(invoke_id, service_choice, &error),
                }
            }
            // The sender's acknowledgment depends only on the request being
            // well formed; forwarding runs after the response and its outcome
            // never reaches the sender (Clause 12.51). A retransmission of a
            // notification already received is answered again but not
            // offered to the forwarders again (#1259).
            s if s == ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION => {
                match ForwardedEventNotification::decode(&req.service_request) {
                    Ok(notification) => {
                        if confirmed_event_repeats.first_receipt(
                            super::request_peer::canonical_requester(
                                source_mac,
                                source_network.as_ref(),
                            ),
                            invoke_id,
                            &req.service_request,
                            Instant::now,
                        ) {
                            received_event = Some(notification);
                        } else {
                            debug!(
                                invoke_id,
                                "Retransmitted ConfirmedEventNotification answered, not forwarded again"
                            );
                        }
                        simple_ack()
                    }
                    // A syntax fault draws the Reject naming it, as the
                    // client's does (#1446).
                    Err(error) => Self::error_apdu_from_error(
                        invoke_id,
                        service_choice,
                        &error.into_request_reject(),
                    ),
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
                )
                .map_err(Error::into_request_reject);
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
                                    provenance,
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
                                    let result =
                                        handlers::handle_life_safety_operation(&mut db, &request);
                                    // Timestamped references capture the
                                    // exact changes under this guard (#856).
                                    if let Ok(changes) = &result {
                                        let capture =
                                            cov_table.read().await.timed_capture_exact(changes);
                                        capture.run(&db);
                                    }
                                    result
                                }
                            }
                        };

                        match execution {
                            Ok(result) => {
                                effects.life_safety_cov_changes.extend(result);
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

        let MutationEffects {
            written_oids,
            coarse_cov_oids,
            life_safety_cov_changes,
            staging_plans,
            command_runs,
            timed_revisits,
        } = effects;

        // LSO-only replay store (server level, never handler/object level).
        // Uniform rule: anything that reaches this admission point and produces
        // an LSO response — success SimpleACK, execution errors, denial, and
        // the pre-authorization deterministic rejects (decode fail,
        // UNKNOWN_OBJECT precheck, VALUE_OUT_OF_RANGE) — is stored once
        // admitted. The replay is a local idempotency extension, not a
        // Standard mandate, and makes no physical-idempotency claim.
        // Lock → clone → unlock → send; never held across `.await`.
        if service_choice == ConfirmedServiceChoice::LIFE_SAFETY_OPERATION {
            if let Some(pending) = lso_pending {
                let mut encoded = BytesMut::new();
                encode_apdu(&mut encoded, &response).expect("valid APDU encoding");
                pending.complete_with_response(encoded.freeze());
            }
        }
        // Non-LSO callers pass `None` (or an untracked guard whose completion
        // is a no-op); dropping here is intentional.

        let cov_ctx = CovNotifyContext {
            db,
            network,
            cov_table,
            cov_in_flight,
            notification_transactions,
            comm_state,
            config,
        };
        Self::execute_staging_plans(&services.event_delivery(), &cov_ctx, staging_plans).await;
        // Command lists run beside this request, so its response never waits
        // on their writes or post delays (#1150).
        if !command_runs.is_empty() {
            super::command_runs::CommandRunner::new(services, request_tasks).start(command_runs);
        }

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
                    if let Err(e) = Self::issue_terminal_response(
                        network,
                        &buf,
                        source_mac,
                        source_network.as_ref(),
                        &route,
                        pending,
                    )
                    .await
                    {
                        warn!(error = %e, "Failed to send Abort for segmentation-not-supported");
                    }
                } else {
                    Self::spawn_segmented_complex_ack(
                        SegmentedSendResources {
                            network,
                            seg_ack_senders,
                            seg_send_permits,
                        },
                        request_tasks,
                        ResponseTarget {
                            source_mac,
                            source_network: source_network.as_ref(),
                            route: &route,
                        },
                        ComplexAckParams {
                            invoke_id,
                            service_choice,
                            client_max_apdu: effective_max_apdu,
                            client_max_segments,
                        },
                        ack.service_ack.clone(),
                        pending,
                    );
                }

                Self::fire_written_event_notifications(
                    &services.event_delivery(),
                    cov_table,
                    &written_oids,
                )
                .await;
                Self::fire_post_write_cov_notifications(
                    &cov_ctx,
                    &coarse_cov_oids,
                    &life_safety_cov_changes,
                )
                .await;
                for notification in &initial_cov_notifications {
                    match notification {
                        InitialCovNotification::Single(subscription) => {
                            Self::fire_initial_cov_notification(&cov_ctx, subscription).await;
                        }
                        InitialCovNotification::Multiple(subscriptions) => {
                            Self::fire_initial_cov_notification_multiple(&cov_ctx, subscriptions)
                                .await;
                        }
                    }
                }
                return;
            }
        }

        // Read execution is observed under the DB read guard, but admission is
        // deferred until the complete response exists and that guard is gone.
        // Segmentation/post-execution transport divergence does not add records.
        audit.admit_reads(db, read_audits).await;
        confirmed_response::send_unsegmented_response(
            network,
            &response,
            source_mac,
            source_network.as_ref(),
            &route,
            reply_tx,
            pending,
        )
        .await;

        if let Some(accepted) = accepted_acknowledgment {
            Self::send_acknowledgment_notification_with_bindings(
                &services.event_delivery(),
                accepted,
            )
            .await;
        }
        if let Some(notification) = received_event {
            // Logged at its first receipt only, with no lock held (#1346).
            let source =
                CanonicalPeer::from_source(source_mac, source_network.as_ref(), local_network);
            let (log, suppressions) = (received_event_log, event_suppressions);
            log_received_event_notification(db, log, suppressions, source, &req.service_request)
                .await;
            // The dispatch loop drops a confirmed request that arrives by
            // broadcast (Clause 5.4.5.1), so this one was addressed to this
            // device alone.
            Self::forward_event_notification(
                &services.event_delivery(),
                vec![notification],
                ForwardOrigin::Received(Reception::UNICAST),
            )
            .await;
        }

        Self::fire_written_event_notifications(
            &services.event_delivery(),
            cov_table,
            &written_oids,
        )
        .await;

        Self::fire_post_write_cov_notifications(
            &cov_ctx,
            &coarse_cov_oids,
            &life_safety_cov_changes,
        )
        .await;
        // WritePropertyMultiple (SimpleACK or Error, never a ComplexACK) may
        // have captured changes that fanout did not select (#856).
        if !timed_revisits.is_empty() {
            cov_table.read().await.revisits().request(timed_revisits);
        }

        for notification in &initial_cov_notifications {
            match notification {
                InitialCovNotification::Single(subscription) => {
                    Self::fire_initial_cov_notification(&cov_ctx, subscription).await;
                }
                InitialCovNotification::Multiple(subscriptions) => {
                    Self::fire_initial_cov_notification_multiple(&cov_ctx, subscriptions).await;
                }
            }
        }
    }
}
