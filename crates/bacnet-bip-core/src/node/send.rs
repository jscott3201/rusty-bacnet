//! What a node sends for an NPDU: the message and its destination, within
//! the datagram budget.

use core::fmt;
use core::net::SocketAddrV4;

use bacnet_bip_codec::{Bvll, HEADER_LENGTH, MAX_BVLL_LENGTH};

use bacnet_npdu::MAX_NPDU;

use super::config::{Link, Mode};

/// A message to send. The caller encodes it
/// ([`Bvll::encode_into`](bacnet_bip_codec::Bvll::encode_into)) and sends
/// it from the node's own socket to `to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Outgoing<'n> {
    /// The destination B/IP address.
    pub to: SocketAddrV4,
    /// The message.
    pub message: Bvll<'n>,
}

/// How an NPDU is addressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cast {
    /// To one node.
    Unicast,
    /// To every node on the network.
    Broadcast,
}

/// Why a send was refused. Nothing went out.
///
/// The link refuses an NPDU it can't carry, a foreign device's node a
/// broadcast it can't hold, and the host's socket, through a port's
/// [`Datagrams`](crate::port::Datagrams), a datagram it can't take.
///
/// It is `#[non_exhaustive]`, so a later reason isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SendRefusal {
    /// The NPDU is longer than [`Link::max_npdu`] allows.
    TooLong {
        /// The longest NPDU this cast takes.
        max: usize,
    },
    /// A foreign device holds as many broadcasts as its caller can keep
    /// ([`Foreign::hold`](super::Foreign::hold)) until its first
    /// registration attempt is answered.
    HoldFull,
    /// A unicast to one of the link's group addresses
    /// ([`Link::groups`](super::Link::groups)): the subnet's broadcast
    /// address, the limited broadcast or a multicast address. An
    /// Original-Unicast-NPDU goes to one node's own address; to reach the
    /// group, broadcast ([`Link::broadcast`]).
    GroupDestination,
    /// The host's socket can't take the datagram now: its queue is full.
    SocketFull,
    /// The host's socket couldn't send the datagram: its buffer can't hold
    /// the encoded message, or the send failed.
    SocketError,
}

impl fmt::Display for SendRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong { max } => write!(f, "NPDU longer than the {max} octets B/IP sends"),
            Self::HoldFull => f.write_str("no room to hold a broadcast until registration"),
            Self::GroupDestination => {
                f.write_str("a unicast can't go to a group address; send a broadcast instead")
            }
            Self::SocketFull => f.write_str("the socket can't take the datagram now"),
            Self::SocketError => f.write_str("the socket couldn't send the datagram"),
        }
    }
}

impl core::error::Error for SendRefusal {}

/// Octets a broadcast's NPDU gains, at worst, before it reaches the
/// network's devices: the 4-octet header, and the 6-octet origin a BBMD
/// adds when it forwards it.
const BROADCAST_OVERHEAD: usize = HEADER_LENGTH + 6;

impl Link {
    /// The longest NPDU sent as `cast`.
    ///
    /// With a budget, a unicast's datagram is the NPDU plus the 4-octet
    /// header, and a broadcast's, once a BBMD forwards it as a
    /// Forwarded-NPDU, 6 octets more; either way at most [`MAX_NPDU`]
    /// (Table 6-1). Without one, what the Length field can state.
    #[must_use]
    pub fn max_npdu(&self, cast: Cast) -> usize {
        let overhead = match cast {
            Cast::Unicast => HEADER_LENGTH,
            Cast::Broadcast => BROADCAST_OVERHEAD,
        };
        match self.budget {
            Some(budget) => usize::from(budget).saturating_sub(overhead).min(MAX_NPDU),
            None => MAX_BVLL_LENGTH - overhead,
        }
    }

    /// An Original-Unicast-NPDU carrying `npdu` to `to` (Annex J.3).
    ///
    /// `to` must be one node's address: a group address of this link
    /// ([`Groups::contains`](super::Groups::contains)) is refused with
    /// [`SendRefusal::GroupDestination`]. A receiver that checks the
    /// destination drops an Original-Unicast-NPDU sent to a group, so it
    /// would reach no one, or nodes that should have ignored it (#1677).
    /// Another subnet's directed broadcast address isn't known here, so it
    /// isn't caught; a receiver's destination check still drops it.
    pub fn unicast<'n>(
        &self,
        to: SocketAddrV4,
        npdu: &'n [u8],
    ) -> Result<Outgoing<'n>, SendRefusal> {
        if self.groups().contains(to) {
            return Err(SendRefusal::GroupDestination);
        }
        self.fits(npdu, Cast::Unicast)?;
        Ok(Outgoing {
            to,
            message: Bvll::OriginalUnicastNpdu(npdu),
        })
    }

    /// A local broadcast of `npdu`, by mode: an Original-Broadcast-NPDU to
    /// the subnet's broadcast address for a normal node or a BBMD (Annex
    /// J.4.1), and a Distribute-Broadcast-To-Network to its BBMD for a
    /// foreign device (Annex J.2.10). The flag is set for a BBMD, which
    /// also forwards its own broadcast
    /// ([`Tables::own_broadcast`](crate::bbmd::Tables::own_broadcast)).
    ///
    /// A foreign device asks [`Node::admit_broadcast`](super::Node::admit_broadcast)
    /// first.
    pub fn broadcast<'n>(&self, npdu: &'n [u8]) -> Result<(Outgoing<'n>, bool), SendRefusal> {
        self.fits(npdu, Cast::Broadcast)?;
        let local = Outgoing {
            to: self.subnet.broadcast,
            message: Bvll::OriginalBroadcastNpdu(npdu),
        };
        Ok(match self.mode {
            Mode::Normal => (local, false),
            Mode::Bbmd => (local, true),
            Mode::Foreign(foreign) => (
                Outgoing {
                    to: foreign.bbmd,
                    message: Bvll::DistributeBroadcastToNetwork(npdu),
                },
                false,
            ),
        })
    }

    fn fits(&self, npdu: &[u8], cast: Cast) -> Result<(), SendRefusal> {
        if self.budget.is_none() {
            return Ok(());
        }
        let max = self.max_npdu(cast);
        if npdu.len() > max {
            return Err(SendRefusal::TooLong { max });
        }
        Ok(())
    }
}
