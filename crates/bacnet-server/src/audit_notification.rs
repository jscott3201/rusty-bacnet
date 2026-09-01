//! Receiver policy for confirmed and unconfirmed AuditNotification services.

use std::sync::Arc;

use bacnet_encoding::npdu::NpduAddress;
use bacnet_services::audit::AuditNotificationRequest;
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;

/// Local maximum accepted AuditNotification service payload.
pub const MAX_AUDIT_NOTIFICATION_BYTES: usize = 64 * 1024;
/// Local maximum number of notifications in one accepted request.
pub const MAX_AUDIT_NOTIFICATIONS: usize = 256;

/// Transport provenance and decoded content supplied to the Audit authorizer.
///
/// The source/target identities inside `request` are peer-reported audit data,
/// not authenticated transport provenance. Policy should use `source_network`
/// for a usable routed origin and `source_mac` for the immediate data-link peer.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditNotificationAuthorizationContext {
    /// Immediate data-link peer (normally a router for routed traffic).
    pub source_mac: MacAddr,
    /// Originating NPDU source when one was present.
    pub source_network: Option<NpduAddress>,
    /// Outer Confirmed-Request invoke identifier.
    pub invoke_id: u8,
    /// Explicitly configured local Audit Log sink.
    pub audit_log_sink: ObjectIdentifier,
    /// Decoded peer-reported notification list, preserved verbatim.
    pub request: AuditNotificationRequest,
}

/// Fast, nonblocking authorization callback for ConfirmedAuditNotification.
///
/// Absence, `false`, or a panic denies the request before storage mutation.
pub type AuditNotificationAuthorizer =
    Arc<dyn Fn(&AuditNotificationAuthorizationContext) -> bool + Send + Sync>;

/// Transport provenance and decoded content supplied to the unconfirmed Audit
/// authorizer.
///
/// The source/target identities inside `request` are peer-reported audit data,
/// not authenticated transport provenance. Policy should use `source_network`
/// for a usable routed origin and `source_mac` for the immediate data-link peer.
#[derive(Debug, Clone, PartialEq)]
pub struct UnconfirmedAuditNotificationAuthorizationContext {
    /// Immediate data-link peer (normally a router for routed traffic).
    pub source_mac: MacAddr,
    /// Originating NPDU source when one was present.
    pub source_network: Option<NpduAddress>,
    /// Explicitly configured local Audit Log sink.
    pub audit_log_sink: ObjectIdentifier,
    /// Decoded peer-reported notification list, preserved verbatim.
    pub request: AuditNotificationRequest,
}

/// Fast, nonblocking authorization callback for UnconfirmedAuditNotification.
///
/// Absence, `false`, or a panic silently denies the request before storage
/// mutation.
pub type UnconfirmedAuditNotificationAuthorizer =
    Arc<dyn Fn(&UnconfirmedAuditNotificationAuthorizationContext) -> bool + Send + Sync>;
