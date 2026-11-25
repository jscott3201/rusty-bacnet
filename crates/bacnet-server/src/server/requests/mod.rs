use super::event_send::settled as settled_lanes;
use super::*;

#[cfg(test)]
mod access_rights_durable_tests;
#[cfg(test)]
mod access_rights_mutation_tests;
mod acknowledge_alarm;
mod alarm_summary;
mod atomic_read_file;
mod atomic_write_file;
mod audit_log_query;
mod audit_notification;
pub(super) use audit_notification::ask_policy;
#[cfg(test)]
mod confirmed;
pub(super) mod confirmed_response;
mod dcc;
mod enrollment_summary;
mod event_information;
mod event_notification;
mod issue;
mod life_safety_operation;
mod mutations;
pub(super) use mutations::ask_mutation_policy;
use mutations::{InitialCovNotification, MutationEffects};
#[cfg(test)]
mod audit_log_buffer_wire_tests;
#[cfg(test)]
mod audit_log_description_wire_tests;
#[cfg(test)]
mod clock_zone_kept_wire_tests;
#[cfg(test)]
mod clock_zone_wire_tests;
#[cfg(test)]
mod color_command_wire_tests;
#[cfg(test)]
mod description_durable_tests;
#[cfg(test)]
mod description_kept_wire_tests;
#[cfg(test)]
mod description_tags_colour_wire_tests;
#[cfg(test)]
mod durable_stop_tests;
#[cfg(test)]
mod durable_write_wire_tests;
#[cfg(test)]
mod enrollment_parameters_durable_tests;
#[cfg(test)]
mod executed;
#[cfg(test)]
mod lighting_command_wire_tests;
#[cfg(test)]
mod list_parameters_durable_tests;
#[cfg(test)]
mod load_control_durable_tests;
#[cfg(test)]
mod log_reporting_durable_tests;
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
mod mutation_wpm_unlocked_tests;
#[cfg(test)]
mod notification_class_durable_tests;
#[cfg(test)]
mod parameters_durable_tests;
#[cfg(test)]
mod policy_inline_tests;
#[cfg(test)]
mod policy_off_task_tests;
#[cfg(test)]
mod reporting_durable_tests;
#[cfg(test)]
mod reporting_durable_types_tests;
#[cfg(test)]
mod state_count_durable_tests;

#[cfg(test)]
mod config_file_revision_tests;
#[cfg(test)]
mod create_config_live_tests;
#[cfg(test)]
mod create_config_types_wire_tests;
#[cfg(test)]
mod create_object_registered_wire_tests;
#[cfg(test)]
mod create_object_types_wire_tests;
mod other_services;
pub(super) mod read_property;
mod read_range;
#[cfg(test)]
mod recipient_mac_bound_tests;
#[cfg(test)]
mod structured_error_wire_tests;
#[cfg(test)]
mod subscribed_recipients_wire_tests;
#[cfg(test)]
mod tags_durable_tests;
mod unconfirmed;
#[cfg(test)]
mod unconfirmed_tests;
#[cfg(test)]
mod write_config_durable_tests;
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
    /// ReadProperty runs on its own path (see `read_property.rs`).
    pub(in crate::server) async fn handle_admitted_confirmed_request(
        services: &Arc<RequestServices<T>>,
        request_tasks: &super::request_tasks::RequestTaskSpawner,
        origin: RequestOrigin<'_>,
        req: bacnet_encoding::apdu::ConfirmedRequest,
        reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
        ownership: Option<ConfirmedRequestOwnership>,
    ) {
        if req.service_choice == ConfirmedServiceChoice::READ_PROPERTY {
            let admitted = read_property::AdmittedRequest {
                services: Arc::clone(services),
                request_tasks: request_tasks.clone(),
                source_mac: MacAddr::from_slice(origin.mac),
                source_network: origin.network,
                route: origin.route,
                req,
                reply_tx,
                ownership,
            };
            return Self::handle_admitted_read_property(admitted).await;
        }
        let RequestServices {
            db,
            network,
            cov_table,
            seg_ack_senders: _,
            seg_send_permits: _,
            cov_in_flight: _,
            learned_routers: _,
            notification_transactions,
            device_bindings,
            comm_state: _,
            dcc_timer: _,
            dcc_outcomes: _,
            event_suppressions: _,
            confirmed_event_repeats: _,
            received_event_log: _,
            mutation_decisions,
            warnings: _,
            config,
            execution: _,
            client_copies: _,
        } = &**services;
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
        let client_max_segments = req.max_segments;
        let (effective_max_apdu, segmented_response_available) =
            issue::response_limits(config, &route, &req, source_network.as_ref());
        let mut effects = MutationEffects::default();
        let mut initial_cov_notifications: Vec<InitialCovNotification> = Vec::new();
        let mut accepted_acknowledgment = None;
        let mut received_event = None;
        let mut after_reply = reinitialize::AfterReply::default();

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
            provenance: provenance.clone(),
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
        // WriteProperty, ReadPropertyMultiple and WritePropertyMultiple keep
        // their arms in this body, and every other service runs from
        // `other_services` (#2558). Run from there too, a WriteProperty took
        // 232 more instructions per request and a ReadPropertyMultiple about
        // 46: whole-binary totals from a local perf_counts.py run (wp and
        // rpm, current-thread runtime, 2026-11-25) against this layout, with
        // per-process hash keys. The structural part was this handler's and
        // the dispatcher's own count rising (wp +192, rpm +143); the
        // ReadPropertyMultiple total is within what code generation alone
        // moves between builds. WritePropertyMultiple, which no workload
        // counts, stays beside WriteProperty.
        let response = match service_choice {
            s if s == ConfirmedServiceChoice::WRITE_PROPERTY => {
                mutation
                    .write_property::<T>(services, &mut effects, &mut audit)
                    .await
            }
            s if s == ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE => {
                let result = confirmed_response::read_property_multiple_observed(
                    db,
                    services.live_tables(),
                    &req.service_request,
                    &mut ack_buf,
                    config,
                    services.execution,
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
                    Ok(()) => complex_ack(invoke_id, service_choice, ack_buf),
                    Err(bacnet_device_core::read::ReadFailure::Service(e)) => {
                        bacnet_device_core::reply::error_apdu(invoke_id, service_choice, &e)
                    }
                    Err(failure) => Apdu::Abort(AbortPdu {
                        sent_by_server: true,
                        invoke_id,
                        abort_reason: match failure {
                            bacnet_device_core::read::ReadFailure::Bytes => {
                                AbortReason::BUFFER_OVERFLOW
                            }
                            // Work, and any other limit the core reports.
                            _ => AbortReason::OUT_OF_RESOURCES,
                        },
                    }),
                }
            }
            s if s == ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE => {
                mutation
                    .write_property_multiple::<T>(services, &mut effects, &mut audit)
                    .await
            }
            // Every other service.
            _ => {
                let call = other_services::ServiceCall {
                    services,
                    request_tasks,
                    req: &req,
                    source_mac,
                    source_network: &source_network,
                    local_network,
                    invoke_id,
                    service_choice,
                    effective_max_apdu,
                    segmented_response_available,
                    mutation: &mutation,
                };
                let out = other_services::ServiceEffects {
                    effects: &mut effects,
                    initial_cov_notifications: &mut initial_cov_notifications,
                    accepted_acknowledgment: &mut accepted_acknowledgment,
                    received_event: &mut received_event,
                    after_reply: &mut after_reply,
                    read_audits: &mut read_audits,
                    audit: &mut audit,
                };
                match Self::other_service_response(call, out, provenance, ack_buf).await {
                    Some(response) => response,
                    None => return,
                }
            }
        };

        let MutationEffects {
            mut settle,
            mut coarse_cov_oids,
            mut life_safety_cov_changes,
            staging_plans,
            mut command_runs,
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
        if service_choice == ConfirmedServiceChoice::LIFE_SAFETY_OPERATION
            && let Some(pending) = lso_pending
        {
            let mut encoded = BytesMut::new();
            encode_apdu(&mut encoded, &response).expect("valid APDU encoding");
            pending.complete_with_response(encoded.freeze());
        }
        // Non-LSO callers pass `None` (or an untracked guard whose completion
        // is a no-op); dropping here is intentional.

        // Boxed: plans are rare, and their future would size every request's.
        // The targets' notifications join this request's settle handle and
        // their objects its COV fanout, made after the response (#2537).
        // The handle goes into the boxed future and back, so it isn't held
        // across that await in this future's state.
        if !staging_plans.is_empty() {
            let cov = (&mut coarse_cov_oids, &mut life_safety_cov_changes);
            let plans = Self::execute_request_staging_plans(services, staging_plans, settle, cov);
            let (runs, returned) = Box::pin(plans).await;
            settle = returned;
            command_runs.extend(runs);
        }
        // The event notifications the writes' transitions, and their Staging
        // plans' targets', pushed onto their recipients' lanes under the
        // writes' guards (#2537): the lanes send them in commit order, with
        // those of the runs this request starts and of every other writer,
        // and the response never waits on them. The COV fanout waits, capped
        // (#889).
        let settled = settle.into_wait();
        // Command lists run beside this request, so its response never waits
        // on their writes or post delays (#1150).
        if !command_runs.is_empty() {
            super::command_runs::CommandRunner::new(services, request_tasks).start(command_runs);
        }

        let plan = issue::ResponsePlan {
            services,
            request_tasks,
            target: ResponseTarget {
                source_mac,
                source_network: source_network.as_ref(),
                route: &route,
            },
            ack: ComplexAckParams {
                invoke_id,
                service_choice,
                client_max_apdu: effective_max_apdu,
                client_max_segments,
            },
            segmented_response_available,
        };
        let issued =
            Self::issue_confirmed_response(plan, &response, &audit, read_audits, reply_tx, pending)
                .await;
        // A ReinitializeDevice restart waits for its SimpleACK's handoff
        // (Clause 16.4.2, #1565); no lock is held here.
        match issued {
            issue::Issued::Unsegmented { handed_off: true } => after_reply.run(),
            _ => drop(after_reply),
        }
        // Made here, so the response's wait doesn't hold it.
        let cov_ctx = services.cov_context();
        if issued == issue::Issued::Segmented {
            if settled.is_some() {
                let timeout_ms = config.cov_retry_timeout_ms;
                Box::pin(settled_lanes(
                    settled,
                    timeout_ms,
                    notification_transactions,
                ))
                .await;
            }
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
                        Self::fire_initial_cov_notification_multiple(&cov_ctx, subscriptions).await;
                    }
                }
            }
            return;
        }

        if let Some(accepted) = accepted_acknowledgment {
            Self::send_acknowledgment_notification_with_bindings(
                &services.event_delivery(),
                accepted,
            )
            .await;
        }
        if let Some(notification) = received_event {
            Self::log_and_forward_received_event(
                services,
                source_mac,
                &source_network,
                local_network,
                &req,
                notification,
            )
            .await;
        }

        // Boxed, and only for a write that pushed something: the capped wait
        // would size every request's future.
        if settled.is_some() {
            let timeout_ms = config.cov_retry_timeout_ms;
            Box::pin(settled_lanes(
                settled,
                timeout_ms,
                notification_transactions,
            ))
            .await;
        }
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

/// The unsegmented ComplexAck answering `invoke_id`'s `service_choice` with
/// `ack_buf`.
fn complex_ack(invoke_id: u8, service_choice: ConfirmedServiceChoice, ack_buf: BytesMut) -> Apdu {
    Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice,
        service_ack: ack_buf.freeze(),
    })
}

/// The SimpleAck answering `invoke_id`'s `service_choice`.
fn simple_ack(invoke_id: u8, service_choice: ConfirmedServiceChoice) -> Apdu {
    Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice,
    })
}
