//! What a watch names and reports: a [`Point`] by its device's address,
//! and [`Why`] a point is polled, unreachable or given up.

use std::fmt;

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;

use crate::device::DeviceAddress;
use crate::label::kebab;

/// What a watch follows on an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Watched {
    /// The object: SubscribeCOV, what [`Reads::object`](super::hub::Reads::object)
    /// names read and polled. Its value is the Present_Value.
    Object,
    /// One property, or one element of it: SubscribeCOVProperty.
    Property {
        /// The property.
        property: PropertyIdentifier,
        /// The array element, if one.
        index: Option<u32>,
    },
}

/// A watched point: an object of the device at an address, whole or one
/// property. The hub keys its entries and its per-device schedulers on
/// it, so two devices sharing an instance are two points, and a device
/// announcing itself again at the same address keeps its entries.
///
/// [`Device::point`](crate::Device::point) makes one from the grammar
/// (`"ai:1"`, `"ao:2 priority-array[8]"`). [`Display`](fmt::Display) writes
/// it back in that grammar after the address: `192.168.1.10:47808
/// analog-input:1`, `2:0x05 analog-output:2 priority-array[8]`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Point {
    /// Where the device answers.
    pub device: DeviceAddress,
    /// The object.
    pub object: ObjectIdentifier,
    /// The object or one property.
    pub watched: Watched,
}

impl Point {
    /// The property whose value an update carries: an object watch's
    /// Present_Value, or the watched property and its element.
    pub fn property(&self) -> (PropertyIdentifier, Option<u32>) {
        match self.watched {
            Watched::Object => (PropertyIdentifier::PRESENT_VALUE, None),
            Watched::Property { property, index } => (property, index),
        }
    }
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let object = self.object;
        write!(
            f,
            "{} {}:{}",
            self.device,
            kebab(object.object_type()),
            object.instance_number()
        )?;
        match self.watched {
            Watched::Object => Ok(()),
            Watched::Property {
                property,
                index: None,
            } => write!(f, " {}", kebab(property)),
            Watched::Property {
                property,
                index: Some(index),
            } => write!(f, " {}[{index}]", kebab(property)),
        }
    }
}

/// Why a point is polled, unreachable or given up: inside a
/// [`State`](super::State), and the hub's
/// [`Snapshot::why`](super::hub::Snapshot::why).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Why {
    /// The device refused the subscription, with its error; or it refused
    /// the service itself for another point before, so it wasn't asked
    /// again ([`CovSupport::Refused`](super::CovSupport::Refused)). Not
    /// asked again.
    Refused(String),
    /// The first read or the subscription had no answer, with the error:
    /// subscribed again after a backoff.
    NoAnswer(String),
    /// The device's subscription table was full, with its error: polled
    /// meanwhile, and asked again after the COV retry's backoff.
    Full(String),
    /// A renewal failed, with its error, or stopped.
    Lapsed(Option<String>),
    /// No route to the device's address is known: a remote network the
    /// discovery table holds no router for.
    Missing,
    /// The device's instance answers at `to` now, on positive evidence:
    /// the row at its old address names another instance and only `to`
    /// claims this one, or a handle following the instance moved there.
    /// The point is given up ([`State::Gone`](super::State::Gone)); watch
    /// the point at `to` instead. The hub doesn't follow it, but starts the
    /// point again if the old address answers for the instance again.
    Moved {
        /// Where the device answers now.
        to: DeviceAddress,
    },
}
