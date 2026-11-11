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
//! it. A device that stops answering, or a caller that cancels, ends the
//! reading once the Object_List is in; what was read is kept, and
//! [`Inventory::stopped`] says why. Before that, either is an error.
//! An Object_List read element by element that stops early ends the
//! reading there: no object is asked for.
//!
//! A device that refuses ReadPropertyMultiple costs one refused request,
//! then a ReadProperty a property: 2 for a binary input or output, 3 for
//! an analog input or output or a binary value, 4 for an analog value or a
//! multi-state input or output, and 5 for a multi-state value.
//!
//! A front end that pages the export (the MCP tool) takes the Object_List
//! its own way and reads the rest piece by piece, as the inventory does:
//! [`Engine::inventory_device_name`] and [`Engine::inventory_object`].

use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::{Inventory, InventoryObject, InventoryOptions};
use crate::engine::{Engine, Halt, Reader, Wanted, device_error, unreachable};
use crate::format::{RawRow, StateTexts, decode_all, units};
use crate::label::object_label;

impl<T: TransportPort + 'static> Engine<T> {
    /// `device`'s objects of the types `options` names, as the Standard
    /// 223 export reads them ([`read_inventory`]). Fails only when the
    /// device's name or Object_List can't be read.
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

    /// `device`'s Object_Name as the inventory reads it: one ReadProperty,
    /// the text exactly as the device sent it, `None` when it gave none.
    /// Fails only when the device doesn't answer.
    pub async fn inventory_device_name(&self, device: u32) -> Result<Option<String>, Error> {
        read_device_name(&self.reader(device))
            .await
            .map_err(device_error)
    }

    /// One object of `device` as the inventory reads it, with the same
    /// properties: one ReadPropertyMultiple, or a ReadProperty each once
    /// the device has refused RPM (remembered, as [`Engine::read`] does).
    /// A read the device refuses comes back as the object's error, which
    /// the export skips, and a property it can't give is left out; only a
    /// device that stops answering is an error.
    pub async fn inventory_object(
        &self,
        device: u32,
        object: ObjectIdentifier,
    ) -> Result<InventoryObject, Error> {
        let mut reader = self.reader(device);
        let read = read_object(&mut reader, object).await;
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
    let device_name = read_device_name(reader).await?;
    let list = reader.object_list(|_, _| {}).await?;
    let mut inventory = Inventory {
        device: reader.device,
        device_name,
        objects: Vec::new(),
        stopped: list.stopped.map(|why| format!("Object_List, {why}")),
    };
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
        match read_object(reader, object).await {
            Ok(entry) => inventory.objects.push(entry),
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

/// The Device object's Object_Name, exactly as the device sent it; `None`
/// when the device answered with an error or with no string. Only a
/// device that doesn't answer, or a stop, is an error.
async fn read_device_name<T: TransportPort + 'static>(
    reader: &Reader<'_, T>,
) -> Result<Option<String>, Halt> {
    let device = reader.device_object()?;
    match reader
        .rp(device, PropertyIdentifier::OBJECT_NAME, None)
        .await
    {
        Ok(data) => Ok(text(&data)),
        Err(Halt::Device(error)) if unreachable(&error) => Err(Halt::Device(error)),
        Err(Halt::Device(_)) => Ok(None),
        Err(Halt::Cancelled) => Err(Halt::Cancelled),
    }
}

/// One object with what the export needs of it ([`properties`]). A read
/// the device refuses is kept as the object's error; only a device that
/// doesn't answer, or a stop, is an error.
async fn read_object<T: TransportPort + 'static>(
    reader: &mut Reader<'_, T>,
    object: ObjectIdentifier,
) -> Result<InventoryObject, Halt> {
    let properties = properties(object.object_type());
    match reader.properties(object, Wanted::List(&properties)).await {
        Ok((rows, _)) => Ok(entry(object, &rows)),
        Err(Halt::Device(error)) if !unreachable(&error) => Ok(InventoryObject {
            error: Some(error.to_string()),
            ..InventoryObject::new(object)
        }),
        Err(halt) => Err(halt),
    }
}

/// An output always has a Priority_Array; a value may.
fn is_output(object_type: ObjectType) -> bool {
    [
        ObjectType::ANALOG_OUTPUT,
        ObjectType::BINARY_OUTPUT,
        ObjectType::MULTI_STATE_OUTPUT,
    ]
    .contains(&object_type)
}

fn is_value(object_type: ObjectType) -> bool {
    [
        ObjectType::ANALOG_VALUE,
        ObjectType::BINARY_VALUE,
        ObjectType::MULTI_STATE_VALUE,
    ]
    .contains(&object_type)
}

fn is_analog(object_type: ObjectType) -> bool {
    [
        ObjectType::ANALOG_INPUT,
        ObjectType::ANALOG_OUTPUT,
        ObjectType::ANALOG_VALUE,
    ]
    .contains(&object_type)
}

/// What one object's read asks for.
fn properties(object_type: ObjectType) -> Vec<PropertyIdentifier> {
    let mut list = vec![
        PropertyIdentifier::OBJECT_NAME,
        PropertyIdentifier::DESCRIPTION,
    ];
    if is_analog(object_type) {
        list.push(PropertyIdentifier::UNITS);
    }
    if crate::format::is_multistate(object_type) {
        list.push(PropertyIdentifier::NUMBER_OF_STATES);
        list.push(PropertyIdentifier::STATE_TEXT);
    }
    if is_value(object_type) {
        list.push(PropertyIdentifier::PRIORITY_ARRAY);
    }
    list
}

/// One object from its rows.
fn entry(object: ObjectIdentifier, rows: &[RawRow]) -> InventoryObject {
    let value = |property| {
        rows.iter()
            .find(|(p, _)| *p == property)
            .and_then(|(_, value)| value.as_deref().ok())
    };
    let object_type = object.object_type();
    let states = value(PropertyIdentifier::NUMBER_OF_STATES)
        .and_then(decode_all)
        .and_then(|values| match values.as_slice() {
            [PropertyValue::Unsigned(n)] => u32::try_from(*n).ok(),
            _ => None,
        });
    InventoryObject {
        name: value(PropertyIdentifier::OBJECT_NAME).and_then(text),
        description: value(PropertyIdentifier::DESCRIPTION).and_then(text),
        units: units(rows),
        states,
        state_texts: StateTexts::from_rows(rows).states,
        priority_array: is_output(object_type)
            || (is_value(object_type) && value(PropertyIdentifier::PRIORITY_ARRAY).is_some()),
        ..InventoryObject::new(object)
    }
}

/// A value that is one character string, exactly as the device sent it.
fn text(data: &[u8]) -> Option<String> {
    match decode_all(data)?.as_slice() {
        [PropertyValue::CharacterString(text)] => Some(text.clone()),
        _ => None,
    }
}
