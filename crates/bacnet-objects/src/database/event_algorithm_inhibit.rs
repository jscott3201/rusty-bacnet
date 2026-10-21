//! Following an Event_Algorithm_Inhibit_Ref (#1329).
//!
//! An intrinsic reporter, or an Event Enrollment (#1507), can't see the
//! object its reference names, so the database reads the property for it. The reference is a
//! BACnetObjectPropertyReference, which has no device member, so it only
//! ever names a property of this device; there is nothing remote to read.
//! The clauses that define the pair (Clause 12.2.31 and its counterparts)
//! expect a Boolean or a BinaryPV there, map ACTIVE to TRUE and INACTIVE to
//! FALSE, and take a property that doesn't exist as FALSE.
//!
//! A read only says Enumerated, not which enumeration, and an Event_State
//! of FAULT or a Reliability of NO_SENSOR is Enumerated 1 as well. So the
//! inhibit is TRUE only for a Boolean TRUE, or for an ACTIVE read from a
//! property [`holds_binary_pv`] knows as a BinaryPV: Present_Value,
//! Relinquish_Default, an element of Priority_Array, Alarm_Value and
//! Feedback_Value on the binary types that have them, and an Access
//! Credential's Credential_Status. Anything else, a property this device
//! can't read included, is FALSE.
//!
//! The bundled server follows the reference every time it evaluates the
//! object: the evaluation a write of the object runs, and the one-second
//! tick every intrinsic reporter gets. A change to the referenced property
//! therefore reaches Event_Algorithm_Inhibit, and the algorithm, within a
//! second, and a read of Event_Algorithm_Inhibit returns the value as of
//! the last evaluation. Both reads happen under the database guard the
//! evaluation already holds, so following takes no lock of its own.

use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{BinaryPV, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::ObjectDatabase;

impl ObjectDatabase {
    /// Read the property the object `oid`'s Event_Algorithm_Inhibit_Ref
    /// names and hand the value over as its Event_Algorithm_Inhibit; returns
    /// whether that changed. An object with no reference, one that doesn't
    /// report intrinsically, or one this database doesn't hold is left
    /// alone.
    pub fn follow_event_algorithm_inhibit(&mut self, oid: &ObjectIdentifier) -> bool {
        let Some(reference) = self
            .objects
            .get(oid)
            .and_then(|object| object.event_algorithm_inhibit_reference_internal())
        else {
            return false;
        };
        let inhibit = self.inhibit_value(&reference);
        self.objects
            .get_mut(oid)
            .is_some_and(|object| object.follow_event_algorithm_inhibit_internal(inhibit))
    }

    /// The Boolean the referenced property stands for; see the module
    /// documentation.
    fn inhibit_value(&self, reference: &BACnetObjectPropertyReference) -> bool {
        let property = PropertyIdentifier::from_raw(reference.property_identifier);
        let index = reference.property_array_index;
        let target = reference.object_identifier;
        let value = self
            .get(&target)
            .and_then(|object| object.read_property(property, index).ok());
        match value {
            Some(PropertyValue::Boolean(inhibit)) => inhibit,
            Some(PropertyValue::Enumerated(raw)) => {
                raw == BinaryPV::ACTIVE.to_raw()
                    && holds_binary_pv(target.object_type(), property, index)
            }
            _ => false,
        }
    }
}

/// Whether `property`, read at `index`, holds a BACnetBinaryPV on an object
/// of `object_type`: Tables 12-6, 12-8 and 12-10 for the binary types, and
/// Table 12-40 for an Access Credential's Credential_Status.
fn holds_binary_pv(
    object_type: ObjectType,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> bool {
    use PropertyIdentifier as P;
    let commanded = property == P::RELINQUISH_DEFAULT
        || (property == P::PRIORITY_ARRAY && index.is_some_and(|slot| slot > 0));
    match object_type {
        ObjectType::BINARY_INPUT => matches!(property, P::PRESENT_VALUE | P::ALARM_VALUE),
        ObjectType::BINARY_OUTPUT => {
            matches!(property, P::PRESENT_VALUE | P::FEEDBACK_VALUE) || commanded
        }
        ObjectType::BINARY_VALUE => {
            matches!(property, P::PRESENT_VALUE | P::ALARM_VALUE) || commanded
        }
        ObjectType::ACCESS_CREDENTIAL => property == P::CREDENTIAL_STATUS,
        _ => false,
    }
}

#[cfg(test)]
#[path = "event_algorithm_inhibit_tests.rs"]
mod tests;
