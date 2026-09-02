use super::*;

pub(super) fn can_segment(segmentation: Segmentation) -> bool {
    matches!(segmentation, Segmentation::BOTH | Segmentation::TRANSMIT)
}

pub(super) fn limit(peer: u16, local: u32) -> u16 {
    peer.min(u16::try_from(local).unwrap_or(u16::MAX))
}

pub(super) async fn response(
    db: &RwLock<ObjectDatabase>,
    request: &ConfirmedRequestPdu,
    effective_max_apdu: u16,
    segmented_response_available: bool,
) -> Apdu {
    let service_ack_budget = (!segmented_response_available).then(|| {
        unsegmented_complex_ack_service_budget(
            request.invoke_id,
            request.service_choice,
            effective_max_apdu,
        )
    });
    let mut service_ack = BytesMut::new();
    let db = db.read().await;
    match handlers::handle_get_event_information_with_budget(
        &db,
        &request.service_request,
        &mut service_ack,
        service_ack_budget,
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
        Err(error) => confirmed_response::error_apdu_from_error(
            request.invoke_id,
            request.service_choice,
            &error,
        ),
    }
}

fn unsegmented_complex_ack_service_budget(
    invoke_id: u8,
    service_choice: ConfirmedServiceChoice,
    max_apdu: u16,
) -> usize {
    let envelope = Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice,
        service_ack: Bytes::new(),
    });
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, &envelope).expect("valid empty ComplexACK encoding");
    usize::from(max_apdu).saturating_sub(encoded.len())
}
