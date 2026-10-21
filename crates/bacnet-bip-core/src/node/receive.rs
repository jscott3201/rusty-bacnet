//! What a node does with one datagram: the receive rules, in order, and the
//! dispatch by BVLC function and mode.

use core::net::SocketAddrV4;

use bacnet_bip_codec::{BvlcFunction, BvlcResultCode, Bvll, BvllError, BvllFrame};
use bacnet_time::Instant;

use super::addressing::Addressed;
use super::config::{ForwardedTrust, Link, Mode};
use crate::bbmd::Request;
use crate::Arrival;

/// One received datagram, as the host's socket reported it.
///
/// It is `#[non_exhaustive]`, so a new field isn't a breaking change:
/// outside this crate, build it with [`Datagram::new`] and set the fields
/// that differ from its defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Datagram<'d> {
    /// The UDP payload.
    pub payload: &'d [u8],
    /// The UDP source: the sender's B/IP address, at any port (Annex J.1.2).
    pub source: SocketAddrV4,
    /// What the destination address was, for this node, or `None` when the
    /// host can't tell: its IP stack reports no destination per datagram
    /// (#1678). Such a datagram is discarded
    /// ([`Discard::UnknownDestination`]): whether it came by unicast or
    /// broadcast decides what a node may do with it, so the node never
    /// guesses.
    pub to: Option<Addressed>,
    /// Which of the host's sockets read it.
    pub path: Path,
    /// Whether the OS or the receive buffer cut the datagram short.
    pub truncated: bool,
}

impl<'d> Datagram<'d> {
    /// `payload` from `source`, sent to what `to` says, read whole by the
    /// [`Primary`](Path::Primary) socket. A host with a broadcast listener
    /// sets [`path`](Self::path), and one whose read was cut short sets
    /// [`truncated`](Self::truncated).
    #[must_use]
    pub const fn new(payload: &'d [u8], source: SocketAddrV4, to: Option<Addressed>) -> Self {
        Self {
            payload,
            source,
            to,
            path: Path::Primary,
            truncated: false,
        }
    }
}

/// Which socket read a datagram. The host builds it, and the node matches
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Path {
    /// The socket every send leaves from.
    Primary,
    /// A receive-only broadcast listener beside a socket bound to the
    /// interface address, which keeps only this link's broadcasts.
    Listener {
        /// The host saw it arrive on another interface than the node's.
        other_interface: bool,
    },
}

/// The host's own admission rules, which [`Link::receive`] consults.
/// [`Permissive`] admits everything.
pub trait Policy {
    /// Whether a Read-Broadcast-Distribution-Table, Read-Foreign-Device-Table,
    /// Register-Foreign-Device or Delete-Foreign-Device-Table-Entry of
    /// `function` from `source` goes on, before its payload is read: a rate
    /// limit. One refused is dropped unanswered.
    fn admit_management(
        &mut self,
        source: SocketAddrV4,
        function: BvlcFunction,
        now: Instant,
    ) -> bool {
        let _ = (source, function, now);
        true
    }

    /// Whether a table ACK the codec decoded is acceptable, such as within
    /// the host's size limit. One refused counts as malformed, and still
    /// completes the exchange it matches, with the ACK as received.
    fn accept_table(&mut self, ack: &Bvll<'_>) -> bool {
        let _ = ack;
        true
    }
}

/// A [`Policy`] that admits everything: for firmware and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Permissive;

impl Policy for Permissive {}

/// What to do with a received datagram. Each borrows the datagram, not the
/// link, so a host can act on it after releasing any lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Received<'d> {
    /// Hand an NPDU up to the network layer.
    Deliver(Delivery<'d>),
    /// Send a BVLC-Result with `code` to `to`: the NAK for a BBMD-only
    /// request this node doesn't serve (Annex J.2.1).
    Nak {
        /// Where the NAK goes: the request's source.
        to: SocketAddrV4,
        /// The request's own NAK.
        code: BvlcResultCode,
        /// The request's payload was malformed; count it so.
        malformed: bool,
    },
    /// A BVLC-Result or table ACK, for [`Node::resolve`](super::Node::resolve)
    /// to match against this node's own requests.
    Response(Response<'d>),
    /// The BBMD's share of a message: [`Mode::Bbmd`] only.
    Bbmd(ForBbmd<'d>),
    /// Dropped, for this reason.
    Discarded(Discard),
}

/// An NPDU to hand up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Delivery<'d> {
    /// The NPDU.
    pub npdu: &'d [u8],
    /// Its source B/IP address: the sender, or a Forwarded-NPDU's origin
    /// (Annex J.2.5).
    pub source: SocketAddrV4,
    /// Whether it reached more than this node (a broadcast), so the network
    /// layer never answers it as a directed message.
    pub group: bool,
}

/// A response to one of this node's management requests, before it is
/// matched. [`Link::receive`] builds it, and
/// [`Node::resolve`](super::Node::resolve) takes it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Response<'d> {
    /// Who sent it.
    pub peer: SocketAddrV4,
    /// Its function: BVLC-Result, Read-BDT-Ack or Read-FDT-Ack.
    pub function: BvlcFunction,
    /// The message, or why the codec refused its payload.
    pub message: Result<Bvll<'d>, BvllError>,
    /// Whether the codec decoded it and [`Policy::accept_table`] took it.
    pub accepted: bool,
}

/// Why a datagram was dropped: each is counted once, under the first rule
/// it fails, in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Discard {
    /// The OS or the buffer cut it short.
    Truncated,
    /// Its UDP source is a group address ([`Groups::contains_ip`](super::Groups::contains_ip)).
    GroupSource,
    /// The host couldn't say where it was sent ([`Datagram::to`] is
    /// `None`), so nothing tells a unicast from a broadcast (#1678).
    UnknownDestination,
    /// A broadcast listener read it but doesn't keep it: not sent to this
    /// link's broadcast address, or it arrived on another interface.
    ListenerMismatch {
        /// It arrived on another interface.
        other_interface: bool,
    },
    /// Shorter than the 4-octet BVLL header.
    TooShort,
    /// The BVLC type isn't B/IP's.
    NotBip,
    /// The Length field differs from the datagram's length.
    LengthMismatch,
    /// Its function doesn't fit the address it was sent to.
    DestinationMismatch,
    /// A management request the [`Policy`] refused.
    RateLimited,
    /// A function Annex J doesn't define.
    UnknownFunction,
    /// The payload doesn't fit the function, and nothing answers it.
    MalformedPayload,
    /// It comes from this node: from its own address, or a Forwarded-NPDU
    /// naming it as the origin.
    OwnEcho,
    /// A Forwarded-NPDU whose origin is a group address.
    GroupOrigin,
    /// A request whose NAK couldn't reach anyone: from the unspecified
    /// address, port 0, a group address or this node.
    UnanswerableSource,
    /// A Forwarded-NPDU a foreign device doesn't take from this sender
    /// ([`ForwardedTrust::ConfiguredBbmd`]).
    ForwardedUntrusted,
}

impl Discard {
    /// Why a datagram failed to decode.
    pub(crate) const fn of_decode_error(error: &BvllError) -> Self {
        match error {
            BvllError::TooShort { .. } => Self::TooShort,
            BvllError::NotBip { .. } => Self::NotBip,
            BvllError::LengthMismatch { .. } => Self::LengthMismatch,
            BvllError::UnknownFunction { .. } => Self::UnknownFunction,
            _ => Self::MalformedPayload,
        }
    }
}

/// The share of a message a BBMD acts on through
/// [`bbmd::Tables`](crate::bbmd::Tables), once every node rule has passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForBbmd<'d> {
    /// An Original-Broadcast-NPDU: deliver it, then forward it
    /// ([`Tables::original_broadcast`](crate::bbmd::Tables::original_broadcast)).
    OriginalBroadcast(Delivery<'d>),
    /// A Forwarded-NPDU: [`Tables::forwarded_npdu`](crate::bbmd::Tables::forwarded_npdu)
    /// decides whether it is delivered and where it goes.
    Forwarded {
        /// The BBMD that sent it.
        sender: SocketAddrV4,
        /// How it arrived.
        arrival: Arrival,
        /// Its originating address, the NPDU's source.
        origin: SocketAddrV4,
        /// The NPDU.
        npdu: &'d [u8],
    },
    /// A Distribute-Broadcast-To-Network:
    /// [`Tables::distribute_broadcast`](crate::bbmd::Tables::distribute_broadcast),
    /// and its NAK on refusal.
    Distribute {
        /// The foreign device that sent it.
        sender: SocketAddrV4,
        /// The NPDU.
        npdu: &'d [u8],
    },
    /// A request for the BBMD's tables (Register-Foreign-Device, the two
    /// table reads, Delete-Foreign-Device-Table-Entry), which
    /// [`Tables::serve`](crate::bbmd::Tables::serve) answers.
    Management(Request),
}

impl<'d> ForBbmd<'d> {
    /// The BBMD's share of a well-formed BBMD-only request from `sender`;
    /// `None` for Write-Broadcast-Distribution-Table, which every device
    /// refuses (Annex J.4.4.2), and any other message.
    fn of_request(message: Bvll<'d>, sender: SocketAddrV4) -> Option<Self> {
        let request = match message {
            Bvll::DistributeBroadcastToNetwork(npdu) => {
                return Some(Self::Distribute { sender, npdu })
            }
            Bvll::RegisterForeignDevice { ttl } => Request::Register { sender, ttl },
            Bvll::ReadBroadcastDistributionTable => Request::ReadBdt { sender },
            Bvll::ReadForeignDeviceTable => Request::ReadFdt { sender },
            Bvll::DeleteForeignDeviceTableEntry(entry) => Request::DeleteFdtEntry { sender, entry },
            _ => return None,
        };
        Some(Self::Management(request))
    }
}

/// The management requests [`Policy::admit_management`] is asked about.
fn rate_limited(function: BvlcFunction) -> bool {
    matches!(
        function,
        BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE
            | BvlcFunction::READ_FOREIGN_DEVICE_TABLE
            | BvlcFunction::REGISTER_FOREIGN_DEVICE
            | BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY
    )
}

impl Link {
    /// Decide what to do with one datagram.
    ///
    /// The rules run in this order, and a dropped datagram is
    /// [`Discarded`](Received::Discarded) under the first it fails:
    ///
    /// 1. truncated by the OS or the buffer;
    /// 2. a UDP source that is a group address;
    /// 3. a destination the host couldn't report ([`Datagram::to`] is
    ///    `None`);
    /// 4. on a broadcast listener: not sent to the broadcast address, or
    ///    arrived on another interface;
    /// 5. the BVLL header: length, type, Length field (Annex J.2);
    /// 6. the function against the destination ([`Addressed`]); for a
    ///    foreign device, a Forwarded-NPDU sent to its own address is
    ///    unicast, even where that address is its broadcast one too;
    /// 7. on a broadcast listener, a function that arrived as unicast;
    /// 8. [`Policy::admit_management`], for the four requests it covers;
    /// 9. the payload against the function's layout.
    ///
    /// Then each function is dispatched by mode. Every one of the six
    /// BBMD-only requests that arrived by unicast gets its own NAK
    /// ([`BvlcFunction::nak`]), unless [`Mode::Bbmd`] claims it, from any
    /// source a reply can reach ([`Link::is_answerable`]). Write-BDT is
    /// refused in every mode, and so is a malformed BBMD-only request.
    pub fn receive<'d>(
        &self,
        datagram: &Datagram<'d>,
        now: Instant,
        policy: &mut impl Policy,
    ) -> Received<'d> {
        let source = datagram.source;
        if datagram.truncated {
            return Received::Discarded(Discard::Truncated);
        }
        if self.groups().contains_ip(*source.ip()) {
            return Received::Discarded(Discard::GroupSource);
        }
        let Some(to) = datagram.to else {
            return Received::Discarded(Discard::UnknownDestination);
        };
        let listener = matches!(datagram.path, Path::Listener { .. });
        if let Path::Listener { other_interface } = datagram.path {
            if !to.broadcast {
                return listener_mismatch(false);
            }
            if other_interface {
                return listener_mismatch(true);
            }
        }
        let frame = match BvllFrame::decode(datagram.payload) {
            Ok(frame) => frame,
            Err(error) => return Received::Discarded(Discard::of_decode_error(&error)),
        };
        let Some(mut arrival) = to.arrival(frame.function) else {
            return Received::Discarded(Discard::DestinationMismatch);
        };
        // A foreign device's BBMD sends it Forwarded-NPDUs by unicast. Where
        // its broadcast address is also its own, as in a loopback test, each
        // of them is flagged as both, and counts as the unicast it was
        // (#1851). The trust rule still asks that the BBMD sent it.
        if to.node
            && frame.function == BvlcFunction::FORWARDED_NPDU
            && matches!(self.mode, Mode::Foreign(_))
        {
            arrival = Arrival::Unicast;
        }
        // Where the broadcast address is also the node's own, as in a
        // loopback test, a unicast function passes both checks above.
        if listener && arrival != Arrival::Broadcast {
            return listener_mismatch(false);
        }
        if rate_limited(frame.function) && !policy.admit_management(source, frame.function, now) {
            return Received::Discarded(Discard::RateLimited);
        }
        match frame.message() {
            Ok(message) => self.dispatch(message, source, arrival, policy),
            Err(error) => self.malformed(frame.function, error, source),
        }
    }

    fn dispatch<'d>(
        &self,
        message: Bvll<'d>,
        source: SocketAddrV4,
        arrival: Arrival,
        policy: &mut impl Policy,
    ) -> Received<'d> {
        match message {
            Bvll::OriginalUnicastNpdu(npdu) => {
                if self.is_own(source) {
                    return Received::Discarded(Discard::OwnEcho);
                }
                // From the address, not the function: Original-Unicast is
                // only ever admitted as unicast.
                let group = arrival == Arrival::Broadcast;
                Received::Deliver(Delivery {
                    npdu,
                    source,
                    group,
                })
            }
            Bvll::OriginalBroadcastNpdu(npdu) => {
                if self.is_own(source) {
                    return Received::Discarded(Discard::OwnEcho);
                }
                let delivery = Delivery {
                    npdu,
                    source,
                    group: true,
                };
                match self.mode {
                    Mode::Bbmd => Received::Bbmd(ForBbmd::OriginalBroadcast(delivery)),
                    Mode::Normal | Mode::Foreign(_) => Received::Deliver(delivery),
                }
            }
            Bvll::ForwardedNpdu { origin, npdu } => self.forwarded(source, arrival, origin, npdu),
            Bvll::BvlcResult(_) => Received::Response(Response {
                peer: source,
                function: message.function(),
                message: Ok(message),
                accepted: true,
            }),
            Bvll::ReadBroadcastDistributionTableAck(_) | Bvll::ReadForeignDeviceTableAck(_) => {
                let accepted = policy.accept_table(&message);
                Received::Response(Response {
                    peer: source,
                    function: message.function(),
                    message: Ok(message),
                    accepted,
                })
            }
            _ => self.request(message, source),
        }
    }

    /// A Forwarded-NPDU: its origin is its NPDU's source (Annex J.2.5).
    fn forwarded<'d>(
        &self,
        sender: SocketAddrV4,
        arrival: Arrival,
        origin: SocketAddrV4,
        npdu: &'d [u8],
    ) -> Received<'d> {
        if self.is_own(origin) {
            return Received::Discarded(Discard::OwnEcho);
        }
        if self.groups().contains(origin) {
            return Received::Discarded(Discard::GroupOrigin);
        }
        match self.mode {
            Mode::Bbmd => {
                // A local rebroadcast keeps its remote origin; this BBMD's
                // own row in its BDT must not pass its echo as a peer's.
                if self.is_own(sender) {
                    return Received::Discarded(Discard::OwnEcho);
                }
                Received::Bbmd(ForBbmd::Forwarded {
                    sender,
                    arrival,
                    origin,
                    npdu,
                })
            }
            Mode::Foreign(foreign)
                if foreign.trust == ForwardedTrust::ConfiguredBbmd
                    && (sender != foreign.bbmd || arrival != Arrival::Unicast) =>
            {
                Received::Discarded(Discard::ForwardedUntrusted)
            }
            Mode::Normal | Mode::Foreign(_) => Received::Deliver(Delivery {
                npdu,
                source: origin,
                group: true,
            }),
        }
    }

    /// One of the six BBMD-only requests, well formed.
    fn request<'d>(&self, message: Bvll<'d>, source: SocketAddrV4) -> Received<'d> {
        let function = message.function();
        let Some(code) = function.nak() else {
            return Received::Discarded(Discard::UnknownFunction);
        };
        if function == BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK && self.is_own(source) {
            return Received::Discarded(Discard::OwnEcho);
        }
        if self.mode == Mode::Bbmd {
            if let Some(share) = ForBbmd::of_request(message, source) {
                return Received::Bbmd(share);
            }
        }
        if !self.is_answerable(source) {
            return Received::Discarded(Discard::UnanswerableSource);
        }
        Received::Nak {
            to: source,
            code,
            malformed: false,
        }
    }

    /// A frame whose payload the codec refused, or whose function it doesn't
    /// know. A BBMD-only request is still refused with its NAK in every
    /// mode, since nothing else would answer the requester; a malformed
    /// response goes on to be matched and counted.
    fn malformed<'d>(
        &self,
        function: BvlcFunction,
        error: BvllError,
        source: SocketAddrV4,
    ) -> Received<'d> {
        if let Some(code) = function.nak() {
            if self.is_answerable(source) {
                return Received::Nak {
                    to: source,
                    code,
                    malformed: true,
                };
            }
            return Received::Discarded(Discard::MalformedPayload);
        }
        match function {
            BvlcFunction::BVLC_RESULT
            | BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK
            | BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK => Received::Response(Response {
                peer: source,
                function,
                message: Err(error),
                accepted: false,
            }),
            _ => Received::Discarded(Discard::of_decode_error(&error)),
        }
    }
}

fn listener_mismatch<'d>(other_interface: bool) -> Received<'d> {
    Received::Discarded(Discard::ListenerMismatch { other_interface })
}
