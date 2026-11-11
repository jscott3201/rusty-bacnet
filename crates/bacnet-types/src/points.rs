//! A device's points as plain data (#2445): the Device's name and
//! description, then each object's identifier, name, description and the
//! settings of its family, in Object_List order.
//!
//! A [`PointList`] only describes a device. `bacnet_objects::points` builds
//! an object database from one, and tools that read a device can report what
//! they find in the same shape, so a host shares these types without
//! compiling the object model. They are owned data, so they need `alloc`;
//! nothing here does I/O or allocates beyond the strings and the one list.
//!
//! Version 1 covers the analog, binary and multi-state Input, Output and
//! Value objects (Clauses 12.2 to 12.4, 12.6 to 12.8 and 12.18 to 12.20).
//! The fields follow one rule:
//!
//! - A property every object of the type has is a plain field: Object_Name,
//!   an analog object's Units, a multi-state object's Number_Of_States.
//! - A property the type may go without is an `Option`, or for State_Text
//!   an empty list. `None` leaves the object as it starts.
//! - [`Access`] says how Present_Value is written. Priority_Array and
//!   Relinquish_Default come with a commandable Present_Value and only with
//!   one, so the relinquish default lives in [`Access::Commandable`].
//!
//! Every identifier is given, never assigned. Building the same list twice
//! gives the same objects, and adding a point doesn't renumber the others,
//! so a reference a head end or a model keeps by type and instance stays
//! good.
//!
//! The family structs and [`PointKind`] are `#[non_exhaustive]`: a later
//! version adds families and optional properties without breaking a match or
//! a constructor call. Build them with their `new` and `with_*` methods.
//!
//! ```
//! use bacnet_types::enums::{BinaryPV, EngineeringUnits, ObjectType};
//! use bacnet_types::points::{Access, Analog, Binary, DeviceInfo, Point, PointList};
//! use bacnet_types::primitives::ObjectIdentifier;
//!
//! let supply = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)?;
//! let fan = ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1)?;
//! let list = PointList::new(
//!     DeviceInfo::new(1001).with_name("AHU-1"),
//!     vec![
//!         Point::new(
//!             supply,
//!             "supply-air-temp",
//!             Analog::new(EngineeringUnits::DEGREES_CELSIUS, Access::ReadOnly)
//!                 .with_cov_increment(0.5),
//!         )
//!         .with_description("Supply air temperature"),
//!         Point::new(
//!             fan,
//!             "supply-fan",
//!             Binary::new(Access::Commandable {
//!                 relinquish_default: Some(BinaryPV::INACTIVE),
//!             }),
//!         ),
//!     ],
//! );
//! assert_eq!(list.points[0].object, supply);
//! # Ok::<(), bacnet_types::error::Error>(())
//! ```

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};

use crate::enums::{BinaryPV, EngineeringUnits};
use crate::primitives::ObjectIdentifier;

/// A device and its points: what `bacnet_objects::points::build_database`
/// builds a database from. The builder takes the list by value and moves
/// each point's strings into its object rather than copying them; the
/// database's name index keeps its own copy of every name, and the
/// Device's name is copied on its way through the identity.
#[derive(Debug, Clone, PartialEq)]
pub struct PointList {
    /// The Device object.
    pub device: DeviceInfo,
    /// The points, in the order the Device's Object_List gives them after
    /// the Device and its Network Ports.
    pub points: Vec<Point>,
}

impl PointList {
    /// A list of `points` on `device`.
    pub const fn new(device: DeviceInfo, points: Vec<Point>) -> Self {
        Self { device, points }
    }
}

/// The Device object's part of a [`PointList`]. The rest of the Device
/// (vendor, APDU settings, Network Ports) comes from the identity the
/// database is built for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The Device's instance. A builder checks it against the identity's.
    pub instance: u32,
    /// Its Object_Name; `None` keeps the identity's.
    pub name: Option<String>,
    /// Its Description; `None` leaves it empty.
    pub description: Option<String>,
}

impl DeviceInfo {
    /// The Device `instance`, with the identity's name and no description.
    pub const fn new(instance: u32) -> Self {
        Self {
            instance,
            name: None,
            description: None,
        }
    }

    /// Give the Device this Object_Name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Give the Device this Description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// One object of a [`PointList`].
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    /// Its type and instance, always given.
    pub object: ObjectIdentifier,
    /// Its Object_Name. A builder takes any name of at least one character
    /// that no other point, the Device or a Network Port has; it sets no
    /// limit on length or characters, so a long dotted path is fine.
    pub name: String,
    /// Its Description; `None` leaves it empty.
    pub description: Option<String>,
    /// The settings of its family, which must fit its type.
    pub kind: PointKind,
}

impl Point {
    /// The point `object` named `name`, with no description.
    pub fn new(
        object: ObjectIdentifier,
        name: impl Into<String>,
        kind: impl Into<PointKind>,
    ) -> Self {
        Self {
            object,
            name: name.into(),
            description: None,
            kind: kind.into(),
        }
    }

    /// Give the point this Description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// The family a point belongs to, with the settings that family has.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PointKind {
    /// An Analog Input, Output or Value.
    Analog(Analog),
    /// A Binary Input, Output or Value.
    Binary(Binary),
    /// A Multi-state Input, Output or Value.
    MultiState(MultiState),
}

impl From<Analog> for PointKind {
    fn from(analog: Analog) -> Self {
        Self::Analog(analog)
    }
}

impl From<Binary> for PointKind {
    fn from(binary: Binary) -> Self {
        Self::Binary(binary)
    }
}

impl From<MultiState> for PointKind {
    fn from(multi_state: MultiState) -> Self {
        Self::MultiState(multi_state)
    }
}

/// How a point's Present_Value is written, and with it whether the object
/// has Priority_Array and Relinquish_Default.
///
/// An Input is always [`ReadOnly`](Self::ReadOnly) and an Output always
/// [`Commandable`](Self::Commandable); a Value may be any of the three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access<T> {
    /// The application owns Present_Value; a peer may write it only for a
    /// test, with Out_Of_Service TRUE.
    ReadOnly,
    /// A peer's write replaces Present_Value. Only a Value can be this.
    Writable,
    /// Peers command Present_Value through Priority_Array, and with every
    /// priority relinquished it is Relinquish_Default.
    Commandable {
        /// Relinquish_Default; `None` keeps the object's own default.
        relinquish_default: Option<T>,
    },
}

/// The settings of an Analog Input, Output or Value.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Analog {
    /// Units, the engineering units of Present_Value.
    pub units: EngineeringUnits,
    /// COV_Increment, the smallest change of Present_Value that sends a
    /// change-of-value notification; `None` keeps the object's default.
    pub cov_increment: Option<f32>,
    /// Min_Pres_Value, the lowest Present_Value the object can report;
    /// `None` leaves the property out.
    pub min_pres_value: Option<f32>,
    /// Max_Pres_Value, the highest Present_Value the object can report;
    /// `None` leaves the property out.
    pub max_pres_value: Option<f32>,
    /// How Present_Value is written.
    pub access: Access<f32>,
}

impl Analog {
    /// An analog point in `units`, written as `access` says, with every
    /// optional property left as the object starts.
    pub const fn new(units: EngineeringUnits, access: Access<f32>) -> Self {
        Self {
            units,
            cov_increment: None,
            min_pres_value: None,
            max_pres_value: None,
            access,
        }
    }

    /// Give the point this COV_Increment.
    pub const fn with_cov_increment(mut self, increment: f32) -> Self {
        self.cov_increment = Some(increment);
        self
    }

    /// Give the point this Min_Pres_Value.
    pub const fn with_min_pres_value(mut self, min: f32) -> Self {
        self.min_pres_value = Some(min);
        self
    }

    /// Give the point this Max_Pres_Value.
    pub const fn with_max_pres_value(mut self, max: f32) -> Self {
        self.max_pres_value = Some(max);
        self
    }
}

/// The settings of a Binary Input, Output or Value.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Binary {
    /// Active_Text, the label of an ACTIVE Present_Value; `None` keeps the
    /// object's default.
    pub active_text: Option<String>,
    /// Inactive_Text, the label of an INACTIVE Present_Value; `None` keeps
    /// the object's default.
    pub inactive_text: Option<String>,
    /// How Present_Value is written.
    pub access: Access<BinaryPV>,
}

impl Binary {
    /// A binary point written as `access` says, with its texts left as the
    /// object starts.
    pub const fn new(access: Access<BinaryPV>) -> Self {
        Self {
            active_text: None,
            inactive_text: None,
            access,
        }
    }

    /// Give the point this Active_Text.
    pub fn with_active_text(mut self, text: impl Into<String>) -> Self {
        self.active_text = Some(text.into());
        self
    }

    /// Give the point this Inactive_Text.
    pub fn with_inactive_text(mut self, text: impl Into<String>) -> Self {
        self.inactive_text = Some(text.into());
        self
    }
}

/// The settings of a Multi-state Input, Output or Value.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MultiState {
    /// Number_Of_States. Present_Value runs from 1 to this.
    pub number_of_states: u32,
    /// State_Text, the label of each state, state 1 first: empty keeps the
    /// object's default labels, otherwise exactly one per state.
    pub state_texts: Vec<String>,
    /// How Present_Value is written; a relinquish default names a state.
    pub access: Access<u32>,
}

impl MultiState {
    /// A multi-state point with `number_of_states` states, written as
    /// `access` says, with the object's default labels.
    pub const fn new(number_of_states: u32, access: Access<u32>) -> Self {
        Self {
            number_of_states,
            state_texts: Vec::new(),
            access,
        }
    }

    /// Give the point these labels, state 1 first: one per state. Owned
    /// strings move in as they are.
    pub fn with_state_texts(mut self, texts: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.state_texts = texts.into_iter().map(Into::into).collect();
        self
    }
}

#[cfg(test)]
mod tests;
