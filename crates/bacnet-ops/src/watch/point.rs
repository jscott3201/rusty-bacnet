//! What the hub watches and reports: a [`Point`] by its device's address,
//! and a point's [`Snapshot`] with how its watch goes ([`State`],
//! [`Source`], [`Why`]).

use std::fmt;

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;
use tokio::time::Instant;

use super::sample::Via;
use super::support::CovSupport;
use crate::device::DeviceAddress;
use crate::format::RawRow;
use crate::label::kebab;

/// What a watch follows on an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Watched {
    /// The object: SubscribeCOV, what [`Reads::object`](super::Reads::object)
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

/// Where a point's values come from now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// COV notifications under a managed subscription.
    Cov,
    /// Polls, because the device refused COV or stopped renewing it.
    Poll,
}

/// How a point's watch is going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Its first read hasn't finished.
    Starting,
    /// Subscribed, or the last poll answered.
    Live,
    /// The device answered the last read with an error for the whole
    /// object (see [`Snapshot::note`]).
    Failed,
    /// The device didn't answer the last request.
    Unreachable,
    /// The watch ended (the hub shut down).
    Stopped,
    /// Nothing watches it any more: no route to its address is known and
    /// [`WatchOptions::missing`](super::WatchOptions::missing) says not to
    /// wait ([`Why::Missing`]), or its device answers at another address
    /// now ([`Why::Moved`]). See [`Snapshot::note`].
    Gone,
}

/// Why a point is polled, unreachable or given up ([`Snapshot::why`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// The device refused the subscription or didn't answer it, with the
    /// error; or it refused the service itself for another point before,
    /// so it wasn't asked again ([`CovSupport::Refused`]).
    Refused(String),
    /// The first read had no answer, so no subscription was asked for.
    NoAnswer(String),
    /// A renewal failed, with its error, or stopped.
    Lapsed(Option<String>),
    /// No route to the device's address is known: a remote network the
    /// discovery table holds no router for.
    Missing,
    /// The device's instance answers at `to` now, while its address here
    /// went quiet: its row there is gone or names another instance, or a
    /// handle following the instance moved there. The point is given up
    /// ([`State::Gone`]); watch the point at `to` instead. The hub doesn't
    /// follow it.
    Moved {
        /// Where the device answers now.
        to: DeviceAddress,
    },
}

/// A point's latest values and how its watch is going.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Each property's latest raw value, or why the device gave none.
    pub rows: Vec<RawRow>,
    /// Where the values come from; `None` before the first read.
    pub source: Option<Source>,
    /// How the latest values came; `None` before any.
    pub via: Option<Via>,
    /// How the watch is going.
    pub state: State,
    /// Why the point is polled or not live, as text.
    pub note: Option<String>,
    /// The hub's sequence number at the last change; 0 before any.
    pub seq: u64,
    /// When the values last changed.
    pub updated: Option<Instant>,
    /// COV notifications applied.
    pub notifications: u64,
    /// Changes folded into another's signal.
    pub coalesced: u64,
    /// Polls answered.
    pub polls: u64,
    /// Renewals the device took.
    pub renewals: u64,
    /// Why the point is polled, or was given up, as its start, a failed
    /// renewal or a move found; a poll's error, which goes to
    /// [`note`](Self::note), leaves it. `None` while subscribed, and
    /// before the first start ends.
    pub why: Option<Why>,
    /// What is known of COV for this point: the device's own answer to
    /// its subscription once asked, before that what the device declared
    /// or did for its other points.
    pub cov: CovSupport,
}

impl Default for Snapshot {
    /// A point not read yet: starting, no rows, nothing counted.
    fn default() -> Self {
        Self::new()
    }
}

impl Snapshot {
    pub(super) fn new() -> Self {
        Self {
            rows: Vec::new(),
            source: None,
            via: None,
            state: State::Starting,
            note: None,
            seq: 0,
            updated: None,
            notifications: 0,
            coalesced: 0,
            polls: 0,
            renewals: 0,
            why: None,
            cov: CovSupport::Unknown,
        }
    }
}
