//! The priority slots a Staging object's stage writes fill, kept so that a
//! change of its targets or its removal can give them up (#2265, #2289).
//!
//! Clause 12.62 says nothing about what a change of Target_References or
//! Priority_For_Writing, or the object's deletion, does to the slots already
//! written; this follows the Schedule's precedent (#1088, #2256, #2264).
//! Every queued plan records its targets at the priority it carries. A
//! change of the references or the priority moves each recorded slot the
//! new configuration no longer writes (a target dropped from the array, or
//! every target at the old priority) to the owed list. The server sends
//! those NULLs under the guard of the next target write the object's plans
//! make, before that write, so a NULL owed to a slot always lands before any
//! later write of the object to it. A slot the new configuration writes
//! again before its NULL is sent stops being owed. On removal the object
//! owes one NULL per slot, recorded or owed, grouped by priority. The server
//! sends each only while the target's Value_Source still names the Staging
//! object, so a slot another writer has taken keeps that writer's value
//! (Clause 19.2 warns about two writers sharing a priority).
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

/// One `(target, priority)` slot.
type Slot = (ObjectIdentifier, u8);

/// The `(target, priority)` slots a Staging object has written and still
/// holds, and those a change of its targets left it owing a NULL. A slot is
/// in one list at most. Bounded by every target it has ever referenced times
/// the 16 priorities.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct HeldSlots {
    held: Vec<Slot>,
    owed: Vec<Slot>,
}

impl HeldSlots {
    /// Record the slots `plan` writes; one that was owed a NULL is held
    /// again instead.
    pub(super) fn record(&mut self, plan: &StagingWritePlan) {
        for write in &plan.writes {
            let slot = (write.object_identifier, plan.priority);
            self.owed.retain(|owed| *owed != slot);
            if !self.held.contains(&slot) {
                self.held.push(slot);
            }
        }
    }

    /// The targets or the priority changed: owe a NULL to every held slot
    /// that isn't one of `targets` at `priority`, and stop holding it.
    pub(super) fn release_uncovered(&mut self, targets: &[ObjectIdentifier], priority: u8) {
        let (kept, left): (Vec<_>, Vec<_>) = core::mem::take(&mut self.held)
            .into_iter()
            .partition(|(target, at)| *at == priority && targets.contains(target));
        self.held = kept;
        self.owed.extend(left);
    }

    /// The NULLs a change of the targets owes, one per priority, and forget
    /// them.
    pub(super) fn take_owed_writes(&mut self) -> Vec<ScheduleWrite> {
        relinquish_writes(core::mem::take(&mut self.owed))
    }

    /// One relinquish NULL per priority, naming every target owed or held
    /// at it, and forget them all.
    pub(super) fn take_relinquish_writes(&mut self) -> Vec<ScheduleWrite> {
        let mut slots = core::mem::take(&mut self.owed);
        slots.append(&mut self.held);
        relinquish_writes(slots)
    }
}

/// Group `slots` into one relinquish NULL per priority, in first-seen order.
fn relinquish_writes(slots: Vec<Slot>) -> Vec<ScheduleWrite> {
    let mut writes: Vec<ScheduleWrite> = Vec::new();
    for (target, priority) in slots {
        let reference =
            BACnetObjectPropertyReference::new(target, PropertyIdentifier::PRESENT_VALUE.to_raw());
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

impl super::StagingObject {
    /// Target_References or Priority_For_Writing changed (#2289): owe a NULL
    /// to each held slot the new configuration no longer writes, then apply
    /// the current stage afresh. An uninitialized reference (Clause 12.62.14)
    /// writes nothing, so it covers no slot.
    pub(super) fn targets_changed(&mut self) {
        let targets: Vec<ObjectIdentifier> = self
            .target_references
            .iter()
            .map(|reference| reference.object_identifier)
            .filter(|target| target.instance_number() != ObjectIdentifier::MAX_INSTANCE)
            .collect();
        self.held
            .release_uncovered(&targets, self.priority_for_writing);
        self.reset_and_reevaluate();
    }
}

#[cfg(test)]
#[path = "held_tests.rs"]
mod tests;
