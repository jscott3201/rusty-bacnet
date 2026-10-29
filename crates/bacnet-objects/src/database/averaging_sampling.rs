//! Server-taken Averaging samples (Clauses 12.5.13 to 12.5.15, #1144). Each
//! object keeps its own schedule; the database reads the property a due
//! sample names, because only it can reach the other object.

use crate::prelude::*;
use core::time::Duration;

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;

use super::ObjectDatabase;

impl ObjectDatabase {
    /// Take every Averaging sample due at the monotonic instant `now` and
    /// return the objects that recorded one, for COV processing.
    ///
    /// The caller must hold exclusive database access for the whole call. A
    /// due object names the local property it averages
    /// (`take_due_averaging_sample_internal`); this reads it and records the
    /// value. An attempt that yields nothing usable is recorded as a miss,
    /// which counts toward Attempted_Samples but not Valid_Samples: a
    /// referenced object or property that doesn't exist, an array index on a
    /// property that isn't an array, any other read error, and a value the
    /// object refuses to average (a datatype outside the five Clause 12.5
    /// accepts, or a REAL that isn't finite).
    ///
    /// Without a bound monotonic clock nothing is ever due. Reads go through
    /// [`BACnetObject::read_property`](crate::traits::BACnetObject::read_property),
    /// so they must stay bounded, as for Trend Log polling.
    pub fn sample_due_averaging_objects(&mut self, now: Duration) -> Vec<ObjectIdentifier> {
        let mut due = Vec::new();
        self.for_each_object_mut(|oid, object| {
            if let Some(reference) = object.take_due_averaging_sample_internal(now) {
                due.push((oid, reference));
            }
        });
        let mut sampled = Vec::with_capacity(due.len());
        for (oid, reference) in due {
            let property = PropertyIdentifier::from_raw(reference.property_identifier);
            let index = reference.property_array_index;
            let value = self
                .get(&reference.object_identifier)
                // An index on anything but an array fails, as ReadProperty's
                // gate makes it; object read arms may ignore the index.
                .filter(|target| index.is_none() || target.is_array_property(property))
                .and_then(|target| target.read_property(property, index).ok());
            let Some(object) = self.get_mut(&oid) else {
                continue;
            };
            let recorded = value
                .and_then(|value| object.add_averaging_sample_internal(Some(value)).ok())
                .is_some()
                || object.add_averaging_sample_internal(None).is_ok();
            if recorded {
                sampled.push(oid);
            }
        }
        sampled
    }
}

#[cfg(test)]
#[path = "averaging_sampling_tests.rs"]
mod tests;
