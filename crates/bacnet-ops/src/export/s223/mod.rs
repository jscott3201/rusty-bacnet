//! An experimental export of a device's objects as a Standard 223 model in
//! Turtle (#2436).
//!
//! Proposed ASHRAE Standard 223 describes a building's equipment, its
//! connections and the properties they observe or actuate. Its draft
//! ontology links a model's property to a BACnet point with an external
//! reference: the device, the object, its name and the property. This
//! module writes that half: one 223 property per object, each with its
//! external reference, units where they map to QUDT, and the state names
//! of multi-state objects. It is a point list in 223 form. An integrator
//! still adds the equipment, the connections and the sensors.
//!
//! The work comes in two halves:
//!
//! - [`Engine::inventory`](crate::engine::Engine::inventory) (or
//!   [`read_inventory`] over a [`Reader`](crate::engine::Reader)) reads
//!   what the export needs into plain data, an [`Inventory`]: the device's
//!   name, its Object_List, then one request for each exported object, all
//!   one at a time through the reader's gate and rate cap;
//! - [`write_turtle`] and [`to_turtle`] turn an inventory into Turtle text
//!   with no I/O, so the same inventory always gives the same text.
//!
//! An inventory holds its points in the shape `bacnet_types::points` gives
//! a device's points (re-exported here), the shape
//! `bacnet_objects::points::build_database` builds a database from. Read at
//! [`Detail::Full`], [`Inventory::point_list`] gives the device as such a
//! list, within the limits its docs name: a field a list leaves out reads
//! back as the object's default, and a list read from another device need
//! not build. The other way, `Inventory::from(list)` takes a [`PointList`]
//! to Turtle without touching the wire.
//!
//! The draft can change, and so can this export: its name says
//! experimental in every front end. The output's header comment names the
//! ontology copy the mapping was written against and the golden files were
//! validated against ([`ONTOLOGY_SOURCE`], [`ONTOLOGY_VERSION`]).

mod inventory;
mod turtle;

pub use self::inventory::read_inventory;
pub use self::turtle::{Counts, to_turtle, write_turtle};
pub use bacnet_types::points::{
    Access, Analog, Binary, DeviceInfo, MultiState, Point, PointKind, PointList,
};

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

/// Where the ontology copy the mapping was written against came from; the
/// golden exports were validated against it.
pub const ONTOLOGY_SOURCE: &str = "https://open223.info/223p.ttl";

/// That copy's `owl:versionInfo`, shortened to its commit and date.
pub const ONTOLOGY_VERSION: &str = "git:master@6fdc11c6 (2026-10-06)";

/// The base IRI when the caller gives none: a documentation domain, so an
/// export nobody pointed at their own namespace is plain to see.
pub const DEFAULT_BASE_IRI: &str = "http://example.org/bacnet/";

/// The object types the export maps: the analog, binary and multi-state
/// inputs, outputs and values.
pub const EXPORTED_TYPES: [ObjectType; 9] = [
    ObjectType::ANALOG_INPUT,
    ObjectType::ANALOG_OUTPUT,
    ObjectType::ANALOG_VALUE,
    ObjectType::BINARY_INPUT,
    ObjectType::BINARY_OUTPUT,
    ObjectType::BINARY_VALUE,
    ObjectType::MULTI_STATE_INPUT,
    ObjectType::MULTI_STATE_OUTPUT,
    ObjectType::MULTI_STATE_VALUE,
];

/// How much of each object an inventory reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Detail {
    /// What the Turtle export uses, the default: the Device's Object_Name;
    /// each object's Object_Name and Description, an analog object's Units,
    /// a multi-state object's Number_Of_States and State_Text, and a
    /// value's Priority_Array. The optional settings of the point list are
    /// left `None`, and a commandable point's relinquish default too.
    #[default]
    Export,
    /// Everything a [`PointList`] holds, in the same requests: also the
    /// Device's Description; an analog object's COV_Increment,
    /// Min_Pres_Value and Max_Pres_Value; a binary object's Active_Text and
    /// Inactive_Text; and an output's or a value's Relinquish_Default. A
    /// property the device doesn't have reads as `None`.
    Full,
}

/// What an inventory reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryOptions {
    /// The object types to read, from [`EXPORTED_TYPES`]; the device's
    /// other objects are left out without a request.
    pub types: Vec<ObjectType>,
    /// How much of each object to read.
    pub detail: Detail,
}

impl Default for InventoryOptions {
    /// Every one of the [`EXPORTED_TYPES`], at [`Detail::Export`].
    fn default() -> Self {
        Self {
            types: EXPORTED_TYPES.to_vec(),
            detail: Detail::Export,
        }
    }
}

impl InventoryOptions {
    /// Options for `types` at [`Detail::Export`], refused when one isn't
    /// among the [`EXPORTED_TYPES`] or none is given.
    pub fn new(types: Vec<ObjectType>) -> Result<Self, String> {
        if types.is_empty() {
            return Err("name at least one object type to export".into());
        }
        if let Some(other) = types.iter().find(|t| !EXPORTED_TYPES.contains(t)) {
            return Err(format!(
                "{} objects can't be exported; the export maps analog, binary and multi-state \
                 inputs, outputs and values",
                crate::label::kebab(other)
            ));
        }
        Ok(Self {
            types,
            detail: Detail::Export,
        })
    }

    /// The same options, reading at `detail`.
    pub fn with_detail(mut self, detail: Detail) -> Self {
        self.detail = detail;
        self
    }
}

/// A device's objects as the export reads them: plain data, device text
/// exactly as the device sent it.
///
/// Read from a device, `device.name` is `None` when the Device gave no
/// name, and `device.description` is read only at [`Detail::Full`].
#[derive(Debug, Clone, PartialEq)]
pub struct Inventory {
    /// The Device object.
    pub device: DeviceInfo,
    /// The objects read, in Object_List order: a point each, or why its
    /// read failed.
    pub entries: Vec<Entry>,
    /// Why the reading stopped early: the device stopped answering (during
    /// the Object_List or after it) or the caller cancelled. The first
    /// reason is kept; the objects after it are not in `entries`.
    pub stopped: Option<String>,
}

impl Inventory {
    /// An inventory of `device` with no entry yet.
    pub fn new(device: DeviceInfo) -> Self {
        Self {
            device,
            entries: Vec::new(),
            stopped: None,
        }
    }

    /// The points read, in Object_List order.
    pub fn points(&self) -> impl Iterator<Item = &Point> {
        self.entries.iter().filter_map(|entry| match entry {
            Entry::Point(point) => Some(point),
            Entry::Failed { .. } => None,
        })
    }

    /// The device and the points read, as a point list; the objects whose
    /// read failed are left out.
    ///
    /// Read at [`Detail::Full`] from a device built from a list, it equals
    /// that list only when the list set every field, and even then a value
    /// given [`Access::Writable`] reads back [`Access::ReadOnly`]: a read
    /// can't tell the two apart. A field the list left out reads back as
    /// the object's default instead: an empty Description (the Device's
    /// too), COV_Increment 0.0, "Active" and "Inactive", "State 1" to
    /// "State N", the object's own relinquish default, and the identity's
    /// name for the Device.
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

impl From<PointList> for Inventory {
    /// The list's device and points, in its order, none failed: Turtle for a
    /// list that never touched the wire.
    fn from(list: PointList) -> Self {
        Self {
            device: list.device,
            entries: list.points.into_iter().map(Entry::Point).collect(),
            stopped: None,
        }
    }
}

/// One object of an [`Inventory`].
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    /// The object as read.
    Point(Point),
    /// Its read failed, or a property every object of its type has
    /// (Object_Name, an analog object's Units, a multi-state object's
    /// Number_Of_States) couldn't be read; the export skips it.
    Failed {
        /// The object.
        object: ObjectIdentifier,
        /// The property every object of its type has that couldn't be
        /// read; `None` when the read failed as a whole, or the type isn't
        /// one the export reads.
        property: Option<PropertyIdentifier>,
        /// Why: the device's error, naming the property when one property
        /// is at fault.
        error: String,
    },
}

impl Entry {
    /// The object.
    pub fn object(&self) -> ObjectIdentifier {
        match self {
            Self::Point(point) => point.object,
            Self::Failed { object, .. } => *object,
        }
    }

    /// The point, unless its read failed.
    pub fn point(&self) -> Option<&Point> {
        match self {
            Self::Point(point) => Some(point),
            Self::Failed { .. } => None,
        }
    }

    /// Why its read failed, `None` when it didn't.
    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Point(_) => None,
            Self::Failed { error, .. } => Some(error),
        }
    }
}

/// A command priority to name in each commandable point's reference, 1 to
/// 16 (Clause 19.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WritePriority(u8);

impl WritePriority {
    /// `priority`, refused outside 1 to 16.
    pub fn new(priority: u8) -> Result<Self, String> {
        let range = 1..=crate::label::PRIORITIES;
        if range.contains(&usize::from(priority)) {
            Ok(Self(priority))
        } else {
            Err(format!("write priority {priority} is not 1 to 16"))
        }
    }

    /// The priority.
    pub fn get(self) -> u8 {
        self.0
    }
}

/// How an inventory becomes Turtle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurtleOptions {
    base_iri: String,
    write_priority: Option<WritePriority>,
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
    pub fn new(base_iri: &str, write_priority: Option<WritePriority>) -> Result<Self, String> {
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
    pub fn write_priority(&self) -> Option<WritePriority> {
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
