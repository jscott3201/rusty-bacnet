//! A device's objects as point records (#2560): what an application needs
//! to list, label and address a device's points, read in as few requests
//! as the device allows.
//!
//! [`Device::inventory`] reads the device summary, the Object_List, and
//! then the objects in batches, into an [`Inventory`]: a [`PointRecord`]
//! an object, of every type unless [`InventoryOptions::types`] names some.
//! A record holds the object's name, description, units, state texts and
//! whether it takes commands; with [`InventoryOptions::settings`] its
//! COV_Increment, range, Relinquish_Default and Polarity too; with
//! [`InventoryOptions::values`] its Present_Value, Status_Flags,
//! Event_State, Reliability and priority in control; with
//! [`InventoryOptions::details`] its Device_Type and Profile_Name. Which
//! properties that is for each object type is one list, [`properties`].
//!
//! [`Device::point_records`] reads the records of objects the caller
//! names, a page of a device's Object_List say, in the same batches and
//! with the same fallbacks, into [`PointRecords`]; [`Device::point_batch`]
//! says how many of them one request reads.
//!
//! The two shrink their batches after an answer the device couldn't send
//! for its size in different units. An inventory reads every object in
//! one call, so it learns as it goes: it halves the objects a batch holds,
//! starting at 16 each time, and its cost below stays exactly what it is
//! whatever the client learned before. Point records come a page a call,
//! each call starting afresh, so what one call learns must outlast it: the
//! client remembers half the refused batch's properties for the device
//! ([`Client::rpm_limit`](crate::Client::rpm_limit)), a limit that holds
//! when the next call asks for more or fewer properties an object.
//!
//! # What it costs
//!
//! By ReadPropertyMultiple: one request for the summary, one for the
//! Object_List, then one for each batch of objects. A batch is filled, in
//! Object_List order, up to 16 objects or 32 properties, whichever comes
//! first, so its size follows from how many properties each object's type
//! reads ([`properties`]): 16 objects of a type that reads two, 10 analog
//! inputs, 8 binary inputs or analog values, 6 multi-state values. With
//! values each object reads four more (Present_Value, Status_Flags,
//! Event_State, Reliability), an output, Access Door or lighting output
//! five (its Current_Command_Priority too; a value object's priority in
//! control comes from the Priority_Array it reads anyway), and a batch
//! holds 3 to 5: 4 analog inputs or analog outputs, 3 binary or
//! multi-state outputs; with settings as well, 2 to 5. Settings alone add
//! up to four properties an object, so fewer objects fit a batch and a
//! device with many of them takes more requests: a Binary Input then reads
//! five (its Polarity among them) and goes six a batch, a Binary Output
//! six and goes five, against eight of either without. Details add two
//! (Device_Type, Profile_Name): 6 analog inputs a batch, 5 binary inputs.
//! N objects of one type that each read p properties take N divided by the
//! batch's size, rounded up, where the size is the smaller of 16 and 32/p
//! rounded down: 31 analog inputs, at three properties each, go ten a
//! batch and take 4 requests. A mix of types takes at most one or two more
//! than its types would apart: about N/8 + 2 for a typical controller,
//! N/4 + 2 with values. A handle made from an address whose instance isn't
//! known yet costs one more, a read of the Device object by the wildcard
//! instance.
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
//!   errors on its neighbours. Such a failure isn't a refusal of the
//!   service, so the next batch is tried by ReadPropertyMultiple again: a
//!   device that fails every one costs one wasted request a batch.
//! - A device that refuses ReadPropertyMultiple costs that one refused
//!   request (remembered by the client, so only once), then a ReadProperty
//!   for each property: 13 for the summary, one for the Object_List, and
//!   each object's count from [`properties`]: four more an object with
//!   values (five on an output), two more with details.
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
//! [`raw::Reader`](crate::raw::Reader). A raw read whose stop flag goes up
//! once the Object_List is in keeps its records the same way, in the
//! [`raw::InventoryHalt::Cancelled`](InventoryHalt::Cancelled) it ends
//! with.
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

mod control;
mod plan;
pub(crate) mod read;
mod record;
mod settings;
mod write;

use bacnet_transport::port::TransportPort;
use bacnet_types::bitstring::{StatusFlags, status_flags_from_bacnet};
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use serde::Serialize;

pub use self::plan::properties;
use self::read::{InventoryHalt, Objects, Sizing, one_batch, read_inventory, read_objects};
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
/// assert!(all.types.is_none() && !all.values && !all.settings && !all.details);
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
    /// Also read each object's run-time state: its Present_Value,
    /// Status_Flags, Event_State and Reliability, and on an object whose
    /// Present_Value always takes commands its Current_Command_Priority
    /// ([`properties`]): four or five more properties an object in the
    /// same requests, so fewer objects fit a batch. Off by default.
    pub values: bool,
    /// Also read each object's settings, where its type has them:
    /// COV_Increment, Min_Pres_Value, Max_Pres_Value, Relinquish_Default
    /// and Polarity ([`properties`]): up to four more properties an object
    /// in the same requests, so fewer objects fit a batch. Off by default.
    pub settings: bool,
    /// Also read each object's Device_Type and Profile_Name: what it is
    /// connected to, and the profile that describes it (#2620); two more
    /// properties an object in the same requests. Off by default.
    pub details: bool,
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

    /// The same options, reading each object's run-time state
    /// ([`values`](Self::values)) or not.
    pub fn with_values(mut self, values: bool) -> Self {
        self.values = values;
        self
    }

    /// The same options, reading each object's settings or not.
    pub fn with_settings(mut self, settings: bool) -> Self {
        self.settings = settings;
        self
    }

    /// The same options, reading each object's Device_Type and
    /// Profile_Name or not.
    pub fn with_details(mut self, details: bool) -> Self {
        self.details = details;
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
    /// ([`PropertyError::Other`]: `not a character string`, `negative`).
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
/// the options didn't ask for it, or its read failed, a value of another
/// datatype than the property's included; only the last is listed in
/// [`problems`](Self::problems). A value a point list builder would refuse
/// is kept as the device gave it and listed there too.
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
    /// Its Device_Type, read with [`details`](InventoryOptions::details):
    /// the device's text for what the object is connected to, such as the
    /// kind of sensor behind an input; optional in the tables. One that
    /// isn't a character string is a problem instead.
    pub device_type: Option<String>,
    /// Its Profile_Name, read with details: the name of the profile the
    /// object follows, as its device writes it; optional, and a problem
    /// when it isn't a character string, as Device_Type.
    pub profile_name: Option<String>,
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
    /// [`settings`](InventoryOptions::settings), of its Present_Value's
    /// datatype: a REAL on an analog output or value, an Enumerated on a
    /// binary one, an Unsigned on a multi-state one, and so on by type. One
    /// of another datatype is a problem instead.
    #[serde(serialize_with = "write::value")]
    pub relinquish_default: Option<Value>,
    /// Its COV_Increment, read with settings: a REAL for an analog
    /// object, an Unsigned for an Integer or Positive Integer Value, a
    /// Double for a Large Analog Value. One of another datatype is a
    /// problem instead.
    ///
    /// It is also a sign, at no cost of its own, of whether a
    /// subscription to the object's Present_Value is worth trying: on
    /// those six types the tables have the property exist exactly when the
    /// object reports changes of its Present_Value (the footnotes to their
    /// property tables, from Table 12-2 on). So with settings read, a
    /// value here is evidence that the object reports them, not proof:
    /// some devices give the property whether or not they do. `None` with
    /// no [`problem`](Self::problem) for it says a conforming device's
    /// object doesn't; [`Inventory::cov`] says whether the device offers
    /// SubscribeCOV at all. Other types have no such sign.
    #[serde(serialize_with = "write::value")]
    pub cov_increment: Option<Value>,
    /// Its Min_Pres_Value, read with settings, of its Present_Value's
    /// datatype; one of another is a problem instead.
    #[serde(serialize_with = "write::value")]
    pub min_present_value: Option<Value>,
    /// Its Max_Pres_Value, read with settings, as Min_Pres_Value.
    #[serde(serialize_with = "write::value")]
    pub max_present_value: Option<Value>,
    /// A Binary Input's or Output's Polarity, read with settings; one
    /// that isn't an Enumerated is a problem instead. JSON: `{"value":
    /// "reverse", "decoded": {"value": 1, "name": "reverse"}}`.
    #[serde(serialize_with = "write::value")]
    pub polarity: Option<Value>,
    /// Its Present_Value, read with [`values`](InventoryOptions::values);
    /// its text names the units and the state, as
    /// [`Device::read_many`] writes it. JSON: `{"value": "72.5 °F",
    /// "decoded": 72.5}`, the text and the typed form.
    #[serde(serialize_with = "write::value")]
    pub present_value: Option<Value>,
    /// The priority in control of its Present_Value, read with values on
    /// a [`commandable`](Self::commandable) object: an Unsigned from 1 to
    /// 16, or NULL when no priority holds a command and Relinquish_Default
    /// applies, as the object's Current_Command_Priority says it. An
    /// output (or Access Door, or lighting output) gives it as that
    /// property. The 2020 tables require it there, but it is treated as
    /// optional, since a device built to an earlier revision may not have
    /// it: `None` then, with no problem. A value object's comes from the
    /// Priority_Array already
    /// read, the first slot that holds a command, at no request of its
    /// own. Its text names the priority (`8 (operator, manual)`, `10 (site
    /// defined)`) or says `relinquish default`. JSON: `{"value": "8
    /// (operator, manual)", "decoded": 8}`, `decoded` null when
    /// relinquished.
    #[serde(serialize_with = "write::value")]
    pub command_priority: Option<Value>,
    /// Its Status_Flags, read with values, its text and JSON naming the
    /// flags set (`in-alarm | fault`); [`flags`](Self::flags) gives them
    /// typed. One that isn't a bit string is a problem instead.
    #[serde(serialize_with = "write::value")]
    pub status_flags: Option<Value>,
    /// Its Event_State, read with values, named (`normal`, `offnormal`);
    /// required on the types whose property tables list it so. One that
    /// isn't an Enumerated is a problem instead.
    #[serde(serialize_with = "write::value")]
    pub event_state: Option<Value>,
    /// Its Reliability, read with values, named (`no-fault-detected`,
    /// `open-loop`). Treated as optional on every type, though some tables
    /// require it (Access Door, Schedule, Network Port and others), since
    /// a device built to an earlier revision may not have it. One that
    /// isn't an Enumerated is a problem instead.
    #[serde(serialize_with = "write::value")]
    pub reliability: Option<Value>,
    /// The properties asked for that gave no usable value, in the order
    /// asked: one the device refused, one whose value isn't of the
    /// datatype the property has on the object's type, named with what
    /// came back (`not a Real (got an Unsigned, 5)`), one whose octets
    /// can't be decoded (`can't be decoded: a character string in JIS X
    /// 0208`), and each property of an object the device doesn't have or
    /// left out of its answer. A State_Text entry that isn't a string is
    /// one too (`state 2 is not a character string (got an Unsigned,
    /// 5)`), the entry held as an empty text.
    ///
    /// So is a value kept as the device gave it that a point list builder
    /// (`bacnet_objects::points::build_database`) would refuse, so the
    /// inventory says so before a rebuild does: on the types a point list
    /// holds, every refusal the builder can give the object's one point.
    /// That is an empty Object_Name, Units past 65535, a state count
    /// outside 1 to 1024 or State_Text with other than one label per
    /// state, an analog setting that isn't a finite number, a negative
    /// COV_Increment, Min_Pres_Value above Max_Pres_Value, a Polarity past
    /// 1, and on a point that holds a relinquish default (an output, or a
    /// value with a Priority_Array) a binary one past 1 or a multi-state
    /// one that names no state (`not one of the states 1 to 3`). A name
    /// another object also has is the builder's to find.
    ///
    /// An optional property the object simply lacks (the device says it
    /// is an unknown property) is not a problem.
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
            device_type: None,
            profile_name: None,
            units: None,
            state_texts: StateTexts::default(),
            number_of_states: None,
            commandable: false,
            relinquish_default: None,
            cov_increment: None,
            min_present_value: None,
            max_present_value: None,
            polarity: None,
            present_value: None,
            command_priority: None,
            status_flags: None,
            event_state: None,
            reliability: None,
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

    /// Its [`status_flags`](Self::status_flags) as the four flags, each by
    /// name: [`StatusFlags::IN_ALARM`], `FAULT`, `OVERRIDDEN` and
    /// `OUT_OF_SERVICE`. A flag the device's bit string is too short to
    /// hold reads as clear. `None` when the flags weren't read (no
    /// [`values`](InventoryOptions::values), or a type without them) or
    /// didn't come as a bit string, which is a
    /// [`problem`](Self::problem) then and leaves the field empty.
    ///
    /// ```
    /// use bacnet_ops::inventory::Inventory;
    /// use bacnet_types::bitstring::StatusFlags;
    ///
    /// /// The names of an inventory's points in fault, read with values.
    /// fn faulted(inventory: &Inventory) -> Vec<&str> {
    ///     let fault = |flags: StatusFlags| flags.contains(StatusFlags::FAULT);
    ///     inventory
    ///         .points
    ///         .iter()
    ///         .filter(|point| point.flags().is_some_and(fault))
    ///         .filter_map(|point| point.name.as_deref())
    ///         .collect()
    /// }
    /// ```
    pub fn flags(&self) -> Option<StatusFlags> {
        match self.status_flags.as_ref()?.decode().ok()? {
            PropertyValue::BitString { data, .. } => Some(status_flags_from_bacnet(&data)),
            _ => None,
        }
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
    /// whether one object reports changes is known once a watch has asked
    /// it, or, for an analog object or an integer value read with
    /// [`settings`](InventoryOptions::settings), from whether it has a
    /// [`cov_increment`](PointRecord::cov_increment).
    pub cov: CovSupport,
    /// The objects read, in Object_List order.
    pub points: Vec<PointRecord>,
    /// The objects of the selected types not read because the reading
    /// stopped, or a raw read's stop flag ended it
    /// ([`raw::InventoryHalt::Cancelled`](InventoryHalt::Cancelled)),
    /// in Object_List order; empty when neither did.
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
                InventoryHalt::Cancelled(_) => device_error(Halt::Cancelled),
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

    /// The records of `objects`, read as [`inventory`](Self::inventory)
    /// reads a device's objects under `options` (whose `types` this
    /// ignores: the objects are the caller's choice): the same records and
    /// problems, one for each object asked, in the order asked, an object
    /// named twice read twice. For a front end that pages a device's
    /// objects itself, a slice of its Object_List a call; nothing else is
    /// read, the summary and the list included.
    ///
    /// # What it costs
    ///
    /// The objects go in batches, filled in the order asked up to 16
    /// objects or 32 properties as the inventory's are ([module
    /// docs](crate::inventory)): one ReadPropertyMultiple a batch, so N
    /// objects of p properties each take N divided by the smaller of 16
    /// and 32/p, rounded up (ten analog inputs a request, four with
    /// values). Its fallbacks are the inventory's: a batch of several
    /// objects whose answer the device can't send for its size is asked
    /// again an object at a time, and the batches after it ask for half
    /// its properties, a limit the client remembers for the device
    /// ([`Client::rpm_limit`](crate::Client::rpm_limit)), so later calls,
    /// under any options, start there; a batch the device fails as a whole
    /// is read by ReadProperty; a device
    /// that refuses ReadPropertyMultiple costs that refused request once
    /// (the client remembers it) and then a ReadProperty a property. A
    /// handle made from an address whose instance isn't known yet costs one
    /// more, as the inventory does. [`point_batch`](Self::point_batch) says
    /// how many objects a call reads in one request, for a caller that
    /// stops between calls.
    ///
    /// A device that stops answering, or whose route is lost, part way
    /// leaves the records read before it, the objects not read
    /// ([`PointRecords::unread`]) and why ([`PointRecords::stopped`]): by
    /// ReadPropertyMultiple a batch is read whole or not at all, and an
    /// object read only in part by ReadProperty is not a record. A device
    /// that doesn't answer at all is `stopped` with no record; a handle
    /// that follows its instance looks for it once then, as any read does,
    /// and reads again where it moved to.
    ///
    /// The objects are named as anywhere else on a handle: an
    /// [`ObjectIdentifier`] or its text (`"ai:1"`, `"analog-value,2"`).
    ///
    /// # Errors
    ///
    /// Whatever is decided before a request is sent, with nothing sent: text
    /// that names no object ([`OpsError::Parse`]), a device that can't be
    /// placed, an instance two addresses claim, or a routed address with no
    /// route ([`OpsError::UnknownDevice`], [`OpsError::Ambiguous`]). And a
    /// request the client or the link failed itself before any record was
    /// read ([`OpsError::Client`]); after that it ends with the records,
    /// the error in [`PointRecords::stopped`].
    pub async fn point_records<O: IntoObject>(
        &self,
        objects: impl IntoIterator<Item = O>,
        options: &InventoryOptions,
    ) -> Result<PointRecords, OpsError> {
        let objects = objects
            .into_iter()
            .map(IntoObject::into_object)
            .collect::<Result<Vec<_>, _>>()?;
        if objects.is_empty() {
            return Ok(PointRecords::default());
        }
        let instance = self.resolve_instance().await?;
        let mut address = self.target().await?;
        let mut read = self
            .records_at(&address, instance, &objects, options)
            .await?;
        if read.points.is_empty()
            && let Some(error) = read.stopped.take()
        {
            let stopped = Err::<(), _>(error);
            match self.retry(&address, &stopped).await? {
                Some(to) => {
                    read = self.records_at(&to, instance, &objects, options).await?;
                    address = to;
                }
                None => read.stopped = stopped.err(),
            }
        }
        // The client's readers have no stop flag that anything sets.
        debug_assert!(
            !read.cancelled,
            "a client's read is never stopped by its flag"
        );
        Ok(PointRecords {
            unread: objects[read.points.len()..].to_vec(),
            points: read.points,
            stopped: read.stopped.map(|error| OpsError::at(&address, error)),
        })
    }

    /// How many of `objects`, from the first, [`point_records`](Self::point_records)
    /// reads in its first ReadPropertyMultiple under `options`: up to 16
    /// objects or 32 properties ([`properties`] counts an object's), or the
    /// fewer properties the device takes
    /// ([`Client::rpm_limit`](crate::Client::rpm_limit)), and at least one
    /// object; none of none. One object once the device has refused
    /// ReadPropertyMultiple: each property is then a ReadProperty of its
    /// own, so reading more at once saves nothing. A caller that pages
    /// with a deadline and gives each call that many loses at most one
    /// request's work to a call it drops. Sends nothing.
    pub fn point_batch(&self, objects: &[ObjectIdentifier], options: &InventoryOptions) -> usize {
        let at = self.address();
        if objects.is_empty() || self.client().refused_rpm(&at) {
            return objects.len().min(1);
        }
        one_batch(objects, self.sizing(&at), options, None).0
    }

    /// How point records are batched at `at`: the device's limit, if it
    /// has one.
    fn sizing(&self, at: &DeviceAddress) -> Sizing {
        let limit = self.client().rpm_limit(at);
        Sizing::pager(limit.unwrap_or(usize::MAX))
    }

    /// `objects` read at `at`, remembering a refusal of RPM and a smaller
    /// limit.
    async fn records_at(
        &self,
        at: &DeviceAddress,
        instance: u32,
        objects: &[ObjectIdentifier],
        options: &InventoryOptions,
    ) -> Result<Objects, OpsError> {
        let client = self.client();
        let mut reader = client.reader(at, Some(instance));
        let sizing = self.sizing(at);
        let read = read_objects(&mut reader, objects, options, None, sizing).await;
        client.settle(&reader);
        let read = read.map_err(|error| OpsError::at(at, error))?;
        client.settle_rpm_limit(at, instance, read.sizing.properties);
        Ok(read)
    }
}

/// What [`Device::point_records`] read: a record for each object asked,
/// in the order asked, until the device stopped answering.
///
/// Not `Clone`: `stopped` holds the low-level client's error, which can
/// hold the operating system's.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct PointRecords {
    /// The records read, in the order asked: those of the first
    /// `points.len()` objects.
    pub points: Vec<PointRecord>,
    /// The objects after them, not read because the reading stopped, in
    /// the order asked; empty when it didn't.
    pub unread: Vec<ObjectIdentifier>,
    /// Why the reading stopped early: the device went silent
    /// ([`OpsError::unreachable`]), or the client or the link failed a
    /// later request itself, the route to it lost included. `points`
    /// keeps what was read before it, possibly nothing.
    pub stopped: Option<OpsError>,
}

#[cfg(test)]
pub(crate) mod test_lab;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod limit_tests;

#[cfg(test)]
mod points_tests;

#[cfg(test)]
mod record_tests;

#[cfg(test)]
mod routed_tests;

#[cfg(test)]
mod settings_tests;

#[cfg(test)]
mod state_tests;

#[cfg(test)]
mod write_tests;
