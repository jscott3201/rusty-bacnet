use super::*;

pub(super) async fn response(
    db: &RwLock<ObjectDatabase>,
    request: &ConfirmedRequestPdu,
    mut budget: ReadRangeBudget,
    effective_max_apdu: u16,
    segmented_response_available: bool,
) -> Apdu {
    if !segmented_response_available {
        budget.max_service_ack_bytes = budget.max_service_ack_bytes.min(
            event_information::unsegmented_complex_ack_service_budget(
                request.invoke_id,
                request.service_choice,
                effective_max_apdu,
            ),
        );
    }
    let mut service_ack = BytesMut::new();
    let db = db.read().await;
    match handlers::handle_read_range_budgeted(
        &db,
        &request.service_request,
        &mut service_ack,
        budget,
    ) {
        Ok(()) => Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id: request.invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: request.service_choice,
            service_ack: service_ack.freeze(),
        }),
        Err(handlers::ReadRangeFailure::Service(error)) => {
            confirmed_response::error_apdu_from_error(
                request.invoke_id,
                request.service_choice,
                &error,
            )
        }
        Err(handlers::ReadRangeFailure::Bytes) => Apdu::Abort(AbortPdu {
            sent_by_server: true,
            invoke_id: request.invoke_id,
            abort_reason: AbortReason::BUFFER_OVERFLOW,
        }),
    }
}
