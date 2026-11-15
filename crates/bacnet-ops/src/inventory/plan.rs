//! Which properties an inventory reads of each object type: the one list.

use bacnet_types::enums::{ObjectType, PropertyIdentifier as P};

use super::InventoryOptions;
use crate::format::{is_binary, is_multistate};
use crate::write::Commandability;

/// The properties an inventory reads of an object of `object_type` under
/// `options`, in the order it asks for them:
///
/// | Object type | Always | With `settings` |
/// |---|---|---|
/// | every type, a vendor's or a later standard's included | Object_Name, Description | |
/// | Analog Input, Output and Value; Integer, Positive Integer and Large Analog Value | Units | COV_Increment, Min_Pres_Value, Max_Pres_Value |
/// | Accumulator, Pulse Converter | Units | |
/// | Binary Input, Output and Value | Active_Text, Inactive_Text | |
/// | Multi-state Input, Output and Value | Number_Of_States, State_Text | |
/// | a type whose Present_Value may be commandable ([`Commandability::Optional`]): the value types, and any type this release doesn't know | Priority_Array | Relinquish_Default |
/// | a type whose Present_Value always is ([`Commandability::Always`]), except Channel | | Relinquish_Default |
///
/// With `values`, every object's Present_Value and Status_Flags follow. An
/// object type this release doesn't know therefore reads its name, its
/// description and whether it has a Priority_Array, and with values
/// whatever Present_Value and Status_Flags it has.
///
/// Object_Name, the Units of the types listed with it and a multi-state
/// object's Number_Of_States are properties every object of the type has:
/// one the device can't give is a [`problem`](super::PointRecord::problems)
/// whatever its error. Any other is optional: an object that lacks it
/// (the device answers that the property is unknown) reads as `None`
/// with no problem.
///
/// ```
/// use bacnet_ops::inventory::{InventoryOptions, properties};
/// use bacnet_types::enums::{ObjectType, PropertyIdentifier as P};
///
/// let plain = InventoryOptions::new();
/// assert_eq!(
///     properties(ObjectType::ANALOG_INPUT, &plain),
///     [P::OBJECT_NAME, P::DESCRIPTION, P::UNITS]
/// );
/// assert_eq!(
///     properties(ObjectType::BINARY_VALUE, &plain.clone().with_values(true)),
///     [
///         P::OBJECT_NAME,
///         P::DESCRIPTION,
///         P::ACTIVE_TEXT,
///         P::INACTIVE_TEXT,
///         P::PRIORITY_ARRAY,
///         P::PRESENT_VALUE,
///         P::STATUS_FLAGS,
///     ]
/// );
/// assert_eq!(properties(ObjectType::TREND_LOG, &plain).len(), 2);
/// ```
pub fn properties(object_type: ObjectType, options: &InventoryOptions) -> Vec<P> {
    let mut list = vec![P::OBJECT_NAME, P::DESCRIPTION];
    if has_units(object_type) {
        list.push(P::UNITS);
    }
    if is_binary(object_type) {
        list.extend([P::ACTIVE_TEXT, P::INACTIVE_TEXT]);
    }
    if is_multistate(object_type) {
        list.extend([P::NUMBER_OF_STATES, P::STATE_TEXT]);
    }
    let commandable = Commandability::of(object_type, P::PRESENT_VALUE);
    // Only a read can tell whether such an object takes commands.
    if commandable == Commandability::Optional {
        list.push(P::PRIORITY_ARRAY);
    }
    if options.settings {
        if has_range(object_type) {
            list.extend([P::COV_INCREMENT, P::MIN_PRES_VALUE, P::MAX_PRES_VALUE]);
        }
        if crate::write::keeps_priorities(object_type) {
            list.push(P::RELINQUISH_DEFAULT);
        }
    }
    if options.values {
        list.extend([P::PRESENT_VALUE, P::STATUS_FLAGS]);
    }
    list
}

/// Whether every object of `object_type` has `property`, among those
/// [`properties`] lists: one the device can't give is a problem whatever
/// its error.
pub(super) fn required(object_type: ObjectType, property: P) -> bool {
    property == P::OBJECT_NAME
        || (property == P::UNITS && has_units(object_type))
        || (property == P::NUMBER_OF_STATES && is_multistate(object_type))
}

/// The types whose objects all have Units.
fn has_units(object_type: ObjectType) -> bool {
    has_range(object_type)
        || [ObjectType::ACCUMULATOR, ObjectType::PULSE_CONVERTER].contains(&object_type)
}

/// The numeric types with a COV_Increment and a range of their own:
/// Min_Pres_Value and Max_Pres_Value.
fn has_range(object_type: ObjectType) -> bool {
    [
        ObjectType::ANALOG_INPUT,
        ObjectType::ANALOG_OUTPUT,
        ObjectType::ANALOG_VALUE,
        ObjectType::INTEGER_VALUE,
        ObjectType::POSITIVE_INTEGER_VALUE,
        ObjectType::LARGE_ANALOG_VALUE,
    ]
    .contains(&object_type)
}
