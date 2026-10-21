//! Receiver policy for confirmed and unconfirmed AuditNotification services.

use bacnet_encoding::npdu::NpduAddress;
use bacnet_services::audit::AuditNotificationRequest;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;

/// Local maximum accepted AuditNotification service payload.
pub const MAX_AUDIT_NOTIFICATION_BYTES: usize = 64 * 1024;
/// Local maximum number of notifications in one accepted request.
pub const MAX_AUDIT_NOTIFICATIONS: usize = 256;

/// Transport provenance and decoded content supplied to
/// [`ServerPolicy::authorize_audit_notification`](crate::ServerPolicy::authorize_audit_notification).
///
/// The source/target identities inside `request` are peer-reported audit data,
/// not authenticated transport provenance. Policy should use `source_network`
/// for a usable routed origin and `source_mac` for the immediate data-link peer.
/// `Debug` is redacted: address lengths, the sink, whether a principal is
/// present and the notification count, never the notifications themselves.
#[derive(Clone, PartialEq)]
pub struct AuditNotificationAuthorizationContext {
    /// Immediate data-link peer (normally a router for routed traffic).
    pub source_mac: MacAddr,
    /// Originating NPDU source when one was present.
    pub source_network: Option<NpduAddress>,
    /// Honest transport + origin provenance for this ingress; the source of
    /// the principal, if any.
    pub provenance: TransportProvenance,
    /// Outer Confirmed-Request invoke identifier.
    pub invoke_id: u8,
    /// Explicitly configured local Audit Log sink.
    pub audit_log_sink: ObjectIdentifier,
    /// Decoded peer-reported notification list, preserved verbatim.
    pub request: AuditNotificationRequest,
}

impl AuditNotificationAuthorizationContext {
    /// The principal of the peer that originated the request: the verified
    /// leaf certificate of a direct BACnet/SC connection, when the request
    /// carries no routed source. `None` for other ingress, hub-relayed and
    /// routed requests included (see [`crate::policy`]).
    pub fn principal(&self) -> Option<bacnet_transport::port::ScPrincipal> {
        crate::policy::originating_principal(self.provenance, self.source_network.as_ref())
    }

    /// The principal of the direct BACnet/SC peer that handed this request
    /// over, a router included when the request was routed; `None` off a
    /// verified direct SC connection. Trusting it trusts everything that
    /// peer forwards.
    pub fn forwarding_peer_principal(&self) -> Option<bacnet_transport::port::ScPrincipal> {
        self.provenance.principal()
    }
}

/// Transport provenance and decoded content supplied to
/// [`ServerPolicy::authorize_unconfirmed_audit_notification`](crate::ServerPolicy::authorize_unconfirmed_audit_notification).
///
/// The source/target identities inside `request` are peer-reported audit data,
/// not authenticated transport provenance. Policy should use `source_network`
/// for a usable routed origin and `source_mac` for the immediate data-link peer.
/// `Debug` is redacted: address lengths, the sink, whether a principal is
/// present and the notification count, never the notifications themselves.
#[derive(Clone, PartialEq)]
pub struct UnconfirmedAuditNotificationAuthorizationContext {
    /// Immediate data-link peer (normally a router for routed traffic).
    pub source_mac: MacAddr,
    /// Originating NPDU source when one was present.
    pub source_network: Option<NpduAddress>,
    /// Honest transport + origin provenance for this ingress; the source of
    /// the principal, if any.
    pub provenance: TransportProvenance,
    /// Explicitly configured local Audit Log sink.
    pub audit_log_sink: ObjectIdentifier,
    /// Decoded peer-reported notification list, preserved verbatim.
    pub request: AuditNotificationRequest,
}

impl UnconfirmedAuditNotificationAuthorizationContext {
    /// The principal of the peer that originated the request: the verified
    /// leaf certificate of a direct BACnet/SC connection, when the request
    /// carries no routed source. `None` for other ingress, hub-relayed and
    /// routed requests included (see [`crate::policy`]).
    pub fn principal(&self) -> Option<bacnet_transport::port::ScPrincipal> {
        crate::policy::originating_principal(self.provenance, self.source_network.as_ref())
    }

    /// The principal of the direct BACnet/SC peer that handed this request
    /// over, a router included when the request was routed; `None` off a
    /// verified direct SC connection. Trusting it trusts everything that
    /// peer forwards.
    pub fn forwarding_peer_principal(&self) -> Option<bacnet_transport::port::ScPrincipal> {
        self.provenance.principal()
    }
}

impl std::fmt::Debug for AuditNotificationAuthorizationContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditNotificationAuthorizationContext")
            .field("source_mac_len", &self.source_mac.len())
            .field(
                "source_network",
                &self
                    .source_network
                    .as_ref()
                    .map(|source| (source.network, source.mac_address.len())),
            )
            .field("provenance", &self.provenance)
            .field("principal", &self.principal().is_some())
            .field("invoke_id", &self.invoke_id)
            .field("audit_log_sink", &self.audit_log_sink)
            .field("notifications", &self.request.notifications.len())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for UnconfirmedAuditNotificationAuthorizationContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnconfirmedAuditNotificationAuthorizationContext")
            .field("source_mac_len", &self.source_mac.len())
            .field(
                "source_network",
                &self
                    .source_network
                    .as_ref()
                    .map(|source| (source.network, source.mac_address.len())),
            )
            .field("provenance", &self.provenance)
            .field("principal", &self.principal().is_some())
            .field("audit_log_sink", &self.audit_log_sink)
            .field("notifications", &self.request.notifications.len())
            .finish_non_exhaustive()
    }
}
