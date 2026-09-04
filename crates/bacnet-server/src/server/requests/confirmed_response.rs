use super::*;

pub(super) async fn read_property_response(
    db: &RwLock<ObjectDatabase>,
    request: &ConfirmedRequestPdu,
) -> Apdu {
    let mut service_ack = BytesMut::with_capacity(512);
    let db = db.read().await;
    match handlers::handle_read_property(&db, &request.service_request, &mut service_ack) {
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

pub(super) fn error_fields(error: &Error) -> (ErrorClass, ErrorCode) {
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
pub(super) async fn send_unsegmented_response<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    response: &Apdu,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
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
                warn!(%error, "Failed to encode NPDU for MS/TP reply");
                if let Err(error) = BACnetServer::<T>::send_confirmed_response_apdu(
                    network,
                    &apdu_bytes,
                    source_mac,
                    source_network,
                )
                .await
                {
                    warn!(%error, "Failed to send response");
                }
            }
        }
    } else if let Err(error) =
        BACnetServer::<T>::send_confirmed_response_apdu(network, &buf, source_mac, source_network)
            .await
    {
        warn!(%error, "Failed to send response");
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
