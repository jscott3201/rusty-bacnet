//! An inventory's records in the export's terms: a
//! [`Point`] for each object of the [`EXPORTED_TYPES`]
//! whose record holds what its type requires, and for any other the
//! reason it has none.
//!
//! Nothing here reads a device: [`Device::inventory`](crate::Device::inventory)
//! does, as it reads any inventory, and the export derives from its
//! [`PointRecord`]s. A record made with
//! [`inventory_options`](super::inventory_options) holds what the Turtle
//! uses; one made with [`point_list_options`](super::point_list_options)
//! holds the rest of a point list too, a Binary Input's or Output's
//! Polarity included.
//!
//! An object is skipped when its record lacks its Object_Name, the Units
//! of an analog object or an Integer Value, or a multi-state object's
//! Number_Of_States, the reason naming that property and what the device
//! said of it. An optional setting the record lacks is `None`: one the
//! device didn't give, or gave as a value of another datatype than the
//! property's, which the record lists among its problems (#2765); and a
//! Binary Value's Polarity, as that type has none. A value the record
//! holds is the point's as the device gave it, one the builder would
//! refuse included, which the record lists among its problems too.
//!
//! An input is [`Access::ReadOnly`] and an output
//! [`Access::Commandable`]. A value is commandable when it has a
//! Priority_Array ([`PointRecord::commandable`]) and [`Access::ReadOnly`]
//! otherwise, even when a peer may write it: a read can't tell a writable
//! value from a read-only one, only a write could.

use bacnet_types::enums::{BinaryPV, ObjectType, Polarity, PropertyIdentifier as P};
use bacnet_types::primitives::PropertyValue;

use super::{
    Access, Analog, Binary, DeviceInfo, EXPORTED_TYPES, Integer, Inventory, Model, MultiState,
    Point, PointKind, PointRecord, Skipped,
};
use crate::error::OpsError;
use crate::format::{Value, decode_all};
use crate::label::{kebab, object_label};

impl PointRecord {
    /// The record as a point of a point list
    /// (`bacnet_types::points::Point`), for an object of one of the ten
    /// [`EXPORTED_TYPES`]; `None` for any other type, and for a record
    /// that lacks what every object of its type has: its Object_Name, the
    /// Units of an analog object or an Integer Value, or a multi-state
    /// object's Number_Of_States ([`exported`] says which).
    ///
    /// The point's optional settings (COV_Increment, range, relinquish
    /// default, Polarity) are those the record holds, so `None` unless the
    /// inventory read [`settings`](crate::inventory::InventoryOptions::settings).
    pub fn to_point(&self) -> Option<Point> {
        exported(self).ok()
    }
}

/// `record` as the export's point, or why it has none
/// ([`PointRecord::to_point`]).
///
/// # Errors
///
/// [`Skipped`] for an object of a type the export doesn't map (no
/// property named), and for one whose record lacks a property every
/// object of its type has, named with the device's error for it.
pub fn exported(record: &PointRecord) -> Result<Point, Skipped> {
    let object = record.object;
    let Some((family, role)) = family_of(object.object_type()) else {
        return Err(Skipped {
            object,
            property: None,
            error: format!("{} objects aren't exported", kebab(object.object_type())),
        });
    };
    point(record, family, role).map_err(|(property, error)| Skipped {
        object,
        property: Some(property),
        error,
    })
}

impl From<&Inventory> for Model {
    /// The inventory's objects of the [`EXPORTED_TYPES`], each
    /// [`exported`], in its order; its other objects are left out. The
    /// Device's name is the summary's, exactly as sent, and its
    /// Description the Device object's own record's, when the inventory
    /// read it. An inventory that stopped early says where: the first
    /// object not read.
    fn from(inventory: &Inventory) -> Self {
        let mut device = DeviceInfo::new(inventory.device.device);
        device.name = inventory
            .device
            .value(P::OBJECT_NAME)
            .and_then(Value::as_str)
            .map(str::to_string);
        device.description = inventory
            .points
            .iter()
            .find(|point| point.object.object_type() == ObjectType::DEVICE)
            .and_then(|point| point.description.clone());
        let entries = inventory
            .points
            .iter()
            .filter(|point| EXPORTED_TYPES.contains(&point.object.object_type()))
            .map(exported)
            .collect();
        let stopped = inventory.stopped.as_ref().map(|error| {
            // The low-level client's own words, as the export has always
            // quoted them.
            let why = match error {
                OpsError::Unreachable { source, .. } => source.to_string(),
                error => error.to_string(),
            };
            match inventory.unread.first() {
                Some(object) => format!("{}: {why}", object_label(*object)),
                // A hand-built inventory that names no unread object.
                None => why,
            }
        });
        Self {
            device,
            entries,
            stopped,
        }
    }
}

/// The families the export maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Analog,
    Binary,
    MultiState,
    /// The Integer Value alone, a value.
    Integer,
}

/// Which of a family's types an object is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Input,
    Output,
    Value,
}

/// The family and role of `object_type`, when it is one of the
/// [`EXPORTED_TYPES`].
fn family_of(object_type: ObjectType) -> Option<(Family, Role)> {
    Some(match object_type {
        ObjectType::ANALOG_INPUT => (Family::Analog, Role::Input),
        ObjectType::ANALOG_OUTPUT => (Family::Analog, Role::Output),
        ObjectType::ANALOG_VALUE => (Family::Analog, Role::Value),
        ObjectType::BINARY_INPUT => (Family::Binary, Role::Input),
        ObjectType::BINARY_OUTPUT => (Family::Binary, Role::Output),
        ObjectType::BINARY_VALUE => (Family::Binary, Role::Value),
        ObjectType::MULTI_STATE_INPUT => (Family::MultiState, Role::Input),
        ObjectType::MULTI_STATE_OUTPUT => (Family::MultiState, Role::Output),
        ObjectType::MULTI_STATE_VALUE => (Family::MultiState, Role::Value),
        ObjectType::INTEGER_VALUE => (Family::Integer, Role::Value),
        _ => return None,
    })
}

/// One object's point from its record, or why it has none: a property
/// every object of its type has is missing, named with the reason.
fn point(record: &PointRecord, family: Family, role: Role) -> Result<Point, (P, String)> {
    let name = required(record, P::OBJECT_NAME, record.name.clone())?;
    let kind = match family {
        Family::Analog => {
            let units = required(record, P::UNITS, record.units)?;
            let mut analog = Analog::new(units, access(role, record, real));
            analog.cov_increment = setting(&record.cov_increment, real);
            analog.min_pres_value = setting(&record.min_present_value, real);
            analog.max_pres_value = setting(&record.max_present_value, real);
            PointKind::Analog(analog)
        }
        Family::Binary => {
            let mut binary = Binary::new(access(role, record, binary_pv));
            binary.active_text = record.state_texts.active.clone();
            binary.inactive_text = record.state_texts.inactive.clone();
            // A Binary Value has no Polarity, and a builder refuses one;
            // a hand-built record could still carry it.
            if role != Role::Value {
                binary.polarity = setting(&record.polarity, polarity);
            }
            PointKind::Binary(binary)
        }
        Family::MultiState => {
            let states = required(record, P::NUMBER_OF_STATES, record.number_of_states)?;
            let mut multi_state = MultiState::new(states, access(role, record, unsigned));
            multi_state.state_texts = record.state_texts.states.clone();
            PointKind::MultiState(multi_state)
        }
        Family::Integer => {
            let units = required(record, P::UNITS, record.units)?;
            let mut integer = Integer::new(units, access(role, record, signed));
            integer.cov_increment = setting(&record.cov_increment, unsigned64);
            integer.min_pres_value = setting(&record.min_present_value, signed);
            integer.max_pres_value = setting(&record.max_present_value, signed);
            PointKind::Integer(integer)
        }
    };
    let mut point = Point::new(record.object, name, kind);
    point.description = record.description.clone();
    Ok(point)
}

/// How Present_Value is written, as far as a read can tell: an input is
/// read-only, an output commandable, and a value commandable when it has a
/// Priority_Array. A commandable point's relinquish default is the one
/// read, when it was.
fn access<T>(role: Role, record: &PointRecord, decode: fn(&[u8]) -> Option<T>) -> Access<T> {
    let commandable = match role {
        Role::Input => false,
        Role::Output => true,
        Role::Value => record.commandable,
    };
    if commandable {
        Access::Commandable {
            relinquish_default: setting(&record.relinquish_default, decode),
        }
    } else {
        Access::ReadOnly
    }
}

/// An optional setting as its type decodes it; `None` when the record
/// lacks it or its value doesn't fit the point: one of another type, which
/// only a hand-built record holds, or a multi-state relinquish default
/// above 4294967295, which the record names as no state.
fn setting<T>(value: &Option<Value>, decode: fn(&[u8]) -> Option<T>) -> Option<T> {
    value.as_ref().and_then(|value| decode(&value.raw))
}

/// `field`, which every object of the type has, or why the record lacks
/// it: the problem its read had, the reason naming the property.
fn required<T>(record: &PointRecord, property: P, field: Option<T>) -> Result<T, (P, String)> {
    let Some(field) = field else {
        let why = record
            .problem(property)
            .map_or_else(|| "not read".to_string(), ToString::to_string);
        return Err((property, format!("{}: {why}", kebab(property))));
    };
    Ok(field)
}

/// The one value in `data`, when it decodes as exactly one.
fn single(data: &[u8]) -> Option<PropertyValue> {
    match <[PropertyValue; 1]>::try_from(decode_all(data)?) {
        Ok([value]) => Some(value),
        Err(_) => None,
    }
}

fn real(data: &[u8]) -> Option<f32> {
    match single(data)? {
        PropertyValue::Real(value) => Some(value),
        _ => None,
    }
}

fn unsigned(data: &[u8]) -> Option<u32> {
    match single(data)? {
        PropertyValue::Unsigned(value) => u32::try_from(value).ok(),
        _ => None,
    }
}

/// An Unsigned of any size, as an Integer Value's COV_Increment is.
fn unsigned64(data: &[u8]) -> Option<u64> {
    match single(data)? {
        PropertyValue::Unsigned(value) => Some(value),
        _ => None,
    }
}

/// An INTEGER: an Integer Value's Present_Value, relinquish default and
/// range ends.
fn signed(data: &[u8]) -> Option<i32> {
    match single(data)? {
        PropertyValue::Signed(value) => Some(value),
        _ => None,
    }
}

fn binary_pv(data: &[u8]) -> Option<BinaryPV> {
    match single(data)? {
        PropertyValue::Enumerated(raw) => Some(BinaryPV::from_raw(raw)),
        _ => None,
    }
}

/// A Polarity as the device sent it, NORMAL and REVERSE alike: a list
/// keeps what the device has.
fn polarity(data: &[u8]) -> Option<Polarity> {
    match single(data)? {
        PropertyValue::Enumerated(raw) => Some(Polarity::from_raw(raw)),
        _ => None,
    }
}
