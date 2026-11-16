use super::*;

use bacnet_services::audit::{AuditLogQueryAck, AuditLogQueryRequest, AuditNotificationRequest};

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Send a typed ConfirmedAuditNotification to the station `to`.
    ///
    /// The request uses the existing qualified Clause 21 Audit wire model.
    pub async fn confirmed_audit_notification(
        &self,
        to: &(impl ToStation + ?Sized),
        request: &AuditNotificationRequest,
    ) -> Result<(), Error> {
        self.call(to, request).await
    }

    /// Send a typed UnconfirmedAuditNotification to the station `to`.
    ///
    /// The request uses the existing qualified Clause 21 Audit wire model and
    /// completes after transport send without waiting for a response.
    pub async fn unconfirmed_audit_notification(
        &self,
        to: &(impl ToStation + ?Sized),
        request: &AuditNotificationRequest,
    ) -> Result<(), Error> {
        self.tell(to, request).await
    }

    /// Query an Audit Log on the station `to` and decode its typed ACK.
    ///
    /// Request and response use the existing qualified Clause 21 Audit wire
    /// model; this helper does not reinterpret query fields or execution.
    pub async fn audit_log_query(
        &self,
        to: &(impl ToStation + ?Sized),
        request: &AuditLogQueryRequest,
    ) -> Result<AuditLogQueryAck, Error> {
        self.call(to, request).await
    }
}
