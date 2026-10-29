//! Judging a Pulse Converter's Input_Reference against the database (Clause
//! 12.23.9, #1341).
//!
//! The object can't see the object its reference names, so the database
//! looks: the reference is usable when it names another object here, at a
//! property that reads as an Unsigned or an INTEGER. Anything else, a
//! missing object or property included, is a CONFIGURATION_ERROR. A Pulse
//! Converter's own properties are no input to it, an index goes only on an
//! array, and index 0, an array's size, counts no pulses. A Priority_Array
//! slot is judged by the datatype the object is commanded in, which its
//! Relinquish_Default carries, so a slot that is relinquished for now (NULL)
//! is usable but has nothing to count. An unset reference is no fault.
//!
//! The verdict is taken when the reference is written (the server calls
//! [`ObjectDatabase::check_input_reference`] as the write commits), when the
//! Pulse Converter is added, and whenever an object a reference names is
//! added, replaced or removed (`ObjectDatabase::add` and `remove`).
//! [`ObjectDatabase::count_pulse_inputs`] judges every converter again on
//! each pass as it reads the property for the count (Clause 12.23.14), so a
//! reference changed past the server's write paths, or a named property
//! whose datatype changes, is caught there too.

use crate::prelude::*;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::ObjectDatabase;
use crate::accumulator::InputReading;
use crate::traits::BACnetObject;

/// What a Pulse Converter's Input_Reference gives it.
enum InputSource {
    /// Nothing it can count from: a CONFIGURATION_ERROR.
    Unusable,
    /// A usable property with nothing to count right now: a relinquished
    /// Priority_Array slot.
    Idle,
    /// A reading to count from.
    Reading(InputReading),
}

impl ObjectDatabase {
    /// Judge the Input_Reference of the Pulse Converter `oid` against this
    /// database and hand it the verdict; returns whether its Reliability
    /// changed, so the caller owes COV for it. Any other object, or one this
    /// database doesn't hold, is left alone.
    ///
    /// The bundled server calls this as a write of Input_Reference commits,
    /// over WriteProperty, WritePropertyMultiple or the local write path;
    /// adding and removing objects calls it for the converters concerned.
    pub fn check_input_reference(&mut self, oid: &ObjectIdentifier) -> bool {
        let Some((usable, _)) = self.judge(oid) else {
            return false;
        };
        self.objects
            .get_mut(oid)
            .is_some_and(|object| object.set_input_usable_internal(usable))
    }

    /// Judge again every Pulse Converter whose Input_Reference names
    /// `changed`, and `changed` itself when it is one, after it was added,
    /// replaced or removed; returns those whose Reliability changed.
    pub(super) fn recheck_input_references(
        &mut self,
        changed: ObjectIdentifier,
    ) -> Vec<ObjectIdentifier> {
        let concerned: Vec<_> = self
            .type_index
            .get(&ObjectType::PULSE_CONVERTER)
            .into_iter()
            .flatten()
            .copied()
            .filter(|converter| {
                *converter == changed
                    || self
                        .objects
                        .get(converter)
                        .and_then(|object| object.input_reference_internal().flatten())
                        .is_some_and(|reference| reference.object_identifier == changed)
            })
            .collect();
        concerned
            .into_iter()
            .filter(|converter| self.check_input_reference(converter))
            .collect()
    }

    /// Judge each Pulse Converter's Input_Reference again and hand over a
    /// reading of the property it names, so the converter counts its
    /// increase into Count (Clause 12.23.14); returns the converters whose
    /// Reliability or Count changed, for COV. A converter with nothing to
    /// read gets `None`, which drops its baseline.
    ///
    /// The bundled server runs this on every wake of its monotonic operation
    /// task, so at least once a second. Reads go through
    /// [`BACnetObject::read_property`], so they must stay bounded, as for
    /// Averaging sampling.
    pub fn count_pulse_inputs(&mut self) -> Vec<ObjectIdentifier> {
        let converters = self.find_by_type(ObjectType::PULSE_CONVERTER);
        let mut changed = Vec::new();
        for oid in converters {
            let Some((usable, reading)) = self.judge(&oid) else {
                continue;
            };
            let Some(object) = self.objects.get_mut(&oid) else {
                continue;
            };
            let judged = object.set_input_usable_internal(usable);
            let counted = object.take_input_reading_internal(reading);
            if judged || counted {
                changed.push(oid);
            }
        }
        changed
    }

    /// The verdict on the Pulse Converter `oid`'s Input_Reference and what
    /// it reads now; `None` for an object that counts no referenced input.
    fn judge(&self, oid: &ObjectIdentifier) -> Option<(bool, Option<InputReading>)> {
        let reference = self.objects.get(oid)?.input_reference_internal()?;
        Some(
            match reference.map(|reference| self.input_source(*oid, reference)) {
                None | Some(InputSource::Idle) => (true, None),
                Some(InputSource::Unusable) => (false, None),
                Some(InputSource::Reading(reading)) => (true, Some(reading)),
            },
        )
    }

    /// What the converter `converter` gets from `reference`; see the module
    /// documentation for the rules.
    fn input_source(
        &self,
        converter: ObjectIdentifier,
        reference: &BACnetObjectPropertyReference,
    ) -> InputSource {
        if reference.object_identifier == converter {
            return InputSource::Unusable;
        }
        let Some(target) = self.get(&reference.object_identifier) else {
            return InputSource::Unusable;
        };
        let property = PropertyIdentifier::from_raw(reference.property_identifier);
        let index = reference.property_array_index;
        match index {
            Some(0) => return InputSource::Unusable,
            Some(_) if !target.is_array_property(property) => return InputSource::Unusable,
            _ => {}
        }
        let slot = property == PropertyIdentifier::PRIORITY_ARRAY && index.is_some();
        if slot
            && integer(target.read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)).is_none()
        {
            return InputSource::Unusable;
        }
        let value = target.read_property(property, index);
        if slot && matches!(value, Ok(PropertyValue::Null)) {
            return InputSource::Idle;
        }
        match integer(value) {
            Some(value) => InputSource::Reading(InputReading {
                value,
                wraps_after: wrap_bound(target, property, index),
            }),
            None => InputSource::Unusable,
        }
    }
}

/// An Unsigned or INTEGER read, widened.
fn integer(read: Result<PropertyValue, bacnet_types::error::Error>) -> Option<i128> {
    match read {
        Ok(PropertyValue::Unsigned(value)) => Some(i128::from(value)),
        Ok(PropertyValue::Signed(value)) => Some(i128::from(value)),
        _ => None,
    }
}

/// The value an Accumulator's Present_Value wraps after, its Max_Pres_Value
/// (Clause 12.61.4); `None` for any other property.
fn wrap_bound(
    target: &dyn BACnetObject,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> Option<u64> {
    let counter = target.object_identifier().object_type() == ObjectType::ACCUMULATOR
        && property == PropertyIdentifier::PRESENT_VALUE
        && index.is_none();
    match target.read_property(PropertyIdentifier::MAX_PRES_VALUE, None) {
        Ok(PropertyValue::Unsigned(max)) if counter => Some(max),
        _ => None,
    }
}

#[cfg(test)]
#[path = "input_references_tests.rs"]
mod tests;
