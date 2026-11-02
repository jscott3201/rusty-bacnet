//! The Audit Log capability: an Audit Log's retained records as
//! AuditLogQuery and ReadRange read them, its forwarding profile, its
//! trusted local parent, and the Audit notifications it takes.

use alloc::sync::Arc;

use bacnet_types::constructed::BACnetDeviceObjectReference;
use bacnet_types::error::Error;

use super::not_supported;
use crate::audit::{AuditLogForwarding, AuditLogNotificationSink, AuditLogStorage};

/// An Audit Log whose records the host queries and fills.
///
/// The object returns itself from
/// [`BACnetObject::audit_log`](crate::traits::BACnetObject::audit_log), for
/// the queries and the forwarding profile the bundled server reads under a
/// shared database guard, and from
/// [`audit_log_mut`](crate::traits::BACnetObject::audit_log_mut) for the
/// parent a binding sets and the notifications the server stores under the
/// write guard. An object that lends one lends the other. The built-in
/// [`AuditLogObject`](crate::audit::AuditLogObject) lends it. Not public API
/// yet (#2179).
#[doc(hidden)]
pub trait AuditLog {
    /// The read-only query channel over the already-loaded retained
    /// records. It exposes no persistence, mutation or downcasting.
    fn storage(&self) -> &dyn AuditLogStorage;

    /// The instance's forwarding profile, never durable pending work. The
    /// default has none: the log forwards nothing.
    fn forwarding(&self) -> Option<Arc<AuditLogForwarding>> {
        None
    }

    /// Set a trusted local parent Audit Log. This is no network property
    /// write: callers own configuration admission, and the server still
    /// validates the route. The default refuses with [`not_supported`].
    fn set_parent(&mut self, _parent: BACnetDeviceObjectReference) -> Result<(), Error> {
        Err(not_supported())
    }

    /// The receiver for Audit notifications. Implementations own their
    /// persistence transaction and leave memory unchanged when a batch
    /// can't be committed. The default has none: the log takes no
    /// notifications.
    fn notification_sink(&mut self) -> Option<&mut dyn AuditLogNotificationSink> {
        None
    }
}
