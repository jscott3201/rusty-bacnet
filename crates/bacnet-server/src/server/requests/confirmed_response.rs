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
