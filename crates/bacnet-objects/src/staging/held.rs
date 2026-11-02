//! The priority slots a Staging object's stage writes fill, kept so that
//! removing the object can give them up (#2265).
//!
//! Clause 12.62 says nothing about deletion; this follows the Schedule's
//! precedent (#2256, #2264). Every queued plan records its targets at the
//! priority it carries, and the slots stay recorded across stage, reference
//! and priority changes, since nothing relinquishes them then. On removal
//! the object owes one NULL per recorded slot, grouped by priority. The
//! server sends each only while the target's Value_Source still names the
//! Staging object, so a slot another writer has taken keeps that writer's
//! value (Clause 19.2 warns about two writers sharing a priority).
//!
//! The question of checking a Schedule's calculated NULLs for ownership
//! (#2265's second item) is out of scope here: a Staging object only writes
//! ACTIVE or INACTIVE, never a calculated NULL.

use crate::prelude::*;

use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::schedule::ScheduleWrite;

use super::StagingWritePlan;

/// The `(target, priority)` slots a Staging object has written. Bounded by
/// every target it has ever referenced times the 16 priorities.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct HeldSlots(Vec<(ObjectIdentifier, u8)>);

impl HeldSlots {
    /// Record the slots `plan` writes.
    pub(super) fn record(&mut self, plan: &StagingWritePlan) {
        for write in &plan.writes {
            let slot = (write.object_identifier, plan.priority);
            if !self.0.contains(&slot) {
                self.0.push(slot);
            }
        }
    }

    /// One relinquish NULL per priority, naming every target held at it,
    /// and forget them all.
    pub(super) fn take_relinquish_writes(&mut self) -> Vec<ScheduleWrite> {
        let mut writes: Vec<ScheduleWrite> = Vec::new();
        for (target, priority) in core::mem::take(&mut self.0) {
            let reference = BACnetObjectPropertyReference::new(
                target,
                PropertyIdentifier::PRESENT_VALUE.to_raw(),
            );
            match writes.iter_mut().find(|write| write.priority == priority) {
                Some(write) => write.references.push(reference),
                None => writes.push(
                    ScheduleWrite::new(PropertyValue::Null, priority, vec![reference])
                        .with_relinquish(true),
                ),
            }
        }
        writes
    }
}

#[cfg(test)]
#[path = "held_tests.rs"]
mod tests;
