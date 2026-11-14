//! Command prioritization as far as a write needs it (Clause 19.2):
//! whether a property takes priorities at all ([`Commandability`]), and
//! what a write at a priority does to Present_Value, from the slots as read
//! ([`effect`]). Pure: the `bacnet` tool's write planner and
//! [`Device::preview_write`](crate::Device::preview_write) share it.

use bacnet_encoding::primitives::{
    decode_application_value, decode_priority_date_time, encode_property_value,
};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

use super::Priority;
use crate::format::{ValueFormat, decode_all};

/// Whether a property takes command priorities, as far as its object type
/// and identifier say with nothing read (Clause 19.2.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Commandability {
    /// The standard makes it commandable: Present_Value on the three
    /// output types (analog, binary, multi-state), the two lighting output
    /// types, Access Door, and Channel, which keeps no Priority_Array and
    /// hands the priority to its members (Clause 19.2.1.6).
    Always,
    /// It is commandable only if the object has a Priority_Array, which
    /// only a read can tell: the Present_Value of the value objects (Analog,
    /// Binary and Multi-state Value, and the CharacterString, Integer, Date
    /// and other value types of Clause 19.2.1.1); any property of an object
    /// type this release doesn't know or a vendor defines; and a
    /// proprietary property, which a vendor may make commandable.
    Optional,
    /// Never commandable: any other property the standard defines, on an
    /// object type it defines.
    Never,
}

impl Commandability {
    /// Whether `property` of an object of `object_type` takes priorities.
    pub fn of(object_type: ObjectType, property: PropertyIdentifier) -> Self {
        type O = ObjectType;
        // 0 to 511 and from 4,194,304 up are the standard's; the range
        // between is the vendors'.
        if (512..4_194_304).contains(&property.to_raw()) {
            return Self::Optional;
        }
        // Past Color Temperature, the newest type the 2020 edition and its
        // addenda define: a later standard type, or a vendor's.
        if object_type.to_raw() > O::COLOR_TEMPERATURE.to_raw() {
            return Self::Optional;
        }
        if property != PropertyIdentifier::PRESENT_VALUE {
            return Self::Never;
        }
        let always = [
            O::ACCESS_DOOR,
            O::ANALOG_OUTPUT,
            O::BINARY_OUTPUT,
            O::MULTI_STATE_OUTPUT,
            O::LIGHTING_OUTPUT,
            O::BINARY_LIGHTING_OUTPUT,
            O::CHANNEL,
        ];
        let optional = [
            O::ANALOG_VALUE,
            O::BINARY_VALUE,
            O::MULTI_STATE_VALUE,
            O::BITSTRING_VALUE,
            O::CHARACTERSTRING_VALUE,
            O::DATE_VALUE,
            O::DATEPATTERN_VALUE,
            O::DATETIME_VALUE,
            O::DATETIMEPATTERN_VALUE,
            O::INTEGER_VALUE,
            O::LARGE_ANALOG_VALUE,
            O::OCTETSTRING_VALUE,
            O::POSITIVE_INTEGER_VALUE,
            O::TIME_VALUE,
            O::TIMEPATTERN_VALUE,
        ];
        if always.contains(&object_type) {
            Self::Always
        } else if optional.contains(&object_type) {
            Self::Optional
        } else {
            Self::Never
        }
    }
}

/// Whether an object of `object_type` keeps its commands in a
/// Priority_Array beside its Present_Value: every type whose Present_Value
/// may be commandable except a Channel, which passes them on.
pub(crate) fn keeps_priorities(object_type: ObjectType) -> bool {
    object_type != ObjectType::CHANNEL
        && Commandability::of(object_type, PropertyIdentifier::PRESENT_VALUE)
            != Commandability::Never
}

/// Who holds the slot a write goes to, as far as a read can say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Occupancy {
    /// The property takes no priority: the object has no Priority_Array,
    /// or the property isn't its Present_Value.
    NotCommandable,
    /// The slot is NULL: no one commands there.
    Empty,
    /// The slot holds a command, which the write replaces without telling
    /// whoever sent it (Clause 19.2.1.5).
    Held,
    /// What the slots hold isn't known: a vendor's own commandable
    /// property, whose priorities go by other names, or a Priority_Array
    /// the device wouldn't give.
    Unknown,
}

/// What a write does to Present_Value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// It changes, `from` its value now `to` its value after, as text.
    Changes {
        /// Now.
        from: String,
        /// After the write.
        to: String,
    },
    /// It stays, held by a lower-numbered (stronger) priority when there
    /// is one (Clause 19.2.1).
    Unchanged {
        /// The slot that keeps control.
        held_by: Option<Priority>,
    },
    /// The write is to another property.
    NotPresentValue,
    /// Not known: the slots that decide it couldn't be read.
    Unknown,
}

/// The sixteen commands a Priority_Array's octets hold, in order, each as
/// the value a Present_Value write carries: NULL for an empty slot, and a
/// DateTime Value's date-time choice (context tag 1, #2098) as its Date
/// and Time. `None` unless they decode as sixteen values.
pub fn commands(priority_array: &[u8]) -> Option<Vec<PropertyValue>> {
    let values = decode_all(priority_array)?;
    (values.len() == Priority::COUNT).then(|| values.into_iter().map(command).collect())
}

/// One slot as the value a Present_Value write carries.
fn command(slot: PropertyValue) -> PropertyValue {
    if let PropertyValue::ApplicationData(octets) = &slot
        && let Ok(((date, time), end)) = decode_priority_date_time(octets, 0)
        && end == octets.len()
    {
        return PropertyValue::List(vec![PropertyValue::Date(date), PropertyValue::Time(time)]);
    }
    slot
}

/// The octets of each value in `data`, in order: a Priority_Array's slots
/// as the device sent them. `None` when any part doesn't decode.
pub(crate) fn elements(data: &[u8]) -> Option<Vec<&[u8]>> {
    let mut offset = 0;
    let mut elements = Vec::new();
    while offset < data.len() {
        let (_, next) = decode_application_value(data, offset).ok()?;
        elements.push(&data[offset..next]);
        offset = next;
    }
    Some(elements)
}

/// The value a write's octets carry, as a slot holds it: one value, or a
/// BACnetDateTime's Date and Time together.
fn carried(octets: &[u8]) -> PropertyValue {
    match decode_all(octets) {
        Some(values) if values.len() == 1 => {
            values.into_iter().next().unwrap_or(PropertyValue::Null)
        }
        Some(values) if !values.is_empty() => PropertyValue::List(values),
        _ => PropertyValue::Null,
    }
}

/// `value` as `format` writes a Present_Value.
fn text(format: &ValueFormat<'_>, value: &PropertyValue) -> String {
    let mut buf = BytesMut::new();
    match encode_property_value(&mut buf, value) {
        Ok(()) => format.value(PropertyIdentifier::PRESENT_VALUE, &buf),
        Err(error) => format!("cannot encode the value: {error}"),
    }
}

/// The slot in control of `slots` and Present_Value's text then, falling
/// back to `relinquish_default` (its text) when every slot is NULL.
fn in_control(
    format: &ValueFormat<'_>,
    slots: &[PropertyValue],
    relinquish_default: Option<&str>,
) -> (Option<Priority>, String) {
    match slots.iter().position(|v| *v != PropertyValue::Null) {
        Some(at) => (Priority::at_index(at), text(format, &slots[at])),
        None => (None, relinquish_default.unwrap_or("?").to_string()),
    }
}

/// What a write of `value` (its octets; NULL relinquishes) at `priority`
/// does to Present_Value, given the sixteen `slots` as [`commands`] reads
/// them and Relinquish_Default's text: it changes when the text in control
/// after differs from the text now; otherwise it stays, and `held_by` names
/// the stronger slot that keeps control, if one does. Texts are `format`'s.
pub fn effect(
    format: &ValueFormat<'_>,
    slots: &[PropertyValue],
    priority: Priority,
    value: &[u8],
    relinquish_default: Option<&str>,
) -> Effect {
    let (held_by, now) = in_control(format, slots, relinquish_default);
    let mut after = slots.to_vec();
    if let Some(slot) = after.get_mut(priority.index()) {
        *slot = carried(value);
    }
    let (_, then) = in_control(format, &after, relinquish_default);
    if now == then {
        Effect::Unchanged {
            held_by: held_by.filter(|slot| *slot < priority),
        }
    } else {
        Effect::Changes {
            from: now,
            to: then,
        }
    }
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;
