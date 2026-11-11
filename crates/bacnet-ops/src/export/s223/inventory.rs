//! Reading what the export needs, one request at a time.
//!
//! The Device object's Object_Name (one ReadProperty), its Object_List
//! (whole, or element by element when it must be), then one property list
//! for each object of a selected type: by ReadPropertyMultiple, or
//! ReadProperty each once the device has refused RPM. Every request takes
//! its turn at the device's gate and the rate cap, as the reader's other
//! reads do. A device with N exported objects costs N + 2 requests by RPM.
//!
//! An object whose read fails is kept with its error, and the export skips
//! it; so is one whose Object_Name, an analog object's Units or a
//! multi-state object's Number_Of_States can't be read, the error naming
//! that property. A device that stops answering, or a caller that cancels,
//! ends the reading once the Object_List is in; what was read is kept, and
//! [`Inventory::stopped`] says why. Before that, either is an error. An
//! Object_List read element by element that stops early ends the reading
//! there: no object is asked for.
//!
//! A device that refuses ReadPropertyMultiple costs one refused request,
//! then a ReadProperty a property: 2 for a binary input or output, 3 for
//! an analog input or output or a binary value, 4 for an analog value or a
//! multi-state input or output, and 5 for a multi-state value.
//!
//! [`Detail::Full`] reads the rest of a point list in the same requests:
//! still N + 2 by RPM, the Device's name and Description going in one
//! RPM. By ReadProperty it costs one more for the Device's Description and
//! these per object: 6 for an analog input, 7 for an analog output, 8 for
//! an analog value, 4 for a binary input, 5 for a binary output, 6 for a
//! binary value, 4 for a multi-state input, 5 for a multi-state output and
//! 6 for a multi-state value.
//!
//! An input reads as [`Access::ReadOnly`] and an output as
//! [`Access::Commandable`]. A value is commandable when it has a
//! Priority_Array and [`Access::ReadOnly`] otherwise, even when a peer may
//! write it: a read can't tell a writable value from a read-only one, only
//! a write could.
//!
//! A front end that pages the export (the MCP tool) takes the Object_List
//! its own way and reads the rest piece by piece, as the inventory does:
//! [`Engine::inventory_device`] and [`Engine::inventory_object`].

use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{BinaryPV, EngineeringUnits, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::{
    Access, Analog, Binary, Detail, DeviceInfo, Entry, Inventory, InventoryOptions, MultiState,
    Point, PointKind,
};
use crate::engine::{Engine, Halt, Reader, Wanted, device_error, unreachable};
use crate::format::{RawRow, StateTexts, decode_all};
use crate::label::{kebab, object_label};

impl<T: TransportPort + 'static> Engine<T> {
    /// `device`'s objects of the types `options` names, as the Standard
    /// 223 export reads them ([`read_inventory`]). Fails only when the
    /// device's name or Object_List can't be read. A value without a
    /// Priority_Array reads as [`Access::ReadOnly`], writable or not.
    pub async fn inventory(
        &self,
        device: u32,
        options: &InventoryOptions,
    ) -> Result<Inventory, Error> {
        let mut reader = self.reader(device);
        let inventory = read_inventory(&mut reader, options).await;
        self.settle(&reader);
        inventory.map_err(device_error)
    }

    /// `device`'s Device object as the inventory reads it at `detail`: at
    /// [`Detail::Export`] one ReadProperty of its Object_Name, at
    /// [`Detail::Full`] its name and Description by one RPM (or a
    /// ReadProperty each). The text is exactly as the device sent it, and a
    /// property it gave none of is `None`. Fails only when the device
    /// doesn't answer.
    pub async fn inventory_device(&self, device: u32, detail: Detail) -> Result<DeviceInfo, Error> {
        let mut reader = self.reader(device);
        let read = read_device(&mut reader, detail).await;
        self.settle(&reader);
        read.map_err(device_error)
    }

    /// One object of `device` as the inventory reads it at `detail`, with
    /// the same properties: one ReadPropertyMultiple, or a ReadProperty
    /// each once the device has refused RPM (remembered, as
    /// [`Engine::read`] does). A read the device refuses, or a property
    /// every object of the type has that it can't give, comes back as
    /// [`Entry::Failed`], which the export skips; an optional property it
    /// can't give is `None`. An object of a type the export doesn't read
    /// (not one of the [`EXPORTED_TYPES`](super::EXPORTED_TYPES)) is
    /// [`Entry::Failed`] at once, with no request sent. Only a device that
    /// stops answering is an error.
    pub async fn inventory_object(
        &self,
        device: u32,
        object: ObjectIdentifier,
        detail: Detail,
    ) -> Result<Entry, Error> {
        let mut reader = self.reader(device);
        let read = read_object(&mut reader, object, detail).await;
        self.settle(&reader);
        read.map_err(device_error)
    }
}

/// The objects of the types `options` names on `reader`'s device, with
/// what the export needs of each (see the module docs). A reader that
/// meets a refusal of RPM leaves `rpm` false, for its owner to remember.
pub async fn read_inventory<T: TransportPort + 'static>(
    reader: &mut Reader<'_, T>,
    options: &InventoryOptions,
) -> Result<Inventory, Halt> {
    let device = read_device(reader, options.detail).await?;
    let list = reader.object_list(|_, _| {}).await?;
    let mut inventory = Inventory::new(device);
    inventory.stopped = list.stopped.map(|why| format!("Object_List, {why}"));
    // An element-by-element read that stopped means the device stopped
    // answering: asking it for each object would only time out again.
    if inventory.stopped.is_some() {
        return Ok(inventory);
    }
    let wanted = list
        .objects
        .into_iter()
        .filter(|object| options.types.contains(&object.object_type()));
    for object in wanted {
        match read_object(reader, object, options.detail).await {
            Ok(entry) => inventory.entries.push(entry),
            Err(Halt::Device(error)) => {
                inventory.stopped = Some(format!("{}: {error}", object_label(object)));
                break;
            }
            // A caller that stops partway keeps what was read.
            Err(Halt::Cancelled) => {
                inventory.stopped = Some(format!("cancelled before {}", object_label(object)));
                break;
            }
        }
    }
    Ok(inventory)
}

/// The Device object's name, and at [`Detail::Full`] its Description,
/// exactly as the device sent them; `None` for one the device answered
/// with an error or with no string. Only a device that doesn't answer, or
/// a stop, is an error.
async fn read_device<T: TransportPort + 'static>(
    reader: &mut Reader<'_, T>,
    detail: Detail,
) -> Result<DeviceInfo, Halt> {
    let device = reader.device_object()?;
    let mut info = DeviceInfo::new(reader.device);
    let rows = match detail {
        // One ReadProperty, as the export has always read it.
        Detail::Export => match reader.rp(device, P::OBJECT_NAME, None).await {
            Ok(data) => vec![(P::OBJECT_NAME, Ok(data))],
            Err(Halt::Device(error)) if !unreachable(&error) => Vec::new(),
            Err(halt) => return Err(halt),
        },
        Detail::Full => {
            let wanted = [P::OBJECT_NAME, P::DESCRIPTION];
            match reader.properties(device, Wanted::List(&wanted)).await {
                Ok((rows, _)) => rows,
                // An RPM the device answers with an error of its own (not
                // a refusal of the service, which `properties` reads past
                // itself) still leaves the name to read as Export does, so
                // Full never knows less than Export.
                Err(Halt::Device(error)) if !unreachable(&error) => {
                    match reader.rp(device, P::OBJECT_NAME, None).await {
                        Ok(data) => vec![(P::OBJECT_NAME, Ok(data))],
                        Err(Halt::Device(error)) if !unreachable(&error) => Vec::new(),
                        Err(halt) => return Err(halt),
                    }
                }
                Err(halt) => return Err(halt),
            }
        }
    };
    info.name = value(&rows, P::OBJECT_NAME).and_then(text);
    info.description = value(&rows, P::DESCRIPTION).and_then(text);
    Ok(info)
}

/// One object with what the export needs of it at `detail`
/// ([`properties`]). A read the device refuses is kept as the object's
/// error; only a device that doesn't answer, or a stop, is an error.
async fn read_object<T: TransportPort + 'static>(
    reader: &mut Reader<'_, T>,
    object: ObjectIdentifier,
    detail: Detail,
) -> Result<Entry, Halt> {
    let Some((family, role)) = family_of(object.object_type()) else {
        let error = format!("{} objects aren't exported", kebab(object.object_type()));
        return Ok(Entry::Failed {
            object,
            property: None,
            error,
        });
    };
    let properties = properties(family, role, detail);
    match reader.properties(object, Wanted::List(&properties)).await {
        Ok((rows, _)) => Ok(entry(object, family, role, &rows)),
        Err(Halt::Device(error)) if !unreachable(&error) => Ok(Entry::Failed {
            object,
            property: None,
            error: error.to_string(),
        }),
        Err(halt) => Err(halt),
    }
}

/// `object`'s entry from the rows its read gave.
fn entry(object: ObjectIdentifier, family: Family, role: Role, rows: &[RawRow]) -> Entry {
    match point(object, family, role, rows) {
        Ok(point) => Entry::Point(point),
        Err((property, error)) => Entry::Failed {
            object,
            property: Some(property),
            error,
        },
    }
}

/// `object`'s entry from `rows`, as [`read_object`] makes it at either
/// detail; for the reader's tests.
#[cfg(test)]
pub(super) fn entry_of(object: ObjectIdentifier, rows: &[RawRow]) -> Entry {
    let (family, role) = family_of(object.object_type()).expect("an exported type");
    entry(object, family, role, rows)
}

/// The families the export maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Analog,
    Binary,
    MultiState,
}

/// Which of a family's three types an object is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Input,
    Output,
    Value,
}

/// The family and role of `object_type`, when it is one of the
/// [`EXPORTED_TYPES`](super::EXPORTED_TYPES).
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
        _ => return None,
    })
}

/// What one object's read asks for: [`Detail::Export`]'s properties in
/// the order the export has always asked for them, then
/// [`Detail::Full`]'s.
fn properties(family: Family, role: Role, detail: Detail) -> Vec<P> {
    let mut list = vec![P::OBJECT_NAME, P::DESCRIPTION];
    match family {
        Family::Analog => list.push(P::UNITS),
        Family::Binary => {}
        Family::MultiState => list.extend([P::NUMBER_OF_STATES, P::STATE_TEXT]),
    }
    // An output always has a Priority_Array; a value may.
    if role == Role::Value {
        list.push(P::PRIORITY_ARRAY);
    }
    if detail == Detail::Full {
        match family {
            Family::Analog => {
                list.extend([P::COV_INCREMENT, P::MIN_PRES_VALUE, P::MAX_PRES_VALUE]);
            }
            Family::Binary => list.extend([P::ACTIVE_TEXT, P::INACTIVE_TEXT]),
            Family::MultiState => {}
        }
        if role != Role::Input {
            list.push(P::RELINQUISH_DEFAULT);
        }
    }
    list
}

/// One object's point from its rows, or why it has none: a property every
/// object of its type has is missing, named with the reason.
fn point(
    object: ObjectIdentifier,
    family: Family,
    role: Role,
    rows: &[RawRow],
) -> Result<Point, (P, String)> {
    let name = required(rows, P::OBJECT_NAME, text, "a character string")?;
    let kind = match family {
        Family::Analog => {
            let units = required(rows, P::UNITS, units, "an engineering unit")?;
            let mut analog = Analog::new(units, access(role, rows, real));
            analog.cov_increment = value(rows, P::COV_INCREMENT).and_then(real);
            analog.min_pres_value = value(rows, P::MIN_PRES_VALUE).and_then(real);
            analog.max_pres_value = value(rows, P::MAX_PRES_VALUE).and_then(real);
            PointKind::Analog(analog)
        }
        Family::Binary => {
            let mut binary = Binary::new(access(role, rows, binary_pv));
            binary.active_text = value(rows, P::ACTIVE_TEXT).and_then(text);
            binary.inactive_text = value(rows, P::INACTIVE_TEXT).and_then(text);
            PointKind::Binary(binary)
        }
        Family::MultiState => {
            let states = required(rows, P::NUMBER_OF_STATES, unsigned, "an Unsigned")?;
            let mut multi_state = MultiState::new(states, access(role, rows, unsigned));
            multi_state.state_texts = StateTexts::from_rows(rows).states;
            PointKind::MultiState(multi_state)
        }
    };
    let mut point = Point::new(object, name, kind);
    point.description = value(rows, P::DESCRIPTION).and_then(text);
    Ok(point)
}

/// How Present_Value is written, as far as a read can tell: an input is
/// read-only, an output commandable, and a value commandable when it has a
/// Priority_Array. A commandable point's relinquish default is the one
/// read, when it was.
fn access<T>(role: Role, rows: &[RawRow], decode: fn(&[u8]) -> Option<T>) -> Access<T> {
    let commandable = match role {
        Role::Input => false,
        Role::Output => true,
        Role::Value => value(rows, P::PRIORITY_ARRAY).is_some(),
    };
    if commandable {
        Access::Commandable {
            relinquish_default: value(rows, P::RELINQUISH_DEFAULT).and_then(decode),
        }
    } else {
        Access::ReadOnly
    }
}

/// The raw value of `property` among `rows`, when the device gave one.
fn value(rows: &[RawRow], property: P) -> Option<&[u8]> {
    rows.iter()
        .find(|(p, _)| *p == property)
        .and_then(|(_, value)| value.as_deref().ok())
}

/// `property`, decoded, or why it can't be: the device's error for it, or
/// a value that isn't `what`. The reason names the property.
fn required<T>(
    rows: &[RawRow],
    property: P,
    decode: fn(&[u8]) -> Option<T>,
    what: &str,
) -> Result<T, (P, String)> {
    let why = match rows.iter().find(|(p, _)| *p == property) {
        None => "not read".to_string(),
        Some((_, Err(error))) => error.to_string(),
        Some((_, Ok(data))) => match decode(data) {
            Some(decoded) => return Ok(decoded),
            None => format!("not {what}"),
        },
    };
    Err((property, format!("{}: {why}", kebab(property))))
}

/// The one value in `data`, when it decodes as exactly one.
fn single(data: &[u8]) -> Option<PropertyValue> {
    match <[PropertyValue; 1]>::try_from(decode_all(data)?) {
        Ok([value]) => Some(value),
        Err(_) => None,
    }
}

/// A value that is one character string, exactly as the device sent it.
fn text(data: &[u8]) -> Option<String> {
    match single(data)? {
        PropertyValue::CharacterString(text) => Some(text),
        _ => None,
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

fn units(data: &[u8]) -> Option<EngineeringUnits> {
    match single(data)? {
        PropertyValue::Enumerated(raw) => Some(EngineeringUnits::from_raw(raw)),
        _ => None,
    }
}

fn binary_pv(data: &[u8]) -> Option<BinaryPV> {
    match single(data)? {
        PropertyValue::Enumerated(raw) => Some(BinaryPV::from_raw(raw)),
        _ => None,
    }
}
