//! A device's objects as point records (#2560): what an application needs
//! to list, label and address a device's points, read in as few requests
//! as the device allows.
//!
//! [`Device::inventory`] reads the device summary, the Object_List, and
//! then the objects in batches, into an [`Inventory`]: a [`PointRecord`]
//! an object, of every type unless [`InventoryOptions::types`] names some.
//! A record holds the object's name, description, units, state texts and
//! whether it takes commands; with [`InventoryOptions::settings`] its
//! COV_Increment, range and Relinquish_Default too; with
//! [`InventoryOptions::values`] its Present_Value and Status_Flags. Which
//! properties that is for each object type is one list, [`properties`].
//!
//! # What it costs
//!
//! By ReadPropertyMultiple: one request for the summary, one for the
//! Object_List, then one for each batch of objects. A batch is filled, in
//! Object_List order, up to 16 objects or 32 properties, whichever comes
//! first, so its size follows from how many properties each object's type
//! reads ([`properties`]): 16 objects of a type that reads two, 10 analog
//! inputs, 8 binary inputs or analog values, 6 multi-state values. With
//! values each object reads two more, and a batch holds 4 to 8; with
//! settings as well, 3 to 8. N objects of one type that each read p
//! properties take N divided by the batch's size, rounded up, where the
//! size is the smaller of 16 and 32/p rounded down: 31 analog inputs, at
//! three properties each, go ten a batch and take 4 requests. A mix of
//! types takes at most one or two more than its types would apart: about
//! N/8 + 2 for a typical controller, N/5 + 2 with values. A handle made from an address
//! whose instance isn't known yet costs one more, a read of the Device
//! object by the wildcard instance.
//!
//! - A batch whose answer the device can't send for its size (it doesn't
//!   segment, or its buffer is small) is asked again an object at a time,
//!   and the batches after it hold at most half as many objects as it did,
//!   down to one: at most four such refused batches an inventory. An
//!   answer that arrives in segments costs no more requests. A single
//!   object whose answer is too large costs its refused request and then
//!   a ReadProperty for each property.
//! - A batch the device fails as a whole for another reason is read by
//!   ReadProperty, a property a request, so one bad object can't put
//!   errors on its neighbours.
//! - A device that refuses ReadPropertyMultiple costs that one refused
//!   request (remembered by the client, so only once), then a ReadProperty
//!   for each property: 13 for the summary, one for the Object_List, and
//!   each object's count from [`properties`].
//! - An Object_List the device can't send whole is read element by
//!   element: one request for its size and one an object.
//!
//! No property is read twice: the Device object's own record takes its
//! Object_Name from the summary. Every request waits for its turn at the
//! device's gate and for the client's rate cap ([`RequestLimits`]), one
//! outstanding at a time.
//!
//! # What it holds while reading
//!
//! The Object_List (eight octets an object), the records built so far, and
//! the answers of one batch, 16 objects at most: each batch's answers
//! become records before the next is sent, however many objects the device
//! has. The list itself is as long as the device sends it; only a list
//! read element by element is bounded, at
//! [`MAX_OBJECTS`](crate::client::MAX_OBJECTS) elements. An object the
//! list names twice is read once.
//!
//! # Errors and partial results
//!
//! Whatever is decided before a request is sent is an error with nothing
//! sent: a device that can't be placed, an instance two addresses claim, a
//! routed address with no route. So is a device that doesn't answer the
//! summary or the Object_List, or refuses the list: no record was read
//! that a caller could keep. A list read element by element that fails at
//! one element fails the same way, as [`OpsError::ObjectList`], which
//! names the element. After that the inventory is `Ok`: a property or an object
//! the device refuses is data on its record ([`PointRecord::problems`]),
//! and a device that goes silent while the objects are read, or whose
//! route is lost then, leaves the records read so far, the objects not
//! read ([`Inventory::unread`]) and why ([`Inventory::stopped`]).
//!
//! [`raw::read_inventory`](crate::raw::read_inventory) is the layer under
//! [`Device::inventory`], for a caller that owns a low-level client and a
//! [`raw::Reader`](crate::raw::Reader).
//!
//! # JSON and CSV
//!
//! [`Inventory`] and [`PointRecord`] serialize with serde to the JSON the
//! `bacnet inventory` command prints ([`Inventory::to_json`]); the field
//! names are stable. [`Inventory::to_csv`] writes the records as CSV under
//! [`CSV_HEADER`]. The JSON keeps device text exactly as the device sent
//! it; the CSV does too, except that a cell of device text a spreadsheet
//! would run as a formula is written behind an apostrophe unless
//! [`CsvOptions::exact`] says otherwise.
//!
//! [`RequestLimits`]: crate::limits::RequestLimits

mod plan;
pub(crate) mod read;
mod record;
mod write;

use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use serde::Serialize;

pub use self::plan::properties;
use self::read::{InventoryHalt, read_inventory, read_point_record};
#[cfg(test)]
pub(crate) use self::record::record as record_of;
pub use self::write::{CSV_HEADER, CsvOptions};
use crate::client::{DeviceSummary, Halt, device_error};
use crate::device::{Device, DeviceAddress, sent};
use crate::error::OpsError;
use crate::format::{PropertyError, StateTexts, Value};
use crate::reading::IntoObject;
use crate::watch::{CovSupport, Watched};

/// What an inventory reads.
///
/// ```
/// use bacnet_ops::inventory::InventoryOptions;
/// use bacnet_types::enums::ObjectType;
///
/// // Every object, names and units only.
/// let all = InventoryOptions::new();
/// assert!(all.types.is_none() && !all.values && !all.settings);
/// // The analog inputs alone, with their values.
/// let inputs = InventoryOptions::new()
///     .with_types([ObjectType::ANALOG_INPUT])
///     .with_values(true);
/// assert!(inputs.reads(ObjectType::ANALOG_INPUT) && !inputs.reads(ObjectType::DEVICE));
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct InventoryOptions {
    /// The object types to read; `None`, the default, reads every object
    /// the Object_List names, of any type, a vendor's included. Objects of
    /// other types are left out without a request.
    pub types: Option<Vec<ObjectType>>,
    /// Also read each object's Present_Value and Status_Flags: two more
    /// properties an object in the same requests, so fewer objects fit a
    /// batch. Off by default.
    pub values: bool,
    /// Also read each object's settings, where its type has them:
    /// COV_Increment, Min_Pres_Value, Max_Pres_Value and
    /// Relinquish_Default ([`properties`]). Off by default.
    pub settings: bool,
}

impl InventoryOptions {
    /// Every object, with no values and no settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same options, reading only objects of `types`.
    pub fn with_types(mut self, types: impl IntoIterator<Item = ObjectType>) -> Self {
        self.types = Some(types.into_iter().collect());
        self
    }

    /// The same options, reading each object's Present_Value and
    /// Status_Flags or not.
    pub fn with_values(mut self, values: bool) -> Self {
        self.values = values;
        self
    }

    /// The same options, reading each object's settings or not.
    pub fn with_settings(mut self, settings: bool) -> Self {
        self.settings = settings;
        self
    }

    /// Whether objects of `object_type` are read.
    pub fn reads(&self, object_type: ObjectType) -> bool {
        self.types
            .as_ref()
            .is_none_or(|types| types.contains(&object_type))
    }
}

/// One property of a [`PointRecord`]'s object that gave no usable value,
/// and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Problem {
    /// The property.
    #[serde(serialize_with = "write::kebab")]
    pub property: PropertyIdentifier,
    /// The device's error for it, [`PropertyError::NoValue`] for one its
    /// answer left out, or what was wrong with the value it gave
    /// ([`PropertyError::Other`]: `not a character string`).
    #[serde(serialize_with = "write::display")]
    pub error: PropertyError,
}

impl Problem {
    /// The problem `error` with `property`.
    pub fn new(property: PropertyIdentifier, error: PropertyError) -> Self {
        Self { property, error }
    }
}

/// One object of a device as an application lists it: where it is, what
/// it is called, how its value reads and whether it takes commands.
///
/// Device text (`name`, `description`, `state_texts`) is exactly as the
/// device sent it: clean it before showing it
/// ([`clean_name`](crate::format::clean_name)). A field is `None` when the
/// object's type has no such property, the object lacks an optional one,
/// the options didn't ask for it, or its read failed; only the last is
/// listed in [`problems`](Self::problems).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct PointRecord {
    /// Where the device answers. JSON: its target text,
    /// `"192.168.1.10:47808"` or `"2:0x05"`.
    pub device: DeviceAddress,
    /// The device's instance.
    pub device_instance: u32,
    /// The object. JSON: `{"type": "analog-input", "type_number": 0,
    /// "instance": 1}`; CSV: `analog-input:1`.
    #[serde(serialize_with = "write::object")]
    pub object: ObjectIdentifier,
    /// Its Object_Name.
    pub name: Option<String>,
    /// Its Description.
    pub description: Option<String>,
    /// Its Units, for the types that have the property. JSON: `{"value":
    /// 62, "name": "degrees-celsius", "symbol": "°C"}`; CSV: the name.
    #[serde(serialize_with = "write::units")]
    pub units: Option<EngineeringUnits>,
    /// A binary object's Active_Text and Inactive_Text, or a multi-state
    /// object's State_Text from state 1; empty for other types. JSON:
    /// `{"active": .., "inactive": .., "states": [..]}`.
    #[serde(serialize_with = "write::state_texts")]
    pub state_texts: StateTexts,
    /// A multi-state object's Number_Of_States.
    pub number_of_states: Option<u32>,
    /// Whether its Present_Value takes command priorities: what
    /// [`Commandability::of`](crate::write::Commandability::of) says of
    /// its type, and for a type that may or may not (a value object, a
    /// vendor's type) whether the object has a Priority_Array. For such a
    /// type, `false` beside a [`problem`](Self::problem) with
    /// Priority_Array means unknown: the device wouldn't say.
    pub commandable: bool,
    /// Its Relinquish_Default, read with
    /// [`settings`](InventoryOptions::settings).
    #[serde(serialize_with = "write::value")]
    pub relinquish_default: Option<Value>,
    /// Its COV_Increment, read with settings: a REAL for an analog
    /// object, an Unsigned for an integer one.
    #[serde(serialize_with = "write::value")]
    pub cov_increment: Option<Value>,
    /// Its Min_Pres_Value, read with settings.
    #[serde(serialize_with = "write::value")]
    pub min_present_value: Option<Value>,
    /// Its Max_Pres_Value, read with settings.
    #[serde(serialize_with = "write::value")]
    pub max_present_value: Option<Value>,
    /// Its Present_Value, read with [`values`](InventoryOptions::values);
    /// its text names the units and the state, as
    /// [`Device::read_many`] writes it. JSON: `{"value": "72.5 °F",
    /// "decoded": 72.5}`, the text and the typed form.
    #[serde(serialize_with = "write::value")]
    pub present_value: Option<Value>,
    /// Its Status_Flags, read with values.
    #[serde(serialize_with = "write::value")]
    pub status_flags: Option<Value>,
    /// The properties asked for that gave no usable value, in the order
    /// asked: one the device refused, one whose value isn't of the
    /// property's type, and each property of an object the device doesn't
    /// have or left out of its answer. An optional property the object
    /// simply lacks (the device says it is an unknown property) is not a
    /// problem.
    pub problems: Vec<Problem>,
}

impl PointRecord {
    /// A record of `object` on the device `device_instance` at `device`,
    /// with nothing read: every field empty, not commandable.
    pub fn new(device: DeviceAddress, device_instance: u32, object: ObjectIdentifier) -> Self {
        Self {
            device,
            device_instance,
            object,
            name: None,
            description: None,
            units: None,
            state_texts: StateTexts::default(),
            number_of_states: None,
            commandable: false,
            relinquish_default: None,
            cov_increment: None,
            min_present_value: None,
            max_present_value: None,
            present_value: None,
            status_flags: None,
            problems: Vec::new(),
        }
    }

    /// The problem with `property`, if it had one.
    pub fn problem(&self, property: PropertyIdentifier) -> Option<&PropertyError> {
        self.problems
            .iter()
            .find(|problem| problem.property == property)
            .map(|problem| &problem.error)
    }
}

/// What [`Device::inventory`] read: the device, and a [`PointRecord`] for
/// each object of the selected types, in Object_List order.
///
/// Not `Clone`: `stopped` holds the low-level client's error, which can
/// hold the operating system's.
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct Inventory {
    /// Where the device answers.
    pub address: DeviceAddress,
    /// The device summary ([`Device::describe_device`]'s), each property
    /// its value or its own error. JSON: `{"instance": 1234, "name": ..,
    /// "properties": {"vendor-name": {"value": .., "decoded": ..}, ..}}`,
    /// a property the device refused as `{"error": ..}`.
    #[serde(serialize_with = "write::summary")]
    pub device: DeviceSummary,
    /// What is known of SubscribeCOV at the device, at no request of its
    /// own: what the summary's Protocol_Services_Supported lists, or what
    /// the device did when this client's watches asked
    /// ([`Device::cov_support`]). It is the device's, not a point's:
    /// whether one object reports changes is known only once a watch has
    /// asked it.
    pub cov: CovSupport,
    /// The objects read, in Object_List order.
    pub points: Vec<PointRecord>,
    /// The objects of the selected types not read because the reading
    /// stopped, in Object_List order; empty when it didn't.
    #[serde(serialize_with = "write::objects")]
    pub unread: Vec<ObjectIdentifier>,
    /// Why the reading stopped early: the device went silent
    /// ([`OpsError::unreachable`]) while its objects were being read, or
    /// the route to it was lost then. `points` keeps what was read before
    /// it. JSON: the error's text, or `null`.
    #[serde(serialize_with = "write::stopped")]
    pub stopped: Option<OpsError>,
}

impl Inventory {
    /// An inventory of the device at `address` with no record yet: for a
    /// caller that builds one by hand, a test's say.
    pub fn new(address: DeviceAddress, device: DeviceSummary) -> Self {
        Self {
            address,
            device,
            cov: CovSupport::Unknown,
            points: Vec::new(),
            unread: Vec::new(),
            stopped: None,
        }
    }

    /// The record of `object`, when it was read.
    pub fn point(&self, object: ObjectIdentifier) -> Option<&PointRecord> {
        self.points.iter().find(|point| point.object == object)
    }
}

impl<T: TransportPort + 'static> Device<T> {
    /// The device's objects as point records: its summary, its
    /// Object_List, then the objects of the types `options` names (all by
    /// default) in batches. See the [module docs](crate::inventory) for
    /// what is read, what it costs and what a partial result holds. The
    /// client notes which COV services the summary lists, as
    /// [`describe_device`](Self::describe_device) does.
    ///
    /// # Errors
    ///
    /// With nothing sent: a device that can't be placed, an instance two
    /// addresses claim, or a routed address with no route
    /// ([`OpsError::UnknownDevice`], [`OpsError::Ambiguous`]). And a
    /// device that doesn't answer the summary or the Object_List, or
    /// refuses the list; a list read element by element that fails at one
    /// element is [`OpsError::ObjectList`], which names the element. A
    /// device that goes silent after that is not an error: see
    /// [`Inventory::stopped`].
    pub async fn inventory(&self, options: &InventoryOptions) -> Result<Inventory, OpsError> {
        let instance = self.resolve_instance().await?;
        // Which element of a list read element by element failed, for the
        // error to name.
        let mut element;
        let read = sent!(self, |at| {
            element = None;
            let mut reader = self.client().reader(at, Some(instance));
            let read = read_inventory(&mut reader, options).await;
            self.client().settle(&reader);
            read.map_err(|halt| match halt {
                InventoryHalt::Device(error) => error,
                InventoryHalt::ObjectList(stop) => {
                    element = Some((stop.index, stop.total));
                    stop.error
                }
                InventoryHalt::Cancelled => device_error(Halt::Cancelled),
            })
        });
        let mut inventory = match (read, element) {
            (Ok(inventory), _) => inventory,
            (Err(source), Some((index, total))) => {
                let source = Box::new(source);
                return Err(OpsError::ObjectList {
                    index,
                    total,
                    source,
                });
            }
            (Err(error), None) => return Err(error),
        };
        let services = inventory
            .device
            .value(PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
            .and_then(|value| crate::watch::services(&value.raw));
        if let Some(services) = services {
            self.client().cov_declared(&inventory.address, services);
        }
        inventory.cov = self
            .client()
            .cov_known(&inventory.address, &Watched::Object);
        Ok(inventory)
    }

    /// One object's record, as [`inventory`](Self::inventory) reads it
    /// under `options` (whose `types` it ignores): one
    /// ReadPropertyMultiple, or a ReadProperty each property once the
    /// device has refused RPM or fails this one. For a front end that
    /// pages a device's objects itself. An object the device doesn't have
    /// is a record whose properties are all
    /// [`problems`](PointRecord::problems).
    ///
    /// # Errors
    ///
    /// Text that names no object ([`OpsError::Parse`]) and a device that
    /// can't be placed, with nothing sent; and a device that doesn't
    /// answer.
    pub async fn point_record(
        &self,
        object: impl IntoObject,
        options: &InventoryOptions,
    ) -> Result<PointRecord, OpsError> {
        let object = object.into_object()?;
        let instance = self.resolve_instance().await?;
        sent!(self, |at| {
            let mut reader = self.client().reader(at, Some(instance));
            let read = read_point_record(&mut reader, object, options).await;
            self.client().settle(&reader);
            read.map_err(device_error)
        })
    }
}

#[cfg(test)]
pub(crate) mod test_lab;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod record_tests;

#[cfg(test)]
mod write_tests;
