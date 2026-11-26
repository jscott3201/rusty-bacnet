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
    /// The watch's settings poll every point
    /// ([`WatchSettings::subscribe`](super::WatchSettings::subscribe) is
    /// false): no subscription is asked for.
    PollOnly,
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
    /// The device's instance answers at `to` now: the device there named
    /// itself so, answering the hub's read of its Device object by the
    /// wildcard instance (#2812), and it is gone from the point's address:
    /// its device, asked once more, times out or isn't that one (a device
    /// still answering as it there is a second claimant, and the point
    /// stays watched). The ask is skipped only when no route to the address
    /// is known, or when the hub's own look, after its own request there
    /// went unanswered, found `to` while the address is failing. That read
    /// is cued by the hub's look for a device that stopped answering (the
    /// row at its old address names another instance and only `to` claims
    /// this one) or by a handle following the instance moving there;
    /// neither alone gives the point up. The point is given up
    /// ([`State::Gone`](super::State::Gone)); watch the point at `to`
    /// instead. The hub doesn't follow it, but starts the point again if
    /// the old address answers for the instance again.
    ///
    /// A point given up already, for another device at its address
    /// ([`Replaced`](Self::Replaced)), is no longer watched there, so the
    /// old address isn't asked: any device at `to` naming the instance has
    /// it say `Moved`, at most once each turn of the hub's pace. Take `to`
    /// as a hint: a watch made there checks which device answers as any
    /// watch does.
    Moved {
        /// Where the device answers now.
        to: DeviceAddress,
    },
    /// Another device answers at the point's address now: asked (after an
    /// announcement there, the discovery table, or a notification naming
    /// another sender said so), or in a poll's own answer, the device there
    /// names itself `found`, not `watched`, the instance an earlier answer
    /// named for the point. Nothing from the address is taken as
    /// the point's from then on, its values are dropped, and nothing more
    /// is sent for it: its old subscription, which the new device never
    /// held, lapses with its lifetime. The point is given up
    /// ([`State::Gone`](super::State::Gone)), becomes
    /// [`Moved`](Self::Moved) once `watched` announces itself at one other
    /// address and names itself there when read, and starts again when its
    /// address answers for `watched` again.
    Replaced {
        /// The instance an earlier answer named for the point.
        watched: u32,
        /// The instance the device there answers for now.
        found: u32,
    },
    /// The device's COV notifications name device `sender` as their
    /// sender, and a read at its address couldn't tell that name from
    /// another device there (one answers for it, or the device couldn't
    /// say), so they aren't taken (#2603): the point is polled instead, its
    /// subscription cancelled. Nothing is remembered of the device's COV
    /// service: the point is subscribed and the name read again when it
    /// starts again, as is another watch of the device.
    ForeignSender {
        /// The instance the notifications name.
        sender: u32,
    },
}
