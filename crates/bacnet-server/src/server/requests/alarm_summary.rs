use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Answer a GetAlarmSummary. The request carries no parameters (Clause
    /// 13.10.1), so any octet in it is refused as a malformed request is.
    pub(super) fn alarm_summary_response(
        db: &ObjectDatabase,
        invoke_id: u8,
        service_request: &[u8],
        budget: GetAlarmSummaryBudget,
    ) -> Apdu {
        let service_choice = ConfirmedServiceChoice::GET_ALARM_SUMMARY;
        if let Err(error) = bacnet_encoding::constructed::tagged::expect_end(
            service_request,
            0,
            0,
            "GetAlarmSummary",
        ) {
            return bacnet_device_core::reply::error_apdu(
                invoke_id,
                service_choice,
                &error.into_request_reject(),
            );
        }
        let mut buf = BytesMut::new();
        match handlers::handle_get_alarm_summary_budgeted(db, &mut buf, budget) {
            Ok(()) => Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice,
                service_ack: buf.freeze(),
            }),
            Err(handlers::AlarmSummaryFailure::Service(e)) => {
                bacnet_device_core::reply::error_apdu(invoke_id, service_choice, &e)
            }
            Err(failure) => Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id,
                abort_reason: match failure {
                    handlers::AlarmSummaryFailure::Work => AbortReason::OUT_OF_RESOURCES,
                    handlers::AlarmSummaryFailure::Bytes => AbortReason::BUFFER_OVERFLOW,
                    handlers::AlarmSummaryFailure::Service(_) => unreachable!(),
                },
            }),
        }
    }
}
