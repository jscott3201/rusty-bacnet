use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    pub(super) fn enrollment_summary_response(
        db: &ObjectDatabase,
        invoke_id: u8,
        request: &[u8],
        budget: GetEnrollmentSummaryBudget,
    ) -> Apdu {
        let service_choice = ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY;
        let mut buf = BytesMut::new();
        match handlers::handle_get_enrollment_summary_budgeted(db, request, &mut buf, budget) {
            Ok(()) => Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice,
                service_ack: buf.freeze(),
            }),
            Err(handlers::EnrollmentSummaryFailure::Service(e)) => {
                Self::error_apdu_from_error(invoke_id, service_choice, &e)
            }
            Err(failure) => Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id,
                abort_reason: match failure {
                    handlers::EnrollmentSummaryFailure::Work => AbortReason::OUT_OF_RESOURCES,
                    handlers::EnrollmentSummaryFailure::Bytes => AbortReason::BUFFER_OVERFLOW,
                    handlers::EnrollmentSummaryFailure::Service(_) => unreachable!(),
                },
            }),
        }
    }
}
