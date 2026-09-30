use super::*;
use crate::cov::active::{LiveCovSelection, LiveDeviceCov};
use crate::device_view::{DeviceExecution, DeviceReadContext};
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_types::error::ErrorDetail;

/// ReadProperty under the narrow responder's actual RP[/WP][/ReinitializeDevice] execution
/// profile, within the responder's configured read work limit.
pub(super) async fn read_property_response(
    db: &RwLock<ObjectDatabase>,
    request: &ConfirmedRequestPdu,
    writes: bool,
    reinitialize: bool,
    registered_port: Option<ObjectIdentifier>,
    work_limit: usize,
) -> Apdu {
    read_property_response_observed(
        db,
        None,
        DeviceExecution::Endpoint {
            writes,
            reinitialize,
        },
        registered_port,
        work_limit,
        request,
        |_, _, _, _| {},
    )
    .await
}

/// The server tables a read samples the selected Device's server-owned lists
/// from: the COV table for its COV lists, the binding table for its
/// Device_Address_Binding (#1369).
#[derive(Clone, Copy)]
pub(in crate::server) struct LiveTables<'a> {
    pub(in crate::server) cov: &'a RwLock<CovSubscriptionTable>,
    pub(in crate::server) bindings: &'a RwLock<DeviceBindingTable>,
}

/// Request-local live Device `Active_COV_Subscriptions`,
/// `Active_COV_Multiple_Subscriptions` and `Device_Address_Binding` (Clause
/// 12.11), as selected.
///
/// Lock order is database, then COV table, then binding table: the caller
/// holds the database read guard; each table's read guard is held only to
/// copy what is live at one instant, and is released before the next is
/// taken and before any object read or encoding. A request that selects
/// neither COV list doesn't touch the COV table.
pub(in crate::server) async fn active_cov_snapshot(
    db: &ObjectDatabase,
    tables: LiveTables<'_>,
    selection: LiveCovSelection,
) -> LiveDeviceCov {
    let live = if selection.reads_cov() {
        let entries = {
            let table = tables.cov.read().await;
            table.live_cov_entries(selection, Instant::now())
        };
        LiveDeviceCov::project(db, selection, entries)
    } else {
        LiveDeviceCov::default()
    };
    let bindings = address_bindings(tables.bindings, selection).await;
    live.with_address_bindings(selection, bindings)
}

/// The selected Device's Device_Address_Binding, when `selection` reads it:
/// the bindings `table` holds now, copied under its read guard (#1369).
pub(in crate::server) async fn address_bindings(
    table: &RwLock<DeviceBindingTable>,
    selection: LiveCovSelection,
) -> Option<PropertyValue> {
    if !selection.address_bindings {
        return None;
    }
    let table = table.read().await;
    Some(table.address_binding_list(Instant::now()))
}

/// Budgeted ReadPropertyMultiple under one database read guard. The request
/// is planned first; when the plan reads one of the Device's server-owned
/// lists, one request-local projection serves every such row, a Group's
/// member rows included (#1171, #1213). A request past its work limit fails
/// in planning, before any table is sampled.
pub(super) async fn read_property_multiple_observed(
    db: &RwLock<ObjectDatabase>,
    tables: LiveTables<'_>,
    service_request: &[u8],
    service_ack: &mut BytesMut,
    budget: crate::server::ReadPropertyMultipleBudget,
    registered_port: Option<ObjectIdentifier>,
    mut completed: impl FnMut(
        &ObjectDatabase,
        ObjectIdentifier,
        PropertyIdentifier,
        Option<u32>,
        Option<(ErrorClass, ErrorCode)>,
    ),
) -> Result<(), handlers::ReadFailure> {
    let db = db.read().await;
    let request = bacnet_services::rpm::ReadPropertyMultipleRequest::decode(service_request)
        .map_err(|error| handlers::ReadFailure::Service(error.into_request_reject()))?;
    let view = DeviceReadContext::new(&db, DeviceExecution::FullServer)
        .with_registered_port(registered_port);
    let plan = handlers::RpmPlan::new(&db, &request, budget.max_result_elements, Some(&view))?;
    let live = match plan.live_cov(&db) {
        Some(selection) => Some(active_cov_snapshot(&db, tables, selection).await),
        None => None,
    };
    let view = view.with_live(live.as_ref());
    plan.read_observed(
        &db,
        Some(&view),
        service_ack,
        budget.max_service_ack_bytes,
        |oid, property, index, result| completed(&db, oid, property, index, result),
    )
}

/// ReadProperty under one database read guard, planned before any table is
/// sampled (#1213). A Group's Present_Value counts against `work_limit` as
/// a ReadPropertyMultiple naming only it would, and a read past it is aborted
/// as that request would be.
pub(super) async fn read_property_response_observed(
    db: &RwLock<ObjectDatabase>,
    tables: Option<LiveTables<'_>>,
    execution: DeviceExecution,
    registered_port: Option<ObjectIdentifier>,
    work_limit: usize,
    request: &ConfirmedRequestPdu,
    mut completed: impl FnMut(
        &ObjectDatabase,
        ObjectIdentifier,
        &ReadPropertyRequest,
        &Result<(), Error>,
    ),
) -> Apdu {
    let mut service_ack = BytesMut::with_capacity(512);
    let db = db.read().await;
    let result = match ReadPropertyRequest::decode(&request.service_request)
        .map_err(Error::into_request_reject)
    {
        Ok(decoded) => {
            let lookup_oid =
                handlers::resolve_read_target(&db, &decoded.object_identifier, registered_port);
            let view = DeviceReadContext::new(&db, execution)
                .with_registered_port(registered_port)
                .with_work_limit(work_limit);
            match handlers::plan_read_property(
                &db,
                Some(&view),
                lookup_oid,
                decoded.property_identifier,
                decoded.property_array_index,
            ) {
                Ok(plan) => {
                    let live = match (tables, plan.live_cov(&db)) {
                        (Some(tables), Some(selection)) => {
                            Some(active_cov_snapshot(&db, tables, selection).await)
                        }
                        _ => None,
                    };
                    let view = view.with_live(live.as_ref());
                    handlers::read_property_request_observed(
                        &db,
                        Some(&view),
                        &decoded,
                        plan,
                        &mut service_ack,
                        |oid, request, result| completed(&db, oid, request, result),
                    )
                    .map_err(handlers::ReadFailure::Service)
                }
                Err(failure) => Err(failure),
            }
        }
        Err(error) => Err(handlers::ReadFailure::Service(error)),
    };
    match result {
        Ok(()) => Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id: request.invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: request.service_choice,
            service_ack: service_ack.freeze(),
        }),
        Err(handlers::ReadFailure::Service(error)) => {
            error_apdu_from_error(request.invoke_id, request.service_choice, &error)
        }
        // A Group's Present_Value past the work limit draws the abort an RPM
        // over its work budget does; ReadProperty has no byte budget.
        Err(handlers::ReadFailure::Work | handlers::ReadFailure::Bytes) => Apdu::Abort(AbortPdu {
            sent_by_server: true,
            invoke_id: request.invoke_id,
            abort_reason: AbortReason::OUT_OF_RESOURCES,
        }),
    }
}

/// ReinitializeDevice: the password checked, then `handler` run with the database write-locked.
/// With no handler every request is refused with SERVICES / SERVICE_REQUEST_DENIED.
pub(super) async fn reinitialize_response(
    db: &RwLock<ObjectDatabase>,
    request: &ConfirmedRequestPdu,
    password: &Option<String>,
    handler: Option<&ReinitializeHandler>,
) -> Apdu {
    let outcome = match handlers::handle_reinitialize_device(&request.service_request, password) {
        Ok(state) => match handler {
            Some(handler) => handler(state, &mut *db.write().await),
            None => Err(Error::Protocol {
                class: ErrorClass::SERVICES.to_raw() as u32,
                code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
            }),
        },
        Err(error) => Err(error),
    };

    match outcome {
        Ok(()) => Apdu::SimpleAck(SimpleAck {
            invoke_id: request.invoke_id,
            service_choice: request.service_choice,
        }),
        Err(error) => error_apdu_from_error(request.invoke_id, request.service_choice, &error),
    }
}

/// The reply to a confirmed request `error` refused.
///
/// An [`Error::Reject`] draws a Reject PDU with its reason, for every
/// service, the formal-error ones included: a Reject has no
/// service-specific body. Each handler turns the error from decoding the
/// request into one with [`Error::into_request_reject`], so a syntax fault
/// in the request draws the Reject naming it (Clauses 18.9 and 20.1.8,
/// #1446). Any other refusal is an Error PDU, in the formal Clause 21
/// production for the services that have one; that includes a decoding
/// error met once the service is running, which is no syntax fault of the
/// request.
pub(super) fn error_apdu_from_error(
    invoke_id: u8,
    service_choice: ConfirmedServiceChoice,
    error: &Error,
) -> Apdu {
    if let Error::Reject { reason } = error {
        return Apdu::Reject(RejectPdu {
            invoke_id,
            reject_reason: RejectReason::from_raw(*reason),
        });
    }
    let (error_class, error_code) = error_fields(error);
    let detail = match error {
        Error::Structured { detail, .. } => Some(detail.as_ref()),
        _ => None,
    };
    // Only an element refusal names an element; a refusal of the request or
    // its target (authorization, object, property, index, list-ness, an
    // object the server cannot create) carries zero (Clauses 15.1.1.3.1,
    // 15.2.1.3.1 and 15.3.1.3).
    let first_failed_element_number = match detail {
        Some(ErrorDetail::FirstFailedElementNumber(number)) => *number,
        _ => 0,
    };
    // These services answer every error with their Clause 21 production.
    if service_choice == ConfirmedServiceChoice::ADD_LIST_ELEMENT
        || service_choice == ConfirmedServiceChoice::REMOVE_LIST_ELEMENT
    {
        return Apdu::Error(
            bacnet_services::list_manipulation::ChangeListError {
                error_class,
                error_code,
                first_failed_element_number,
            }
            .to_error_pdu(invoke_id, service_choice),
        );
    }
    if service_choice == ConfirmedServiceChoice::CREATE_OBJECT {
        return Apdu::Error(
            bacnet_services::object_mgmt::CreateObjectError {
                error_class,
                error_code,
                first_failed_element_number,
            }
            .to_error_pdu(invoke_id),
        );
    }
    if service_choice == ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE {
        // A refusal of one COV reference names it; any failure before the
        // references are processed is the general choice (Clause 13.16.2).
        let first_failed_subscription = match detail {
            Some(ErrorDetail::FirstFailedSubscription(reference)) => Some(reference.clone()),
            _ => None,
        };
        return Apdu::Error(
            bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleError {
                error_class,
                error_code,
                first_failed_subscription,
            }
            .to_error_pdu(invoke_id),
        );
    }
    Apdu::Error(ErrorPdu {
        invoke_id,
        service_choice,
        error_class,
        error_code,
        error_data: Bytes::new(),
    })
}

pub(in crate::server) fn error_fields(error: &Error) -> (ErrorClass, ErrorCode) {
    match error {
        Error::Protocol { class, code } | Error::Structured { class, code, .. } => (
            ErrorClass::from_raw(*class as u16),
            ErrorCode::from_raw(*code as u16),
        ),
        _ => (ErrorClass::SERVICES, ErrorCode::OTHER),
    }
}

/// Send one non-segmented confirmed-service response through its existing
/// transport or reply-channel path.
pub(in crate::server) async fn send_unsegmented_response<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    response: &Apdu,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    route: &bacnet_network::response_route::ResponseRoute,
    reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
    pending: Option<PendingConfirmedRequest>,
) {
    send_response(
        network,
        response,
        ResponseTarget {
            source_mac,
            source_network,
            route,
        },
        reply_tx,
        true,
        pending,
    )
    .await;
}

/// Overload work shares the identical wire path without per-rejection error
/// logging. Admission/fallback telemetry is bounded and does not imply send success.
pub(in crate::server) async fn send_overload_response<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    response: &Apdu,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    route: &bacnet_network::response_route::ResponseRoute,
    reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
) {
    send_response(
        network,
        response,
        ResponseTarget {
            source_mac,
            source_network,
            route,
        },
        reply_tx,
        false,
        None,
    )
    .await;
}

/// Resend already-encoded LSO response bytes without re-execution.
///
/// Replay path only: no mutation, no authorizer re-invocation, no COV/event
/// re-fire. Bytes are the exact APDU encoding stored at first-execution
/// response time (same invoke echo); the NPDU wrap / transport send mirrors
/// [`send_unsegmented_response`] so the wire image is byte-identical.
pub(in crate::server) async fn send_replay_bytes<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    apdu_bytes: &Bytes,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    route: &bacnet_network::response_route::ResponseRoute,
    reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
) {
    send_raw_response(
        network,
        apdu_bytes,
        source_mac,
        source_network,
        route,
        reply_tx,
    )
    .await;
}

async fn send_raw_response<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    apdu_bytes: &Bytes,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    route: &bacnet_network::response_route::ResponseRoute,
    reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
) {
    let reply_tx = if route.provenance().is_direct_peer() {
        None
    } else {
        reply_tx
    };
    if let Some(tx) = reply_tx {
        use bacnet_encoding::npdu::{encode_npdu, Npdu};
        let npdu = Npdu {
            is_network_message: false,
            expecting_reply: false,
            priority: NetworkPriority::NORMAL,
            destination: source_network.cloned(),
            source: None,
            payload: apdu_bytes.clone(),
            ..Npdu::default()
        };
        let mut npdu_buf = BytesMut::with_capacity(2 + apdu_bytes.len());
        match encode_npdu(&mut npdu_buf, &npdu) {
            Ok(()) => {
                let _ = tx.send(npdu_buf.freeze());
            }
            Err(error) => {
                warn!(%error, "Failed to encode NPDU for MS/TP replay");
                if let Err(error) = BACnetServer::<T>::send_confirmed_response_apdu(
                    network,
                    apdu_bytes,
                    source_mac,
                    source_network,
                    route,
                )
                .await
                {
                    warn!(%error, "Failed to send replay");
                }
            }
        }
    } else if let Err(error) = BACnetServer::<T>::send_confirmed_response_apdu(
        network,
        apdu_bytes,
        source_mac,
        source_network,
        route,
    )
    .await
    {
        warn!(%error, "Failed to send replay");
    }
}

/// Where a confirmed-service response goes: the requester's MAC, its network
/// address when routed, and the route the request arrived on.
#[derive(Clone, Copy)]
pub(in crate::server) struct ResponseTarget<'a> {
    pub(in crate::server) source_mac: &'a [u8],
    pub(in crate::server) source_network: Option<&'a NpduAddress>,
    pub(in crate::server) route: &'a bacnet_network::response_route::ResponseRoute,
}

async fn send_response<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    response: &Apdu,
    target: ResponseTarget<'_>,
    reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
    log_errors: bool,
    pending: Option<PendingConfirmedRequest>,
) {
    let ResponseTarget {
        source_mac,
        source_network,
        route,
    } = target;
    let mut buf = BytesMut::new();
    encode_apdu(&mut buf, response).expect("valid APDU encoding");

    let reply_tx = if route.provenance().is_direct_peer() {
        None
    } else {
        reply_tx
    };
    if let Some(tx) = reply_tx {
        use bacnet_encoding::npdu::{encode_npdu, Npdu};
        let apdu_bytes = buf.freeze();
        let npdu = Npdu {
            is_network_message: false,
            expecting_reply: false,
            priority: NetworkPriority::NORMAL,
            destination: source_network.cloned(),
            source: None,
            payload: apdu_bytes.clone(),
            ..Npdu::default()
        };
        let mut npdu_buf = BytesMut::with_capacity(2 + apdu_bytes.len());
        match encode_npdu(&mut npdu_buf, &npdu) {
            Ok(()) => {
                let handed_off = tx.send(npdu_buf.freeze()).is_ok();
                // Synchronous reply-channel handoff, not the serial worker's
                // later turnaround. A failed handoff releases too, but is not
                // reported as a successful reply.
                drop(pending);
                if !handed_off && log_errors {
                    warn!("MS/TP reply receiver closed before response handoff");
                }
            }
            Err(error) => {
                if log_errors {
                    warn!(%error, "Failed to encode NPDU for MS/TP reply");
                }
                if let Err(error) = BACnetServer::<T>::issue_terminal_response(
                    network,
                    &apdu_bytes,
                    source_mac,
                    source_network,
                    route,
                    pending,
                )
                .await
                {
                    if log_errors {
                        warn!(%error, "Failed to send response");
                    }
                }
            }
        }
    } else if let Err(error) = BACnetServer::<T>::issue_terminal_response(
        network,
        &buf,
        source_mac,
        source_network,
        route,
        pending,
    )
    .await
    {
        if log_errors {
            warn!(%error, "Failed to send response");
        }
    }
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Convert an error into its protocol response APDU.
    pub(in crate::server) fn error_apdu_from_error(
        invoke_id: u8,
        service_choice: ConfirmedServiceChoice,
        error: &Error,
    ) -> Apdu {
        error_apdu_from_error(invoke_id, service_choice, error)
    }
}

#[cfg(test)]
#[path = "../alarm_summary_tests.rs"]
mod alarm_summary_tests;
