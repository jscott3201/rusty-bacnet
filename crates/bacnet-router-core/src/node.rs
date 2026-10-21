//! The receive side of the network layer for a node that doesn't route
//! (Clause 6.5.2): which arriving NPDUs reach its application or its
//! network-message consumer, and which are dropped.
//!
//! The checks every frame passes first, at a [`Router`](crate::Router) as
//! at a [`Node`], are written once, here (`admit`), and the router's
//! [`receive`](crate::Router::receive) runs them too, so the two accept and
//! drop the same frames for the same reasons, in the same order:
//!
//! 1. A link source longer than [`MAX_MAC_LEN`](crate::MAX_MAC_LEN) is
//!    dropped (#1198).
//! 2. The NPDU is decoded: a DLEN or SLEN past the same bound is dropped
//!    (#1141), as is anything else [`Npdu::decode`] refuses. Only a router
//!    answers the first with a reject (Clause 6.6.3.5).
//! 3. A DADR beside DNET 0xFFFF is dropped (#1379), and so is a broadcast,
//!    global or remote, whose APDU isn't an Unconfirmed-Request (Clause
//!    6.3, #1491).
//!
//! A node then goes on where a router would route: an NPDU whose DNET names
//! one network is for some other device, and a node drops it, APDU or
//! network message alike (Clauses 6.5.2.1, 6.5.2.2 and 6.5.4). What is left
//! has no DNET, or is a global broadcast: a network message goes to the
//! node's consumer for them, and an APDU to its application.
//!
//! A [`Node`] keeps nothing but its counts, which it drains as the router
//! and the link cores do ([`Node::take_counters`]).

use crate::npdu::{Address, DecodeError, Npdu, GLOBAL_NETWORK};
use crate::router::{bump, Discard};
use crate::Mac;

/// A frame that passed [`admit`]: its link source, stored, and its NPDU.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Admitted<'f> {
    /// The link source.
    pub(crate) source: Mac,
    /// The NPDU.
    pub(crate) npdu: Npdu<'f>,
}

/// Which count a refusal of [`admit`] goes in: the router's
/// [`Counters`](crate::router::Counters) and a node's [`Counters`] both
/// have each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Count {
    /// `address_length_drops`.
    AddressLength,
    /// `global_broadcast_dadr_drops`.
    GlobalDadr,
    /// `broadcast_pdu_type_drops`.
    BroadcastType,
    /// `malformed`.
    Malformed,
}

/// A frame [`admit`] refused.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Refused<'f> {
    /// Why.
    pub(crate) discard: Discard,
    /// Where it is counted.
    pub(crate) count: Count,
    /// For an over-long DLEN or SLEN: the link source, the DNET, if the
    /// frame names one, and the SNET and SADR behind the DADR, if
    /// readable. A router's reject goes there; a node sends nothing.
    pub(crate) reject_to: Option<(Mac, Option<u16>, Option<Address<'f>>)>,
}

impl Refused<'_> {
    /// A refusal for `discard`, counted in `count`, that no reject answers.
    const fn only(discard: Discard, count: Count) -> Self {
        Self {
            discard,
            count,
            reject_to: None,
        }
    }
}

/// The checks every frame passes before anything acts on it, at a router
/// and at a node alike: the link source's length, the NPDU's decode, a
/// DADR beside DNET 0xFFFF, and what a broadcast may carry. See the
/// [module](self) for the order.
pub(crate) fn admit<'f>(source: &[u8], npdu: &'f [u8]) -> Result<Admitted<'f>, Refused<'f>> {
    let Ok(link_source) = Mac::from_slice(source) else {
        let length = source.len();
        return Err(Refused::only(
            Discard::SourceTooLong { length },
            Count::AddressLength,
        ));
    };
    let npdu = match Npdu::decode(npdu) {
        Ok(npdu) => npdu,
        Err(DecodeError::AddressTooLong {
            field,
            length,
            dnet,
            source,
        }) => {
            return Err(Refused {
                discard: Discard::AddressTooLong { field, length },
                count: Count::AddressLength,
                reject_to: Some((link_source, dnet, source)),
            })
        }
        Err(DecodeError::Malformed(fault)) => {
            return Err(Refused::only(Discard::Malformed(fault), Count::Malformed));
        }
    };
    if let Some(destination) = npdu.destination {
        let global = destination.network == GLOBAL_NETWORK;
        if global && !destination.mac.is_empty() {
            return Err(Refused::only(Discard::GlobalWithDadr, Count::GlobalDadr));
        }
        let broadcast = global || destination.mac.is_empty();
        if broadcast && !npdu.is_network_message() && !is_unconfirmed(npdu.payload) {
            return Err(Refused::only(
                Discard::BroadcastNotUnconfirmed,
                Count::BroadcastType,
            ));
        }
    }
    Ok(Admitted {
        source: link_source,
        npdu,
    })
}

/// Whether an APDU is an Unconfirmed-Request: the high nibble of its first
/// octet (Clause 20.1). An empty one names no type.
pub(crate) fn is_unconfirmed(apdu: &[u8]) -> bool {
    apdu.first().is_some_and(|&first| first >> 4 == 1)
}

/// What a node does with a frame.
///
/// Like the router's [`Received`](crate::router::Received), it stays
/// exhaustive: a host acts on each outcome, so a new one should not compile
/// unhandled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "a frame may carry an APDU for the application"]
pub enum Received<'f> {
    /// An APDU for the node's application.
    Apdu(Apdu<'f>),
    /// A network message for the node's consumer of them, if it has one.
    /// It has no DNET, or is a global broadcast.
    Message(Npdu<'f>),
    /// Discarded, and counted, for this reason.
    Discarded(Discard),
}

/// An APDU for the node's application, with how it was addressed.
///
/// It is `#[non_exhaustive]`, so a new field isn't a breaking change; only
/// a [`Node`] makes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Apdu<'f> {
    /// The NPDU that carried it: its APDU is the payload, at its offset in
    /// the frame, and its SNET and SADR, when a router relayed it, are the
    /// source.
    pub npdu: Npdu<'f>,
    /// Whether it was sent to a group: a global broadcast, or a link
    /// broadcast or multicast with no DNET.
    pub group: bool,
    /// Whether it was a global broadcast.
    pub global: bool,
}

/// What a node counts: each frame it discarded, by reason, since its
/// counters were last taken ([`Node::take_counters`]). Each is a `u32`
/// that saturates between takes, for the caller to add to totals as wide
/// as it needs, as the [router's](crate::router::Counters) are.
///
/// It is `#[non_exhaustive]`, so a new count isn't a breaking change.
/// Outside this crate, build one from `Counters::default()` and set its
/// fields.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Counters {
    /// Frames refused because the link source, the DLEN or the SLEN was
    /// past [`MAX_MAC_LEN`](crate::MAX_MAC_LEN) (#1141, #1198).
    pub address_length_drops: u32,
    /// Frames dropped for a DADR beside DNET 0xFFFF (#1379).
    pub global_broadcast_dadr_drops: u32,
    /// Broadcasts dropped for carrying an APDU other than an
    /// Unconfirmed-Request (#1491).
    pub broadcast_pdu_type_drops: u32,
    /// Frames that didn't parse as an NPDU for any other reason.
    pub malformed: u32,
    /// NPDUs whose DNET named one network, for some other device: a node
    /// doesn't route them ([`Discard::NotRouted`]).
    pub routed_drops: u32,
}

impl Counters {
    /// The count a refusal of the shared admission checks goes in.
    pub(crate) fn slot(&mut self, count: Count) -> &mut u32 {
        match count {
            Count::AddressLength => &mut self.address_length_drops,
            Count::GlobalDadr => &mut self.global_broadcast_dadr_drops,
            Count::BroadcastType => &mut self.broadcast_pdu_type_drops,
            Count::Malformed => &mut self.malformed,
        }
    }
}

/// The network layer's receive side at a node that doesn't route. See the
/// [module](self).
///
/// ```
/// use bacnet_router_core::node::{Node, Received};
/// use bacnet_router_core::npdu::{Address, Npdu};
/// use bacnet_router_core::router::Discard;
///
/// let mut node = Node::new();
/// // A Who-Is, globally broadcast: for the application.
/// let mut frame = [0u8; 16];
/// let mut who_is = Npdu::apdu(&[0x10, 0x08]);
/// who_is.destination = Some(Address { network: 0xFFFF, mac: &[] });
/// let len = who_is.encode_into(&mut frame)?;
/// let Received::Apdu(apdu) = node.receive(&[7], false, &frame[..len]) else {
///     panic!("an APDU");
/// };
/// assert_eq!((apdu.npdu.payload, apdu.group, apdu.global), (&[0x10, 0x08][..], true, true));
///
/// // The same request for station 5 on network 2: a router's to pass on.
/// who_is.destination = Some(Address { network: 2, mac: &[5] });
/// let len = who_is.encode_into(&mut frame)?;
/// assert_eq!(node.receive(&[7], false, &frame[..len]), Received::Discarded(Discard::NotRouted));
/// assert_eq!(node.take_counters().routed_drops, 1);
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Debug, Default, Clone)]
pub struct Node {
    counters: Counters,
}

impl Node {
    /// A node with nothing counted.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            counters: Counters {
                address_length_drops: 0,
                global_broadcast_dadr_drops: 0,
                broadcast_pdu_type_drops: 0,
                malformed: 0,
                routed_drops: 0,
            },
        }
    }

    /// What the frame `npdu`, from link source `source`, means for this
    /// node. `link_group` says whether it arrived as a link-layer broadcast
    /// or multicast.
    pub fn receive<'f>(&mut self, source: &[u8], link_group: bool, npdu: &'f [u8]) -> Received<'f> {
        let npdu = match admit(source, npdu) {
            Ok(admitted) => admitted.npdu,
            Err(refused) => {
                bump(self.counters.slot(refused.count));
                return Received::Discarded(refused.discard);
            }
        };
        // Only a global broadcast is left with a DNET a node may act on.
        let global = match npdu.destination {
            None => false,
            Some(destination) if destination.network == GLOBAL_NETWORK => true,
            Some(_) => {
                bump(&mut self.counters.routed_drops);
                return Received::Discarded(Discard::NotRouted);
            }
        };
        if npdu.is_network_message() {
            return Received::Message(npdu);
        }
        Received::Apdu(Apdu {
            npdu,
            group: global || link_group,
            global,
        })
    }

    /// The counts since they were last taken, without taking them.
    #[must_use]
    pub const fn counters(&self) -> Counters {
        self.counters
    }

    /// The counts since they were last taken, starting them again from
    /// zero.
    pub fn take_counters(&mut self) -> Counters {
        core::mem::take(&mut self.counters)
    }
}
