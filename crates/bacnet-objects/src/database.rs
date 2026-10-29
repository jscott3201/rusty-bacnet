//! ObjectDatabase — stores and retrieves BACnet objects by identifier.

use crate::collections::{HashMap, HashSet};
use crate::prelude::*;
use alloc::sync::Arc;

use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use crate::clock::{ClockFrame, ClockReader};
use crate::event_enrollment::EventEnrollmentMonitoredSource;
use crate::traits::{BACnetObject, DeadlineWaker, MonotonicClock};

mod averaging_sampling;
mod event_algorithm_inhibit;
mod event_log;
mod input_references;
mod local_device;
mod membership;
mod network_port;
mod trend_poll;
pub use local_device::LocalDevice;
#[doc(hidden)]
pub use membership::{MembershipWaker, MembershipWork};
use trend_poll::TrendPollSchedule;

/// A collection of BACnet objects, keyed by ObjectIdentifier.
///
/// Enforces Object_Name uniqueness within a device.
/// Maintains secondary indexes for O(1) name lookup and O(1) type lookup.
pub struct ObjectDatabase {
    audit_owner: Option<alloc::sync::Weak<AuditOwnership>>,
    network_port: Option<network_port::NetworkPortRegistration>,
    objects: HashMap<ObjectIdentifier, Box<dyn BACnetObject>>,
    trend_poll: TrendPollSchedule,
    /// Shared Device clock reader. `None` is an explicit clockless database.
    clock: Option<Arc<dyn ClockReader>>,
    monotonic_clock: Option<Arc<MonotonicClock>>,
    /// Bound into every object with the monotonic clock (#1384).
    deadline_waker: Option<Arc<DeadlineWaker>>,
    /// Device-local EventNotification ordering source for clockless operation.
    event_sequence: Arc<EventSequence>,
    /// Reverse index: object name → ObjectIdentifier for uniqueness enforcement.
    name_index: HashMap<String, ObjectIdentifier>,
    /// Type index: object type → set of ObjectIdentifiers for fast enumeration.
    type_index: HashMap<ObjectType, Vec<ObjectIdentifier>>,
    /// Event Enrollment objects whose private evaluator state must be reset
    /// before it can be used again.
    invalid_enrollment_eval_state: HashSet<ObjectIdentifier>,
    /// Source ownership for custom Event Enrollment objects that implement
    /// evaluation state but not the optional object-owned source channel.
    enrollment_eval_sources: HashMap<ObjectIdentifier, EventEnrollmentMonitoredSource>,
    /// What adding and removing objects left for the server (#1341, #1440).
    membership: membership::MembershipQueue,
}

/// A non-consuming reservation of the database-local event sequence source.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventSequenceReservation {
    number: u16,
}

impl EventSequenceReservation {
    /// The sequence number selected by this reservation.
    #[doc(hidden)]
    pub fn number(self) -> u16 {
        self.number
    }
}

mod audit_ownership;
mod event_sequence;
pub use audit_ownership::AuditOwnership;
pub use event_sequence::EventSequence;

impl Default for ObjectDatabase {
    fn default() -> Self {
        Self::new()
    }
}

impl ObjectDatabase {
    /// Create an empty database.
    pub fn new() -> Self {
        Self {
            objects: HashMap::new(),
            trend_poll: TrendPollSchedule::default(),
            clock: None,
            monotonic_clock: None,
            deadline_waker: None,
            event_sequence: Arc::default(),
            audit_owner: None,
            network_port: None,
            name_index: HashMap::new(),
            type_index: HashMap::new(),
            invalid_enrollment_eval_state: HashSet::new(),
            enrollment_eval_sources: HashMap::new(),
            membership: membership::MembershipQueue::default(),
        }
    }

    /// Add an object to the database.
    ///
    /// Returns PROPERTY / DUPLICATE_NAME if another object already has the
    /// same `object_name()`, as
    /// [`check_name_available`](Self::check_name_available) does.
    /// Replacing an object with the same OID is allowed unless an installed
    /// Audit runtime protects its membership. Protection is checked before any binding.
    ///
    /// Once the object is in, every Pulse Converter whose Input_Reference
    /// names it, or the object itself when it is one, has the reference
    /// judged again (`ObjectDatabase::check_input_reference`); one whose
    /// Reliability changes is queued for the server's COV fanout. A Schedule
    /// holding a refused reference to it is queued for the server to retry
    /// that reference (#1440).
    pub fn add(&mut self, mut object: Box<dyn BACnetObject>) -> Result<(), Error> {
        self.check_network_port_membership(&object.object_identifier())?;
        self.check_audit_membership(&object.object_identifier(), true)?;
        object.bind_clock_internal(self.clock.clone());
        object.bind_monotonic_clock_internal(self.monotonic_clock.clone());
        object.bind_deadline_waker_internal(self.deadline_waker.clone());
        let oid = object.object_identifier();
        let name = object.object_name().to_string();

        // Check for name collision with a *different* object
        self.check_name_available(&oid, &name)?;

        // If replacing an existing object, remove its old name from the index
        // and invalidate state owned by enrollments that monitor it.
        if let Some(old) = self.objects.get(&oid) {
            if let Some(reporter) = old.audit_reporter_internal() {
                reporter.status_internal().configuration_changed();
            }
            let old_name = old.object_name().to_string();
            self.name_index.remove(&old_name);
            self.invalidate_enrollments_monitoring(&oid);
        }

        self.name_index.insert(name, oid);
        let is_new = !self.objects.contains_key(&oid);
        self.enrollment_eval_sources.remove(&oid);
        self.trend_poll.retire(&oid);
        self.objects.insert(oid, object);
        self.audit_membership_changed(oid, true);
        if is_new {
            self.type_index
                .entry(oid.object_type())
                .or_default()
                .push(oid);
        }
        self.membership_changed(oid, true);
        Ok(())
    }

    /// Look up an object by its name. O(1) via the name index.
    pub fn find_by_name(&self, name: &str) -> Option<&dyn BACnetObject> {
        let oid = self.name_index.get(name)?;
        self.objects.get(oid).map(|o| o.as_ref())
    }

    /// Check whether `new_name` is available for object `oid`.
    ///
    /// Returns `Ok(())` if the name is unused or already belongs to `oid`,
    /// and PROPERTY / DUPLICATE_NAME if another object owns it. Clause 18.3
    /// lists that code under the PROPERTY class, and the WriteProperty and
    /// WritePropertyMultiple error tables (15.9.1.3.1, 15.10.1.3.1) give the
    /// same pair; CreateObject's table (15.3.1.3.1) has no row for a name in
    /// use, so its initial values answer as WriteProperty does.
    pub fn check_name_available(
        &self,
        oid: &ObjectIdentifier,
        new_name: &str,
    ) -> Result<(), Error> {
        if let Some(&owner) = self.name_index.get(new_name)
            && owner != *oid
        {
            return Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::DUPLICATE_NAME,
            ));
        }
        Ok(())
    }

    /// Update the name index after a successful Object_Name write.
    ///
    /// Call this after `write_property(OBJECT_NAME, …)` succeeds.
    pub fn update_name_index(&mut self, oid: &ObjectIdentifier) {
        if let Some(obj) = self.objects.get(oid) {
            // Remove any old name mapping for this OID
            self.name_index.retain(|_, v| v != oid);
            // Insert the current name
            self.name_index.insert(obj.object_name().to_string(), *oid);
        }
    }

    /// Get a shared reference to an object by identifier.
    pub fn get(&self, oid: &ObjectIdentifier) -> Option<&dyn BACnetObject> {
        self.objects.get(oid).map(|o| o.as_ref())
    }

    /// Get a mutable reference to an object by identifier.
    pub fn get_mut(&mut self, oid: &ObjectIdentifier) -> Option<&mut (dyn BACnetObject + '_)> {
        if let Some(object) = self.objects.get_mut(oid) {
            Some(object.as_mut())
        } else {
            None
        }
    }

    /// Install an identity-preserving adapter through scoped structural access.
    ///
    /// The callback must preserve the object's identifier, name and type. It is
    /// responsible for allocating/validating before changing the slot. This hook
    /// neither rebinds clocks nor updates indexes, and does not provide rollback.
    /// Poll ownership is retired before invocation, even if the callback returns
    /// an error or unwinds after modifying the slot. Slot borrows cannot escape.
    /// An installed Audit runtime can reject protected members before the callback.
    ///
    /// ```compile_fail
    /// use bacnet_objects::{database::ObjectDatabase, traits::BACnetObject};
    /// use bacnet_types::primitives::ObjectIdentifier;
    /// fn escape<'a>(db: &'a mut ObjectDatabase, oid: &ObjectIdentifier)
    ///     -> Result<Option<&'a mut Box<dyn BACnetObject>>, bacnet_types::error::Error> {
    ///     db.with_object_adapter(oid, |slot| slot)
    /// }
    /// ```
    pub fn with_object_adapter<R, F>(
        &mut self,
        oid: &ObjectIdentifier,
        adapt: F,
    ) -> Result<Option<R>, Error>
    where
        F: for<'slot> FnOnce(&'slot mut Box<dyn BACnetObject>) -> R,
    {
        self.check_network_port_membership(oid)?;
        self.check_audit_membership(oid, false)?;
        self.trend_poll.retire(oid);
        Ok(self.objects.get_mut(oid).map(adapt))
    }

    /// Whether an Event Enrollment object's private evaluator state requires a
    /// successful reset before reuse.
    pub fn enrollment_eval_state_invalidated(&self, oid: &ObjectIdentifier) -> bool {
        self.invalid_enrollment_eval_state.contains(oid)
    }

    /// Mark or clear the reset requirement for Event Enrollment private state.
    pub fn set_enrollment_eval_state_invalidated(
        &mut self,
        oid: ObjectIdentifier,
        invalidated: bool,
    ) {
        if invalidated {
            self.invalid_enrollment_eval_state.insert(oid);
        } else {
            self.invalid_enrollment_eval_state.remove(&oid);
        }
    }

    /// Return database-owned monitored-source state for a custom Event
    /// Enrollment object.
    pub fn enrollment_eval_source(
        &self,
        oid: &ObjectIdentifier,
    ) -> Option<EventEnrollmentMonitoredSource> {
        self.enrollment_eval_sources.get(oid).copied()
    }

    /// Store database-owned monitored-source state for a custom Event
    /// Enrollment object.
    pub fn set_enrollment_eval_source(
        &mut self,
        oid: ObjectIdentifier,
        source: Option<EventEnrollmentMonitoredSource>,
    ) {
        if let Some(source) = source {
            self.enrollment_eval_sources.insert(oid, source);
        } else {
            self.enrollment_eval_sources.remove(&oid);
        }
    }

    fn invalidate_enrollments_monitoring(&mut self, monitored_oid: &ObjectIdentifier) {
        let affected = self
            .objects
            .iter()
            .filter_map(|(oid, object)| {
                let source = object
                    .enrollment_eval_source_internal()
                    .flatten()
                    .or_else(|| self.enrollment_eval_sources.get(oid).copied());
                source
                    .is_some_and(|source| source.0 == *monitored_oid)
                    .then_some(*oid)
            })
            .collect::<Vec<_>>();
        self.invalid_enrollment_eval_state.extend(affected);
    }

    /// Remove an object by identifier. Installed Audit membership protection
    /// returns an error before removing the object or changing indexes.
    /// Pulse Converters whose Input_Reference names a removed object have
    /// the reference judged again, as [`add`](Self::add) does.
    pub fn remove(
        &mut self,
        oid: &ObjectIdentifier,
    ) -> Result<Option<Box<dyn BACnetObject>>, Error> {
        self.check_network_port_membership(oid)?;
        self.check_audit_membership(oid, false)?;
        self.trend_poll.retire(oid);
        if self.objects.contains_key(oid) {
            self.invalidate_enrollments_monitoring(oid);
        }
        match self.objects.remove(oid) {
            Some(mut obj) => {
                if let Some(reporter) = obj.audit_reporter_internal() {
                    reporter.status_internal().configuration_changed();
                }
                self.enrollment_eval_sources.remove(oid);
                if obj.enrollment_eval_state_internal().is_some() {
                    if obj
                        .set_enrollment_eval_state_internal(Default::default())
                        .is_err()
                    {
                        self.invalid_enrollment_eval_state.insert(*oid);
                    } else {
                        self.invalid_enrollment_eval_state.remove(oid);
                    }
                    let _ = obj.set_enrollment_eval_source_internal(None);
                } else {
                    self.invalid_enrollment_eval_state.remove(oid);
                }
                self.name_index.remove(obj.object_name());
                if let Some(type_set) = self.type_index.get_mut(&oid.object_type()) {
                    type_set.retain(|o| o != oid);
                }
                self.audit_membership_changed(*oid, false);
                self.membership_changed(*oid, false);
                Ok(Some(obj))
            }
            _ => Ok(None),
        }
    }

    /// List all object identifiers in the database.
    pub fn list_objects(&self) -> Vec<ObjectIdentifier> {
        self.objects.keys().copied().collect()
    }

    /// Find all objects of a given type.
    ///
    /// Returns a `Vec` of `ObjectIdentifier`s whose object type matches `object_type`.
    /// Useful for WhoHas, object enumeration, and similar queries.
    pub fn find_by_type(&self, object_type: ObjectType) -> Vec<ObjectIdentifier> {
        self.type_index
            .get(&object_type)
            .cloned()
            .unwrap_or_default()
    }

    /// Iterate over all `(ObjectIdentifier, &dyn BACnetObject)` pairs.
    ///
    /// Avoids the double-lookup pattern of `list_objects()` followed by `get()`.
    pub fn iter_objects(&self) -> impl Iterator<Item = (ObjectIdentifier, &dyn BACnetObject)> {
        self.objects.iter().map(|(&oid, obj)| (oid, obj.as_ref()))
    }

    /// Bind one clock reader to the database and every object it contains.
    ///
    /// Devices added after this call receive the same reader in [`add`](Self::add).
    pub fn set_clock_reader(&mut self, clock: Option<Arc<dyn ClockReader>>) {
        self.clock = clock;
        for object in self.objects.values_mut() {
            object.bind_clock_internal(self.clock.clone());
        }
    }

    /// Bind one process-local monotonic source to every contained object.
    #[doc(hidden)]
    pub fn set_monotonic_clock_internal(&mut self, clock: Option<Arc<MonotonicClock>>) {
        self.trend_poll.clear();
        self.monotonic_clock = clock;
        for object in self.objects.values_mut() {
            object.bind_monotonic_clock_internal(self.monotonic_clock.clone());
        }
    }

    /// Bind the waker objects call when a write arms a monotonic deadline to
    /// every contained object, and to each one added later.
    #[doc(hidden)]
    pub fn set_deadline_waker_internal(&mut self, waker: Option<Arc<DeadlineWaker>>) {
        self.deadline_waker = waker;
        for object in self.objects.values_mut() {
            object.bind_deadline_waker_internal(self.deadline_waker.clone());
        }
    }

    /// Read one coherent sample from the database's shared clock.
    pub fn clock_frame(&self) -> Option<ClockFrame> {
        self.clock.as_ref()?.read_clock()
    }

    #[doc(hidden)]
    pub fn event_sequence_internal(&self) -> Arc<EventSequence> {
        Arc::clone(&self.event_sequence)
    }

    /// Consume the next Device-local EventNotification sequence number.
    ///
    /// The counter wraps modulo 65536 as required by the timestamp production.
    #[doc(hidden)]
    pub fn next_event_sequence_number(&mut self) -> u16 {
        let reservation = self.reserve_event_sequence_number();
        let number = reservation.number();
        let confirmed = self.confirm_event_sequence_number(reservation);
        debug_assert!(
            confirmed,
            "a reservation cannot become stale without mutation"
        );
        number
    }

    /// Reserve the current sequence number without consuming it.
    #[doc(hidden)]
    pub fn reserve_event_sequence_number(&self) -> EventSequenceReservation {
        EventSequenceReservation {
            number: self.event_sequence.current(),
        }
    }

    /// Consume an exact reservation if it is still current.
    #[doc(hidden)]
    pub fn confirm_event_sequence_number(&mut self, reservation: EventSequenceReservation) -> bool {
        self.event_sequence.confirm(reservation.number)
    }

    /// Visit every `(ObjectIdentifier, &mut dyn BACnetObject)` pair.
    ///
    /// Mutable counterpart to [`iter_objects`](Self::iter_objects), used by
    /// the intrinsic-reporting tick task to advance pending transitions. A
    /// callback is used (rather than a returned `impl Iterator`) so the
    /// `&mut dyn BACnetObject` borrow is tied to this call's lifetime.
    pub fn for_each_object_mut<F>(&mut self, mut f: F)
    where
        F: FnMut(ObjectIdentifier, &mut dyn BACnetObject),
    {
        for (&oid, obj) in self.objects.iter_mut() {
            f(oid, obj.as_mut());
        }
    }

    /// Number of objects in the database.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Whether the database is empty.
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod network_port_tests;
