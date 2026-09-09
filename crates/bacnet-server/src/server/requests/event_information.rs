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
    mut budget: GetEventInformationBudget,
    effective_max_apdu: u16,
    segmented_response_available: bool,
) -> Apdu {
    if !segmented_response_available {
        budget.max_service_ack_bytes =
            budget
                .max_service_ack_bytes
                .min(unsegmented_complex_ack_service_budget(
                    request.invoke_id,
                    request.service_choice,
                    effective_max_apdu,
                ));
    }
    let mut service_ack = BytesMut::new();
    let db = db.read().await;
    match handlers::handle_get_event_information_configured(
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
        Err(handlers::EventInformationFailure::Service(error)) => {
            confirmed_response::error_apdu_from_error(
                request.invoke_id,
                request.service_choice,
                &error,
            )
        }
        Err(failure) => Apdu::Abort(AbortPdu {
            sent_by_server: true,
            invoke_id: request.invoke_id,
            abort_reason: match failure {
                handlers::EventInformationFailure::Objects => AbortReason::OUT_OF_RESOURCES,
                _ => AbortReason::BUFFER_OVERFLOW,
            },
        }),
    }
}

pub(super) fn unsegmented_complex_ack_service_budget(
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

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[tokio::test]
    async fn get_event_information_default_rejects_4097_total_objects() {
        for count in [4096, 4097] {
            let mut database = ObjectDatabase::new();
            for instance in 0..count {
                database
                    .add(Box::new(
                        bacnet_objects::notification_class::NotificationClass::new(
                            instance,
                            format!("NC-{instance}"),
                        )
                        .unwrap(),
                    ))
                    .unwrap();
            }
            let request = ConfirmedRequestPdu {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: true,
                max_segments: None,
                max_apdu_length: 1476,
                invoke_id: 77,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::GET_EVENT_INFORMATION,
                service_request: Bytes::new(),
            };
            let response = response(
                &RwLock::new(database),
                &request,
                GetEventInformationBudget::default(),
                1476,
                true,
            )
            .await;
            if count == 4096 {
                assert!(
                    matches!(response, Apdu::ComplexAck(ack) if ack.service_ack.as_ref() == [0x0e, 0x0f, 0x19, 0])
                );
            } else {
                assert!(matches!(response, Apdu::Abort(abort)
            if abort.sent_by_server && abort.abort_reason == AbortReason::OUT_OF_RESOURCES));
            }
        }
    }
}
