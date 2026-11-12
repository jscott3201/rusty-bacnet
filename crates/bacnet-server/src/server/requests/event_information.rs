use super::*;

pub(super) fn can_segment(segmentation: Segmentation) -> bool {
    matches!(
        segmentation,
        Segmentation::SEGMENTED_BOTH | Segmentation::SEGMENTED_TRANSMIT
    )
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
            bacnet_device_core::reply::error_apdu(request.invoke_id, request.service_choice, &error)
        }
        Err(handlers::EventInformationFailure::Projection(problem)) => {
            warn!(
                object_identifier = ?problem.object_identifier,
                reason = %describe(&problem.fault),
                "GetEventInformation projection failed"
            );
            bacnet_device_core::reply::error_apdu(
                request.invoke_id,
                request.service_choice,
                &problem.to_error(),
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

/// The reason a summary failed, as the projection once logged it.
fn describe(fault: &handlers::EventSummaryFault) -> String {
    use handlers::EventSummaryFault as F;
    let coordinate = |transition: &bacnet_objects::event::EventTransition| match transition {
        bacnet_objects::event::EventTransition::ToOffnormal => "TO_OFFNORMAL",
        bacnet_objects::event::EventTransition::ToFault => "TO_FAULT",
        bacnet_objects::event::EventTransition::ToNormal => "TO_NORMAL",
    };
    match fault {
        F::Unreadable { property, error } => {
            format!("required property {property:?} is unreadable: {error}")
        }
        F::NotEnumerated(property) => format!("required property {property:?} is not Enumerated"),
        F::ExceedsUnsigned32(property) => {
            format!("required property {property:?} exceeds Unsigned32")
        }
        F::NotUnsigned(property) => format!("required property {property:?} is not Unsigned"),
        F::NotTransitionBits(property) => {
            format!("required property {property:?} is not a canonical three-bit BitString")
        }
        F::DetectionEnableNotBoolean => "Event_Detection_Enable is not Boolean".to_owned(),
        F::TimestampsNotList => "Event_Time_Stamps is not a three-item List".to_owned(),
        F::TimestampsNotThree => {
            "Event_Time_Stamps does not contain exactly three items".to_owned()
        }
        F::InvalidTimestamp(transition) => {
            format!("invalid {} event timestamp", coordinate(transition))
        }
        F::DuplicateClass(class) => {
            format!("multiple Notification Class objects match {class}")
        }
        F::MissingClass(class) => format!("no Notification Class object matches {class}"),
        F::PriorityNotList => "Notification Class Priority is not a three-item List".to_owned(),
        F::PriorityNotThree => {
            "Notification Class Priority does not contain exactly three items".to_owned()
        }
        F::PriorityOver255(transition) => format!(
            "Notification Class {} Priority exceeds 255",
            coordinate(transition)
        ),
        F::PriorityNotUnsigned(transition) => format!(
            "Notification Class {} Priority is not Unsigned",
            coordinate(transition)
        ),
    }
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
