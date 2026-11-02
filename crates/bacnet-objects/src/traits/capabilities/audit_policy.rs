//! The Audit policy capability: an object's own Audit_Level and
//! Auditable_Operations overrides, which a target Audit Reporter reads.

use crate::audit::{AuditPolicyAuthority, ObjectAuditPolicy};

/// An object that carries its own Audit overrides.
///
/// The object returns itself from
/// [`BACnetObject::audit_policy`](crate::traits::BACnetObject::audit_policy),
/// for the snapshot the bundled server takes under a shared database guard,
/// and from [`audit_policy_mut`](crate::traits::BACnetObject::audit_policy_mut)
/// for the pre-commit check of a write that changes them. An object that
/// lends one lends the other. The built-in Analog Value, Binary Value, Color
/// and Color Temperature objects lend it. An object that lends none has no
/// overrides: every field inherits the selected Reporter. Not public API yet
/// (#2179).
#[doc(hidden)]
pub trait AuditPolicy {
    /// A snapshot of the overrides. Absent fields inherit the selected
    /// Reporter; source reporting doesn't inspect remote policies.
    fn object_policy(&self) -> ObjectAuditPolicy;

    /// The sealed built-in assignment authority for the two policy
    /// properties. The default has none: the object's ordinary write route
    /// takes their writes.
    fn policy_authority(&mut self) -> Option<AuditPolicyAuthority<'_>> {
        None
    }
}

/// [`AuditPolicy`] for a built-in object whose `audit_policy` field holds
/// its overrides.
macro_rules! impl_audit_policy {
    ($object:ty) => {
        impl $crate::traits::AuditPolicy for $object {
            fn object_policy(&self) -> $crate::audit::ObjectAuditPolicy {
                self.audit_policy
            }

            fn policy_authority(&mut self) -> Option<$crate::audit::AuditPolicyAuthority<'_>> {
                Some($crate::audit::AuditPolicyAuthority::new(
                    &mut self.audit_policy,
                ))
            }
        }
    };
}

/// The two accessors of an object that lends itself as its [`AuditPolicy`],
/// for inside its `BACnetObject` impl.
macro_rules! lend_audit_policy {
    () => {
        fn audit_policy(&self) -> Option<&dyn $crate::traits::AuditPolicy> {
            Some(self)
        }

        fn audit_policy_mut(&mut self) -> Option<&mut dyn $crate::traits::AuditPolicy> {
            Some(self)
        }
    };
}

pub(crate) use {impl_audit_policy, lend_audit_policy};
