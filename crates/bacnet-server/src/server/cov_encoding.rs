use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    pub(super) fn encode_confirmed_cov_multiple_apdu(
        notification: &COVNotificationMultipleRequest,
        invoke_id: u8,
        max_apdu_length: u16,
    ) -> Result<BytesMut, Error> {
        let mut service_buf = BytesMut::new();
        notification.encode(&mut service_buf)?;
        let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
            service_request: service_buf.freeze(),
        });
        let mut buf = BytesMut::new();
        encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");
        Ok(buf)
    }

    pub(super) fn encode_unconfirmed_cov_multiple_apdu(
        notification: &COVNotificationMultipleRequest,
    ) -> Result<BytesMut, Error> {
        let mut service_buf = BytesMut::new();
        notification.encode(&mut service_buf)?;
        let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE,
            service_request: service_buf.freeze(),
        });
        let mut buf = BytesMut::new();
        encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");
        Ok(buf)
    }
}
