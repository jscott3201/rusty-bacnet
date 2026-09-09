use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    pub(super) fn atomic_read_file_response(
        db: &ObjectDatabase,
        invoke_id: u8,
        request: &[u8],
        budget: AtomicReadFileBudget,
    ) -> Apdu {
        let service_choice = ConfirmedServiceChoice::ATOMIC_READ_FILE;
        let mut buf = BytesMut::new();
        match handlers::handle_atomic_read_file_budgeted(db, request, &mut buf, budget) {
            Ok(()) => Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice,
                service_ack: buf.freeze(),
            }),
            Err(handlers::AtomicReadFileFailure::Service(error)) => {
                Self::error_apdu_from_error(invoke_id, service_choice, &error)
            }
            Err(handlers::AtomicReadFileFailure::Budget(abort_reason)) => Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id,
                abort_reason,
            }),
        }
    }
}
