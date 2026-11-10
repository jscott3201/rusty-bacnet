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
}

/// The objects of the types `options` names on `reader`'s device, with
/// what the export needs of each (see the module docs). A reader that
/// meets a refusal of RPM leaves `rpm` false, for its owner to remember.
pub async fn read_inventory<T: TransportPort + 'static>(
    reader: &mut Reader<'_, T>,
    options: &InventoryOptions,
) -> Result<Inventory, Halt> {
    let device = reader.device_object()?;
    let device_name = match reader
        .rp(device, PropertyIdentifier::OBJECT_NAME, None)
        .await
    {
        Ok(data) => text(&data),
        Err(Halt::Device(error)) if unreachable(&error) => return Err(Halt::Device(error)),
        Err(Halt::Device(_)) => None,
        Err(Halt::Cancelled) => return Err(Halt::Cancelled),
    };
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
        let properties = properties(object.object_type());
        match reader.properties(object, Wanted::List(&properties)).await {
            Ok((rows, _)) => inventory.objects.push(entry(object, &rows)),
            Err(Halt::Device(error)) if unreachable(&error) => {
                inventory.stopped = Some(format!("{}: {error}", object_label(object)));
                break;
            }
            Err(Halt::Device(error)) => inventory.objects.push(InventoryObject {
                error: Some(error.to_string()),
                ..InventoryObject::new(object)
            }),
            // A caller that stops partway (an MCP call out of budget, say)
            // keeps what was read.
            Err(Halt::Cancelled) => {
                inventory.stopped = Some(format!("cancelled before {}", object_label(object)));
                break;
            }
        }
    }
    Ok(inventory)
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
