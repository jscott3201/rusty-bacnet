//! An experimental export of a device's objects as a Standard 223 model in
//! Turtle (#2436).
//!
//! Proposed ASHRAE Standard 223 describes a building's equipment, its
//! connections and the properties they observe or actuate. Its draft
//! ontology links a model's property to a BACnet point with an external
//! reference: the device, the object, its name and the property. This
//! module writes that half: one 223 property per object of the
//! [`EXPORTED_TYPES`], each with its external reference, units where they
//! map to QUDT, and the state names of multi-state objects. It is a point
//! list in 223 form. An integrator still adds the equipment, the
//! connections and the sensors.
//!
//! The work comes in two halves:
//!
//! - [`Device::inventory`](crate::Device::inventory) reads the device's
//!   objects into [`PointRecord`]s, as any inventory does
//!   ([`inventory`](crate::inventory)); [`inventory_options`] names the
//!   types the export maps;
//! - a [`Model`] is that inventory in the export's terms, each record a
//!   point ([`PointRecord::to_point`]) or the reason it has none, and
//!   [`write_turtle`] and [`to_turtle`] turn a model into Turtle text with
//!   no I/O, so the same model always gives the same text.
//!
//! A model holds its points in the shape `bacnet_types::points` gives a
//! device's points (re-exported here), the shape
//! `bacnet_objects::points::build_database` builds a database from. Read
//! with [`point_list_options`], [`Model::point_list`] gives the device as
//! such a list, within the limits its docs name: a field a list leaves out
//! reads back as the object's default, and a list read from another device
//! need not build. The other way, `Model::from(list)` takes a
//! [`PointList`] to Turtle without touching the wire.
//!
//! The draft can change, and so can this export: its name says
//! experimental in every front end. The output's header comment names the
//! ontology copy the mapping was written against and the golden files were
//! validated against ([`ONTOLOGY_SOURCE`], [`ONTOLOGY_VERSION`]).

mod inventory;
mod turtle;

pub use self::inventory::exported;
pub use self::turtle::{Counts, to_turtle, write_turtle};
pub use bacnet_types::points::{
    Access, Analog, Binary, DeviceInfo, Integer, MultiState, Point, PointKind, PointList,
};

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use crate::inventory::{Inventory, InventoryOptions, PointRecord};
use crate::write::Priority;

/// Where the ontology copy the mapping was written against came from; the
/// golden exports were validated against it.
pub const ONTOLOGY_SOURCE: &str = "https://open223.info/223p.ttl";

/// That copy's `owl:versionInfo`, shortened to its commit and date.
pub const ONTOLOGY_VERSION: &str = "git:master@6fdc11c6 (2026-10-06)";

/// The base IRI when the caller gives none: a documentation domain, so an
/// export nobody pointed at their own namespace is plain to see.
pub const DEFAULT_BASE_IRI: &str = "http://example.org/bacnet/";

/// The object types the export maps, and reads by default: the analog,
/// binary and multi-state inputs, outputs and values, and the Integer Value
/// (#2456), which maps as an analog value does.
pub const EXPORTED_TYPES: [ObjectType; 10] = [
    ObjectType::ANALOG_INPUT,
    ObjectType::ANALOG_OUTPUT,
    ObjectType::ANALOG_VALUE,
    ObjectType::BINARY_INPUT,
    ObjectType::BINARY_OUTPUT,
    ObjectType::BINARY_VALUE,
    ObjectType::MULTI_STATE_INPUT,
    ObjectType::MULTI_STATE_OUTPUT,
    ObjectType::MULTI_STATE_VALUE,
    ObjectType::INTEGER_VALUE,
];

/// The options of an inventory for the export: the objects of `types`, or
/// of every one of the [`EXPORTED_TYPES`] for `None`, with no values and
/// no settings.
///
/// # Errors
///
/// A type that isn't among the [`EXPORTED_TYPES`], or an empty list, in a
/// sentence for the person who asked.
pub fn inventory_options(types: Option<Vec<ObjectType>>) -> Result<InventoryOptions, String> {
    let Some(types) = types else {
        return Ok(InventoryOptions::new().with_types(EXPORTED_TYPES));
    };
    if types.is_empty() {
        return Err("name at least one object type to export".into());
    }
    if let Some(other) = types.iter().find(|t| !EXPORTED_TYPES.contains(t)) {
        return Err(format!(
            "{} objects can't be exported; the export maps analog, binary and multi-state \
             inputs, outputs and values, and integer values",
            crate::label::kebab(other)
        ));
    }
    Ok(InventoryOptions::new().with_types(types))
}

/// The options of an inventory that reads everything a [`PointList`]
/// holds: every one of the [`EXPORTED_TYPES`] with its settings
/// (COV_Increment, Min_Pres_Value, Max_Pres_Value, Relinquish_Default),
/// and the Device object, whose record gives the list's Device its
/// Description.
pub fn point_list_options() -> InventoryOptions {
    let types = EXPORTED_TYPES.into_iter().chain([ObjectType::DEVICE]);
    InventoryOptions::new()
        .with_types(types)
        .with_settings(true)
}

/// A device's points in the export's terms: plain data, device text
/// exactly as the device sent it. Made from an [`Inventory`]
/// (`Model::from(&inventory)`), from a [`PointList`], or by hand.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// The Device object: `name` is `None` when the Device gave no name,
    /// and `description` is set only from an inventory that read the
    /// Device object's own record ([`point_list_options`]).
    pub device: DeviceInfo,
    /// The objects, in Object_List order: a point each, or why it has
    /// none.
    pub entries: Vec<Result<Point, Skipped>>,
    /// Why the reading stopped early, when it did: the objects after that
    /// are not in `entries`. The Turtle's header says so.
    pub stopped: Option<String>,
}

impl Model {
    /// A model of `device` with no entry yet.
    pub fn new(device: DeviceInfo) -> Self {
        Self {
            device,
            entries: Vec::new(),
            stopped: None,
        }
    }

    /// The points, in order.
    pub fn points(&self) -> impl Iterator<Item = &Point> {
        self.entries.iter().filter_map(|entry| entry.as_ref().ok())
    }

    /// The device and the points, as a point list; the objects without a
    /// point are left out.
    ///
    /// Read with [`point_list_options`] from a device built from a list,
    /// it equals that list only when the list set every field, and even
    /// then a value given [`Access::Writable`] reads back
    /// [`Access::ReadOnly`]: a read can't tell the two apart. A field the
    /// list left out reads back as the object's default instead: an empty
    /// Description (the Device's too), COV_Increment 0, "Active" and
    /// "Inactive", "State 1" to "State N", the object's own relinquish
    /// default, and the identity's name for the Device.
    ///
    /// A list read from another device need not build: it can hold what
    /// `bacnet_objects::points::build_database` refuses, such as State_Text
    /// with other than one label per state, a Real that isn't finite,
    /// Min_Pres_Value above Max_Pres_Value, or a multi-state relinquish
    /// default that names no state. The builder stops at the first refusal.
    pub fn point_list(&self) -> PointList {
        PointList::new(self.device.clone(), self.points().cloned().collect())
    }
}

impl From<PointList> for Model {
    /// The list's device and points, in its order, none skipped: Turtle for
    /// a list that never touched the wire.
    fn from(list: PointList) -> Self {
        Self {
            device: list.device,
            entries: list.points.into_iter().map(Ok).collect(),
            stopped: None,
        }
    }
}

/// An object a [`Model`] has no point for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// The object.
    pub object: ObjectIdentifier,
    /// The property every object of its type has that couldn't be read
    /// (Object_Name, the Units of an analog object or an Integer Value, a
    /// multi-state object's Number_Of_States); `None` when its read failed
    /// as a whole, or the type isn't one the export maps.
    pub property: Option<PropertyIdentifier>,
    /// Why: the device's error, naming the property when one property is
    /// at fault.
    pub error: String,
}

/// How a model becomes Turtle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurtleOptions {
    base_iri: String,
    write_priority: Option<Priority>,
}

impl Default for TurtleOptions {
    /// [`DEFAULT_BASE_IRI`], and no write priority.
    fn default() -> Self {
        Self {
            base_iri: DEFAULT_BASE_IRI.into(),
            write_priority: None,
        }
    }
}

impl TurtleOptions {
    /// Options under `base_iri`; each device gets the namespace
    /// `<base_iri>device-<instance>/`. `write_priority` goes in each
    /// commandable point's reference.
    ///
    /// The base must be ASCII, start with a scheme (`http:`, `urn:`, ...),
    /// hold no space, control character, backtick or any of `<>"{}|^\`,
    /// write `%` only as the start of a two-digit hex escape, hold at most one `#`,
    /// and end in `/` or `#`. A refusal says which rule failed.
    pub fn new(base_iri: &str, write_priority: Option<Priority>) -> Result<Self, String> {
        check_base_iri(base_iri).map_err(|why| format!("base IRI {base_iri:?} {why}"))?;
        Ok(Self {
            base_iri: base_iri.into(),
            write_priority,
        })
    }

    /// The base IRI.
    pub fn base_iri(&self) -> &str {
        &self.base_iri
    }

    /// The write priority, if any.
    pub fn write_priority(&self) -> Option<Priority> {
        self.write_priority
    }
}

/// Why `base` can't be a base IRI, per [`TurtleOptions::new`].
fn check_base_iri(base: &str) -> Result<(), String> {
    if let Some(bad) = base.chars().find(|c| !c.is_ascii()) {
        return Err(format!(
            "holds the non-ASCII character {bad:?}; percent-encode it"
        ));
    }
    let scheme = base.split_once(':').map(|(scheme, _)| scheme);
    let scheme_ok = scheme.is_some_and(|s| {
        s.starts_with(|c: char| c.is_ascii_alphabetic())
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    });
    if !scheme_ok {
        return Err("is not an absolute IRI: it has no scheme such as http:".into());
    }
    if let Some(bad) = base
        .chars()
        .find(|c| c.is_ascii_control() || *c == ' ' || "<>\"{}|^`\\".contains(*c))
    {
        return Err(format!("holds {bad:?}, which an IRI can't"));
    }
    let bytes = base.as_bytes();
    for (at, _) in base.match_indices('%') {
        let hex = bytes.get(at + 1..at + 3);
        if !hex.is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit)) {
            return Err(format!(
                "has a malformed percent escape at byte {at}: % takes two hex digits"
            ));
        }
    }
    if base.matches('#').count() > 1 {
        return Err("has more than one #".into());
    }
    if !base.ends_with(['/', '#']) {
        return Err("must end in / or #".into());
    }
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod testing;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod golden_tests;

#[cfg(test)]
mod inventory_tests;

#[cfg(test)]
mod round_trip_tests;
