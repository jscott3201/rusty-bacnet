//! Lifetime protection for one installed target Audit runtime.
use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Weak,
};

/// A lease retained by the runtime and every task that can still use its profile.
/// Sealing denies writes; structural protection lasts until all leases are gone.
#[doc(hidden)]
pub struct AuditOwnership {
    device: ObjectIdentifier,
    reporter: ObjectIdentifier,
    active: AtomicBool,
}

impl AuditOwnership {
    pub fn new(device: ObjectIdentifier, reporter: ObjectIdentifier) -> Arc<Self> {
        Arc::new(Self {
            device,
            reporter,
            active: AtomicBool::new(true),
        })
    }
    pub fn seal(&self) {
        self.active.store(false, Ordering::Release);
    }
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
    fn protects(&self, oid: &ObjectIdentifier) -> bool {
        *oid == self.device || *oid == self.reporter
    }
}

impl ObjectDatabase {
    /// Install one runtime lifetime guard; an existing live owner rejects replacement.
    #[doc(hidden)]
    pub fn protect_audit_internal(&mut self, owner: &Arc<AuditOwnership>) -> Result<(), Error> {
        if self.audit_owner.as_ref().and_then(Weak::upgrade).is_some() {
            return Err(protected());
        }
        self.audit_owner = Some(Arc::downgrade(owner));
        Ok(())
    }
    /// Release the matching sealed owner after its runtime has joined all tasks.
    #[doc(hidden)]
    pub fn release_audit_internal(&mut self, owner: &Arc<AuditOwnership>) {
        if !owner.is_active()
            && self
                .audit_owner
                .as_ref()
                .and_then(Weak::upgrade)
                .is_some_and(|current| Arc::ptr_eq(&current, owner))
        {
            self.audit_owner = None;
        }
    }
    pub(super) fn check_audit_membership(
        &self,
        oid: &ObjectIdentifier,
        insertion: bool,
    ) -> Result<(), Error> {
        if self
            .audit_owner
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|owner| {
                owner.protects(oid) || (insertion && oid.object_type() == ObjectType::DEVICE)
            })
        {
            return Err(protected());
        }
        Ok(())
    }
}

fn protected() -> Error {
    Error::Protocol {
        class: ErrorClass::OBJECT.to_raw() as u32,
        code: ErrorCode::OBJECT_DELETION_NOT_PERMITTED.to_raw() as u32,
    }
}
