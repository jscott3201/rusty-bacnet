use super::*;
use crate::cov::active::{LiveCovSelection, LiveDeviceCov};
use bacnet_services::read_property::ReadPropertyRequest;

/// ReadProperty for a responder without COV service execution: the Device's
/// `Active_COV_Subscriptions` and `Active_COV_Multiple_Subscriptions` read as
/// their standalone empty lists.
pub(super) async fn read_property_response(
    db: &RwLock<ObjectDatabase>,
    request: &ConfirmedRequestPdu,
) -> Apdu {
    read_property_response_observed(db, None, request, |_, _, _, _| {}).await
}

/// Request-local live Device `Active_COV_Subscriptions` and
/// `Active_COV_Multiple_Subscriptions` (Clause 12.11), as selected.
///
/// Lock order is database, then COV table: the caller holds the database read
/// guard; the table read guard is held only to sample one instant and copy the
/// selected live entries, and is released before any object read or encoding.
pub(in crate::server) async fn active_cov_snapshot(
    db: &ObjectDatabase,
    cov_table: &RwLock<CovSubscriptionTable>,
    selection: LiveCovSelection,
) -> LiveDeviceCov {
    let entries = {
        let table = cov_table.read().await;
        table.live_cov_entries(selection, Instant::now())
    };
    LiveDeviceCov::project(db, selection, entries)
}

/// Budgeted ReadPropertyMultiple under one database read guard. A single
/// request-local Device projection serves every explicit and expanded row.
pub(super) async fn read_property_multiple_observed(
    db: &RwLock<ObjectDatabase>,
    cov_table: &RwLock<CovSubscriptionTable>,
    service_request: &[u8],
    service_ack: &mut BytesMut,
    budget: crate::server::ReadPropertyMultipleBudget,
    mut completed: impl FnMut(
        &ObjectDatabase,
        ObjectIdentifier,
        PropertyIdentifier,
        Option<u32>,
        Option<(ErrorClass, ErrorCode)>,
    ),
) -> Result<(), handlers::RpmFailure> {
    let db = db.read().await;
    let request = bacnet_services::rpm::ReadPropertyMultipleRequest::decode(service_request)
        .map_err(handlers::RpmFailure::Service)?;
    let live = match handlers::active_cov_device_for_rpm(&db, &request) {
        Some(selection) => Some(active_cov_snapshot(&db, cov_table, selection).await),
        None => None,
    };
    handlers::rpm_budgeted_request_observed(
        &db,
        live.as_ref(),
        &request,
        service_ack,
        budget,
        |oid, property, index, result| completed(&db, oid, property, index, result),
    )
}

pub(super) async fn read_property_response_observed(
    db: &RwLock<ObjectDatabase>,
    cov_table: Option<&RwLock<CovSubscriptionTable>>,
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
    let result = match ReadPropertyRequest::decode(&request.service_request) {
        Ok(decoded) => {
            let lookup_oid = handlers::resolve_device_wildcard(&db, &decoded.object_identifier);
            let live = match (
                cov_table,
                handlers::active_cov_device(&db, lookup_oid, decoded.property_identifier),
            ) {
                (Some(cov_table), Some(selection)) => {
                    Some(active_cov_snapshot(&db, cov_table, selection).await)
                }
                _ => None,
            };
            handlers::read_property_request_observed(
                &db,
                live.as_ref(),
                &decoded,
                &mut service_ack,
                |oid, request, result| completed(&db, oid, request, result),
            )
        }
        Err(error) => Err(error),
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
        Err(error) => error_apdu_from_error(request.invoke_id, request.service_choice, &error),
    }
}

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
        Error::Protocol { class, code } => (
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
    reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
) {
    send_response(
        network,
        response,
        source_mac,
        source_network,
        reply_tx,
        true,
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
    reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
) {
    send_response(
        network,
        response,
        source_mac,
        source_network,
        reply_tx,
        false,
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
    reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
) {
    send_raw_response(network, apdu_bytes, source_mac, source_network, reply_tx).await;
}

async fn send_raw_response<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    apdu_bytes: &Bytes,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
) {
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
    )
    .await
    {
        warn!(%error, "Failed to send replay");
    }
}

async fn send_response<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    response: &Apdu,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
    log_errors: bool,
) {
    let mut buf = BytesMut::new();
    encode_apdu(&mut buf, response).expect("valid APDU encoding");

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
                let _ = tx.send(npdu_buf.freeze());
            }
            Err(error) => {
                if log_errors {
                    warn!(%error, "Failed to encode NPDU for MS/TP reply");
                }
                if let Err(error) = BACnetServer::<T>::send_confirmed_response_apdu(
                    network,
                    &apdu_bytes,
                    source_mac,
                    source_network,
                )
                .await
                {
                    if log_errors {
                        warn!(%error, "Failed to send response");
                    }
                }
            }
        }
    } else if let Err(error) =
        BACnetServer::<T>::send_confirmed_response_apdu(network, &buf, source_mac, source_network)
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
