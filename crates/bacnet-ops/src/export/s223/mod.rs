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
//! The draft can change, and so can this export: its name says
//! experimental in every front end. The output's header comment names the
//! ontology copy it was checked against ([`ONTOLOGY_SOURCE`],
//! [`ONTOLOGY_VERSION`]).

mod inventory;
mod turtle;
mod units;

pub use self::inventory::read_inventory;
pub use self::turtle::{Counts, to_turtle, write_turtle};
pub use self::units::qudt_unit;

use bacnet_types::enums::{EngineeringUnits, ObjectType};
use bacnet_types::primitives::ObjectIdentifier;

/// Where the ontology copy the output was checked against came from.
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

/// What an inventory reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryOptions {
    /// The object types to read, from [`EXPORTED_TYPES`]; the device's
    /// other objects are left out without a request.
    pub types: Vec<ObjectType>,
}

impl Default for InventoryOptions {
    /// Every one of the [`EXPORTED_TYPES`].
    fn default() -> Self {
        Self {
            types: EXPORTED_TYPES.to_vec(),
        }
    }
}

impl InventoryOptions {
    /// Options for `types`, refused when one isn't among the
    /// [`EXPORTED_TYPES`] or none is given.
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
        Ok(Self { types })
    }
}

/// A device's objects as the export reads them: plain data, device text
/// exactly as the device sent it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    /// The device instance.
    pub device: u32,
    /// The Device object's Object_Name; `None` when it couldn't be read.
    pub device_name: Option<String>,
    /// The objects read, in Object_List order.
    pub objects: Vec<InventoryObject>,
    /// Why the reading stopped before the end of the list: the device
    /// stopped answering. The objects after it are not in `objects`.
    pub stopped: Option<String>,
}

/// One object of an [`Inventory`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryObject {
    /// The object.
    pub object: ObjectIdentifier,
    /// Its Object_Name; `None` when it couldn't be read.
    pub name: Option<String>,
    /// Its Description; `None` when it has none or couldn't be read.
    pub description: Option<String>,
    /// An analog object's Units; `None` when not read.
    pub units: Option<EngineeringUnits>,
    /// A multi-state object's Number_Of_States; `None` when not read.
    pub states: Option<u32>,
    /// A multi-state object's State_Text, state 1 first; an entry that
    /// wasn't a string is empty.
    pub state_texts: Vec<String>,
    /// Whether it has a Priority_Array: always for an output, and for a
    /// value when the device gave one.
    pub priority_array: bool,
    /// Why the object's read failed; the export skips such an object.
    pub error: Option<String>,
}

impl InventoryObject {
    /// An object with nothing read yet.
    pub fn new(object: ObjectIdentifier) -> Self {
        Self {
            object,
            name: None,
            description: None,
            units: None,
            states: None,
            state_texts: Vec::new(),
            priority_array: false,
            error: None,
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
    /// Options under `base_iri`, which must be an absolute IRI ending in
    /// `/` or `#` with no character Turtle refuses in an IRI; each device
    /// gets the namespace `<base_iri>device-<instance>/`. `write_priority`
    /// goes in each commandable point's reference.
    pub fn new(base_iri: &str, write_priority: Option<WritePriority>) -> Result<Self, String> {
        let scheme = base_iri.split_once(':').map(|(scheme, _)| scheme);
        let scheme_ok = scheme.is_some_and(|s| {
            s.starts_with(|c: char| c.is_ascii_alphabetic())
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        });
        if !scheme_ok {
            return Err(format!("base IRI {base_iri:?} is not an absolute IRI"));
        }
        if let Some(bad) = base_iri
            .chars()
            .find(|c| c.is_control() || c.is_whitespace() || "<>\"{}|^`\\".contains(*c))
        {
            return Err(format!("base IRI {base_iri:?} holds {bad:?}"));
        }
        if !base_iri.ends_with(['/', '#']) {
            return Err(format!("base IRI {base_iri:?} must end in / or #"));
        }
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

#[cfg(test)]
mod tests;

#[cfg(test)]
mod golden_tests;

#[cfg(test)]
mod inventory_tests;
