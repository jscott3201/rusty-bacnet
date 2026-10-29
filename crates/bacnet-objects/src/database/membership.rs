//! Work a change of the database's membership leaves for the server (#1341,
//! #1440).
//!
//! Adding, replacing or removing an object can change others: a Pulse
//! Converter whose Input_Reference names it is judged again
//! (`input_references.rs`), and its Reliability can change with the verdict.
//! The database makes that change at once, under the caller's exclusive
//! access. Adding one can also let a Schedule's refused reference to it
//! through, but the Schedule's writes to its targets are the server's
//! (`crate::traits::BACnetObject::retry_refusals_naming` builds them). So the
//! database queues the objects that changed and the Schedules owed a retry,
//! and calls the waker the server installed. The server takes the queue
//! under its next database guard, makes the retries and fans COV out: at
//! once after a CreateObject or DeleteObject, and from a background task
//! after an application's own `add` or `remove`.
//!
//! The queue holds each entry once, and a retry only while a Schedule holds
//! a refusal naming the object. Without a server nothing takes it, so it
//! stays bounded by the objects and refusals that can produce entries.

use crate::prelude::*;
use alloc::sync::Arc;

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

use super::ObjectDatabase;

/// Called when a change of membership queues work, so a server can take it
/// (`ObjectDatabase::take_membership_work_internal`). It runs under the
/// caller's exclusive access to the database, so it must not wait for that
/// access itself.
#[doc(hidden)]
pub type MembershipWaker = dyn Fn() + Send + Sync;

/// The queue behind [`MembershipWork`], and the waker to call when it grows.
#[derive(Default)]
pub(super) struct MembershipQueue {
    changed: Vec<ObjectIdentifier>,
    schedule_retries: Vec<(ObjectIdentifier, ObjectIdentifier)>,
    waker: Option<Arc<MembershipWaker>>,
}

/// What changes of membership left for the server since it last took them.
#[doc(hidden)]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MembershipWork {
    /// Objects whose readable state the changes altered, each owing COV:
    /// Pulse Converters whose Reliability the check of their Input_Reference
    /// changed.
    pub changed: Vec<ObjectIdentifier>,
    /// `(schedule, object)` pairs: a Schedule held a refused reference to
    /// the object when it was added, so the server owes it the
    /// [`retry_refusals_naming`](crate::traits::BACnetObject::retry_refusals_naming)
    /// write for that object.
    pub schedule_retries: Vec<(ObjectIdentifier, ObjectIdentifier)>,
}

impl MembershipWork {
    /// Whether there is nothing to do.
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.schedule_retries.is_empty()
    }
}

impl ObjectDatabase {
    /// Install the callback a change of membership that queues work calls,
    /// or remove it with `None`. The bundled server installs one at start.
    #[doc(hidden)]
    pub fn set_membership_waker_internal(&mut self, waker: Option<Arc<MembershipWaker>>) {
        self.membership.waker = waker;
    }

    /// Take the work changes of membership queued since the last call.
    #[doc(hidden)]
    pub fn take_membership_work_internal(&mut self) -> MembershipWork {
        MembershipWork {
            changed: core::mem::take(&mut self.membership.changed),
            schedule_retries: core::mem::take(&mut self.membership.schedule_retries),
        }
    }

    /// Do what adding (`added`), replacing or removing `oid` asks of the
    /// other objects, and queue what the server owes for it. The object added
    /// is judged too, but owes nothing: it starts from what it holds.
    pub(super) fn membership_changed(&mut self, oid: ObjectIdentifier, added: bool) {
        let mut changed = self.recheck_input_references(oid);
        changed.retain(|converter| *converter != oid);
        let retries = if added {
            self.schedules_refusing(oid)
        } else {
            Vec::new()
        };
        if changed.is_empty() && retries.is_empty() {
            return;
        }
        for oid in changed {
            if !self.membership.changed.contains(&oid) {
                self.membership.changed.push(oid);
            }
        }
        for retry in retries.into_iter().map(|schedule| (schedule, oid)) {
            if !self.membership.schedule_retries.contains(&retry) {
                self.membership.schedule_retries.push(retry);
            }
        }
        if let Some(waker) = &self.membership.waker {
            waker();
        }
    }

    /// The Schedules holding a refused reference to `oid` that they would
    /// offer their value again (#1440); found by asking each Schedule, not
    /// by evaluating it.
    fn schedules_refusing(&self, oid: ObjectIdentifier) -> Vec<ObjectIdentifier> {
        self.type_index
            .get(&ObjectType::SCHEDULE)
            .into_iter()
            .flatten()
            .copied()
            .filter(|schedule| {
                self.objects
                    .get(schedule)
                    .is_some_and(|object| object.retry_refusals_naming(oid).is_some())
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "membership_tests.rs"]
mod tests;
