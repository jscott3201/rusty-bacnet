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
/// Some of these every object of the type has, and one the device can't
/// give is a [`problem`](super::PointRecord::problems) whatever its error:
/// Object_Name; the Units of the types listed with it; a multi-state
/// object's Number_Of_States; the Relinquish_Default of a type whose
/// Present_Value is always commandable; and the Present_Value and
/// Status_Flags of the types whose tables require them: the analog,
/// binary and multi-state inputs, outputs and values, the twelve value
/// types from CharacterString Value to Date Pattern Value, Access Door,
/// both lighting outputs, Accumulator, Pulse Converter, Loop, the life
/// safety types, Channel, Schedule, Load Control, Timer, Staging, Global
/// Group and Credential Data Input (Present_Value alone for Group, Color
/// and Color Temperature). Any other is optional: an object that lacks it (the
/// device answers that the property is unknown) reads as `None` with no
/// problem.
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
/// its error. From the conformance codes of the object types' property
/// tables in Clause 12, where a row is required (read-only or writable)
/// with no condition attached:
///
/// - Object_Name, in every table;
/// - Units, in the tables of [`has_units`]'s types;
/// - Number_Of_States, in the multi-state tables (Clauses 12.18 to 12.20);
/// - Relinquish_Default, in the tables of the types whose Present_Value is
///   always commandable and kept in a Priority_Array: Analog, Binary and
///   Multi-state Output (Clauses 12.3, 12.7, 12.19), Access Door (Table
///   12-30), Lighting Output (Table 12-64) and Binary Lighting Output
///   (Table 12-69). In the value types' tables it is conditional on the
///   object being commandable, so it stays optional there;
/// - Present_Value, in the tables of [`has_value`]'s types, and
///   Status_Flags, in those of [`has_flags`]'s.
pub(super) fn required(object_type: ObjectType, property: P) -> bool {
    match property {
        P::OBJECT_NAME => true,
        P::UNITS => has_units(object_type),
        P::NUMBER_OF_STATES => is_multistate(object_type),
        P::RELINQUISH_DEFAULT => {
            Commandability::of(object_type, P::PRESENT_VALUE) == Commandability::Always
                && crate::write::keeps_priorities(object_type)
        }
        P::PRESENT_VALUE => has_value(object_type),
        P::STATUS_FLAGS => has_flags(object_type),
        _ => false,
    }
}

/// The types whose tables require Present_Value, as far as this list was
/// checked against them: those of [`has_flags`], and three whose tables
/// have no Status_Flags: Group (Table 12-17) and, from Addendum
/// 135-2020ca, Color and Color Temperature (its Tables 12-X and 12-Y). A
/// type not listed is treated as optional.
fn has_value(object_type: ObjectType) -> bool {
    type O = ObjectType;
    has_flags(object_type) || [O::GROUP, O::COLOR, O::COLOR_TEMPERATURE].contains(&object_type)
}

/// The types whose tables require both Present_Value and Status_Flags, as
/// far as this list was checked against them: the analog, binary and
/// multi-state inputs, outputs and values (Clauses 12.2 to 12.4, 12.6 to
/// 12.8 and 12.18 to 12.20), Life Safety Point and Zone (Tables 12-18 and
/// 12-19), Loop (Table 12-20), Pulse Converter (Table 12-27), Schedule
/// (Table 12-28), Access Door (Table 12-30), Load Control (Table 12-32),
/// Credential Data Input (Table 12-43), the value types of Clauses 12.37
/// to 12.48 (Table 12-44 and the tables shaped like it), Global Group
/// (Table 12-57), Channel (Table 12-62), Lighting Output (Table 12-64),
/// Binary Lighting Output (Table 12-69), Timer (Table 12-75), Accumulator
/// (Table 12-79) and Staging (Table 12-80). A type not listed is treated
/// as optional.
fn has_flags(object_type: ObjectType) -> bool {
    type O = ObjectType;
    is_binary(object_type)
        || is_multistate(object_type)
        || [
            O::ANALOG_INPUT,
            O::ANALOG_OUTPUT,
            O::ANALOG_VALUE,
            O::LIFE_SAFETY_POINT,
            O::LIFE_SAFETY_ZONE,
            O::LOOP,
            O::PULSE_CONVERTER,
            O::SCHEDULE,
            O::ACCESS_DOOR,
            O::LOAD_CONTROL,
            O::CREDENTIAL_DATA_INPUT,
            O::CHARACTERSTRING_VALUE,
            O::DATETIME_VALUE,
            O::LARGE_ANALOG_VALUE,
            O::BITSTRING_VALUE,
            O::OCTETSTRING_VALUE,
            O::TIME_VALUE,
            O::INTEGER_VALUE,
            O::POSITIVE_INTEGER_VALUE,
            O::DATE_VALUE,
            O::DATETIMEPATTERN_VALUE,
            O::TIMEPATTERN_VALUE,
            O::DATEPATTERN_VALUE,
            O::GLOBAL_GROUP,
            O::CHANNEL,
            O::LIGHTING_OUTPUT,
            O::BINARY_LIGHTING_OUTPUT,
            O::TIMER,
            O::ACCUMULATOR,
            O::STAGING,
        ]
        .contains(&object_type)
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
