//! What a watch names and reports: a [`Point`] by what its device is
//! bound to ([`Binding`]), [`Why`] a point is polled, unreachable or given
//! up, and where a followed device [`Moved`].

use std::fmt;
use std::time::SystemTime;

use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;

use crate::device::DeviceAddress;
use crate::label::kebab;

/// What a watched point's device is bound to (#2795): an address, or a
/// device instance the hub follows from address to address.
///
/// [`Device::point`](crate::Device::point) picks by the handle: a pinned
/// handle's points are [`At`](Self::At) its address, a following handle's
/// [`Follows`](Self::Follows) its instance. Either way the point is its own
/// identity: an [`Update`](super::Update) carries the point as watched, and
/// says where its device answers now ([`Update::at`](super::Update::at)).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Binding {
    /// The device at this address, whichever it is: given up as
    /// [`Why::Moved`] when its device answers elsewhere, or
    /// [`Why::Replaced`] when another answers here.
    At(DeviceAddress),
    /// The device of this instance, wherever it answers: on `network` when
    /// named with one (0, or the client's own network's number, is the
    /// local network), on any otherwise. The hub watches it at the one
    /// address claiming it when the watch starts, and moves its watch to
    /// another address once its own reads there and here say the device
    /// went there ([`Update::moved`](super::Update::moved)).
    Follows {
        /// The device instance.
        instance: u32,
        /// The network it is looked for on; any for `None`.
        network: Option<u16>,
    },
}

impl Binding {
    /// The address a pinned point is at; `None` for a following one, whose
    /// address is state ([`Update::at`](super::Update::at)).
    pub fn address(&self) -> Option<&DeviceAddress> {
        match self {
            Self::At(address) => Some(address),
            Self::Follows { .. } => None,
        }
    }

    /// The instance a following point follows; `None` for a pinned one.
    pub fn instance(&self) -> Option<u32> {
        match self {
            Self::At(_) => None,
            Self::Follows { instance, .. } => Some(*instance),
        }
    }
}

impl From<DeviceAddress> for Binding {
    fn from(address: DeviceAddress) -> Self {
        Self::At(address)
    }
}

impl fmt::Display for Binding {
    /// In the target grammar: the address, or the instance (`1234`, or
    /// `2:1234` on network 2).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::At(address) => write!(f, "{address}"),
            Self::Follows {
                instance,
                network: Some(network),
            } => write!(f, "{network}:{instance}"),
            Self::Follows { instance, .. } => write!(f, "{instance}"),
        }
    }
}

/// A followed device's move, as the hub took it (#2795): its reads named
/// the device at `to`, and its device was gone from `from`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    /// Where the device answered before.
    pub from: DeviceAddress,
    /// Where it answers now.
    pub to: DeviceAddress,
    /// When the hub took the move, by the wall clock.
    pub when: SystemTime,
    /// How many moves the hub has taken for the point, this one included:
    /// a consumer that fell behind compares it to tell a new move, whatever
    /// the wall clock did meanwhile.
    pub count: u64,
}

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

/// A watched point: an object, whole or one property, of the device its
/// [`Binding`] names: at an address, or followed by its instance. The hub
/// keys its entries on it, so two devices sharing an instance, each named
/// by its address, are two points, and a followed device's point stays the
/// same point when the device moves (#2795).
///
/// [`Device::point`](crate::Device::point) makes one from the grammar
/// (`"ai:1"`, `"ao:2 priority-array[8]"`). [`Display`](fmt::Display) writes
/// it back in that grammar after the target: `192.168.1.10:47808
/// analog-input:1`, `2:0x05 analog-output:2 priority-array[8]`, `1234
/// analog-input:1`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Point {
    /// What the device is bound to: an address, or an instance followed.
    pub device: Binding,
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
    /// A pinned point's device answers at `to` now: the device there named
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
    /// instead, or watch it following its instance
    /// ([`Binding::Follows`]). The hub starts the point again if the old
    /// address answers for the instance again.
    ///
    /// A following point is never given up for a move: on the same
    /// answers the hub moves its watch to `to` and says so in
    /// [`Update::moved`](super::Update::moved) (#2795).
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
    /// held, lapses with its lifetime. A pinned point is given up
    /// ([`State::Gone`](super::State::Gone)), becomes
    /// [`Moved`](Self::Moved) once `watched` announces itself at one other
    /// address and names itself there when read, and starts again when its
    /// address answers for `watched` again. A following point is
    /// [`State::Unreachable`](super::State::Unreachable) for this reason
    /// while the hub looks for its device, and its watch moves where the
    /// hub's reads find it (#2795).
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
    /// A following point's device can't be confirmed at the address the
    /// point was placed at (#2795): the device there refuses the read that
    /// names it, a read of the instance's own Device object names nothing
    /// on its own (a gateway answers for any instance), and no address the
    /// device left was asked. Nothing it answers is taken and nothing is subscribed there: the
    /// point is [`State::Unreachable`](super::State::Unreachable). Watch
    /// such a device by its address ([`Binding::At`]).
    Unconfirmed,
}
