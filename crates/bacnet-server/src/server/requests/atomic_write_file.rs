use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    pub(super) fn atomic_write_file_response(
        db: &mut ObjectDatabase,
        invoke_id: u8,
        request: &[u8],
        budget: AtomicWriteFileBudget,
    ) -> Apdu {
        let service_choice = ConfirmedServiceChoice::ATOMIC_WRITE_FILE;
        let mut buf = BytesMut::new();
        match handlers::handle_atomic_write_file_budgeted(db, request, &mut buf, budget) {
            Ok(()) => Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice,
                service_ack: buf.freeze(),
            }),
            Err(handlers::AtomicWriteFileFailure::Service(error)) => {
                Self::error_apdu_from_error(invoke_id, service_choice, &error)
            }
            Err(handlers::AtomicWriteFileFailure::Budget) => Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id,
                abort_reason: AbortReason::OUT_OF_RESOURCES,
            }),
        }
    }
}
