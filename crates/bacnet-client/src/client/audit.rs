use super::*;

use bacnet_services::audit::{AuditLogQueryAck, AuditLogQueryRequest, AuditNotificationRequest};

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Send a typed ConfirmedAuditNotification to a directly addressed MAC.
    ///
    /// The request uses the existing qualified Clause 21 Audit wire model.
    pub async fn confirmed_audit_notification(
        &self,
        destination_mac: &[u8],
        request: &AuditNotificationRequest,
    ) -> Result<(), Error> {
        let mut service_data = BytesMut::new();
        request.try_encode(&mut service_data)?;

        let _ = self
            .confirmed_request(
                destination_mac,
                ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
                &service_data,
            )
            .await?;

        Ok(())
    }

    /// Send a typed UnconfirmedAuditNotification to a directly addressed MAC.
    ///
    /// The request uses the existing qualified Clause 21 Audit wire model and
    /// completes after transport send without waiting for a response.
    pub async fn unconfirmed_audit_notification(
        &self,
        destination_mac: &[u8],
        request: &AuditNotificationRequest,
    ) -> Result<(), Error> {
        let mut service_data = BytesMut::new();
        request.try_encode(&mut service_data)?;

        self.unconfirmed_request(
            destination_mac,
            UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION,
            &service_data,
        )
        .await
    }

    /// Query an Audit Log at a directly addressed MAC and decode its typed ACK.
    ///
    /// Request and response use the existing qualified Clause 21 Audit wire
    /// model; this helper does not reinterpret query fields or execution.
    pub async fn audit_log_query(
        &self,
        destination_mac: &[u8],
        request: &AuditLogQueryRequest,
    ) -> Result<AuditLogQueryAck, Error> {
        let mut service_data = BytesMut::new();
        request.try_encode(&mut service_data)?;

        let response = self
            .confirmed_request(
                destination_mac,
                ConfirmedServiceChoice::AUDIT_LOG_QUERY,
                &service_data,
            )
            .await?;

        AuditLogQueryAck::decode(&response)
    }
}
