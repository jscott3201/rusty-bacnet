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
    reporters: Vec<ObjectIdentifier>,
    association: Option<Arc<crate::audit::TargetAuditAssociation>>,
    active: AtomicBool,
}

impl AuditOwnership {
    pub fn for_source(device: ObjectIdentifier, reporter: ObjectIdentifier) -> Arc<Self> {
        Arc::new(Self {
            device,
            reporters: vec![reporter],
            association: None,
            active: AtomicBool::new(true),
        })
    }
    pub fn for_target(
        device: ObjectIdentifier,
        association: Arc<crate::audit::TargetAuditAssociation>,
    ) -> Arc<Self> {
        Arc::new(Self {
            device,
            reporters: association
                .reporters()
                .iter()
                .map(|(oid, _)| *oid)
                .collect(),
            association: Some(association),
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
        *oid == self.device || self.reporters.contains(oid)
    }
}

impl ObjectDatabase {
    /// Install one runtime lifetime guard; an existing live owner rejects replacement.
    #[doc(hidden)]
    pub fn protect_audit_internal(&mut self, owner: &Arc<AuditOwnership>) -> Result<(), Error> {
        self.validate_audit_installation_internal()?;
        if let Some(association) = &owner.association {
            association.set_subjects(self.list_objects());
        }
        self.audit_owner = Some(Arc::downgrade(owner));
        Ok(())
    }
    /// Reject installation while any previous runtime frame still owns the database.
    #[doc(hidden)]
    pub fn validate_audit_installation_internal(&self) -> Result<(), Error> {
        if self.audit_owner.as_ref().and_then(Weak::upgrade).is_some() {
            return Err(protected());
        }
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
    pub(super) fn audit_membership_changed(&self, oid: ObjectIdentifier, present: bool) {
        if let Some(owner) = self.audit_owner.as_ref().and_then(Weak::upgrade) {
            if let Some(association) = &owner.association {
                association.membership_changed(oid, present);
            }
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
