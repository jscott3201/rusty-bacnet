//! Every I-Am this node receives, one record per arrival, from its
//! transport's frame tap (#2612).
//!
//! The low-level client's discovery table keeps one row per address and
//! reports changes to it. [`IAmArrivals`] yields an [`IAmArrival`] for each
//! I-Am frame the link hands this node instead: in the order they came,
//! repeats and two devices claiming one instance included, with the
//! capture times and who put the frame on this node's link. A scanner can
//! time answers to its Who-Is, see an instance claimed at two addresses,
//! and tell a BBMD or router that passed an I-Am on from the device that
//! sent it.
//!
//! It works wherever there is a [`FrameTap`](bacnet_transport::tap::FrameTap):
//! a standalone client's transport (`BACnetClient::transport()`), a client
//! seated on an endpoint, or an endpoint with no client role at all. The
//! frames are read with this crate's frame decoder ([`crate::decode`]), the
//! same one `decode_frame` and the capture view use.
//!
//! # Who sent it
//!
//! | Field | B/IP | B/IPv6 | BACnet/SC | MS/TP |
//! |---|---|---|---|---|
//! | [`link_sender`](IAmArrival::link_sender) | The datagram's UDP source: for a Forwarded-NPDU, the BBMD that forwarded it | The datagram's UDP source | The hub connection | The frame's source station |
//! | [`link_source`](IAmArrival::link_source) | The origin a Forwarded-NPDU carries (Annex J.2.5), else the UDP source | The source VMAC: the original source's in a Forwarded-NPDU (Annex U.2.9) | The originating VMAC the hub fills in (Annex AB.5.3.2, AB.5.3.3) | The source station (Clause 9.3) |
//! | [`forwarded`](IAmArrival::forwarded) | A Forwarded-NPDU whose origin is not its sender | The same | Never: the hub is the link | Never |
//!
//! [`link_source`](IAmArrival::link_source) is what the stack takes as the
//! frame's source MAC. When a router relayed the I-Am from another network,
//! the NPDU names the device by SNET and SADR (Clause 6.2.2), in
//! [`source_network`](IAmArrival::source_network), and `link_source` is the
//! router: the next hop back (Clause 6.5.4).
//!
//! # What is a record
//!
//! A record is an inbound Unconfirmed-Request whose service is I-Am and
//! whose parameters decode. I-Am has no confirmed form and an
//! Unconfirmed-Request has no segmentation fields (Clauses 20.1.3, 21), so
//! no I-Am comes segmented or confirmed.
//!
//! - A frame this node sends is no record. Nor is this node's own I-Am
//!   come back to it on B/IP or B/IPv6, an echo the stack drops by its
//!   origin, once [`IAmArrivals::with_own_address`] names that address. A
//!   BACnet/SC hub never returns a broadcast to its sender (Annex AB.5.3.3)
//!   and the MS/TP port takes its own echo out before the tap.
//! - An I-Am whose parameters don't decode, or one on BACnet/SC whose
//!   message names no originating VMAC, is counted in
//!   [`IAmArrivals::malformed`] and is no record. Odd but decodable values
//!   are kept as sent: any object type, a max APDU below 50 octets, vendor
//!   0 or 65535, a segmentation value outside the enumeration.
//! - A frame whose link, network or application header fails to decode is
//!   not known to hold an I-Am, so it is neither: the tap's frames and the
//!   frame decoder show it.
//! - MS/TP is a shared bus and the station hears all of it, so an I-Am one
//!   station sends to another (an answer to a Who-Is may be unicast, Clause
//!   16.10.4) is a record too, with [`broadcast`](IAmArrival::broadcast)
//!   false.
//! - Ethernet frames are not read yet: the frame decoder has no Clause 7
//!   framing, so an Ethernet tap yields no records.
//!
//! # Limits
//!
//! - The tap shows frames before the stack filters them, so a record does
//!   not prove the discovery table took the I-Am: a Forwarded-NPDU a
//!   foreign device refuses from a BBMD it doesn't trust is a record.
//! - With a subscriber, the transport copies every frame once, as for any
//!   tap subscriber. The decode runs in the task that reads the records,
//!   never on the transport's path.
//! - The tap's queue is bounded. When it is full the newest frames are
//!   dropped and counted, of every kind, since the tap cannot tell which held
//!   an I-Am: [`IAmArrivals::missed`], and each record's
//!   [`missed_before`](IAmArrival::missed_before).

use std::net::SocketAddr;
use std::task::{Context, Poll};
use std::time::{Instant, SystemTime};

use bacnet_encoding::npdu::NpduAddress;
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::bvll::encode_bip_mac;
use bacnet_transport::tap::{FrameTapReceiver, TapDirection, TapPeer, TappedFrame};
use bacnet_types::MacAddr;
use bacnet_types::data_link::DataLink;
use bytes::Bytes;

use crate::decode::decode_npdu_layer;
use crate::decode::frame::{Framing, Npdu, link};
use crate::device::DeviceAddress;

/// One I-Am this node received, as the frame tap saw it arrive. See the
/// [module docs](self) for each field on each link.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct IAmArrival {
    /// Wall-clock time the transport handed the frame to the tap.
    pub timestamp: SystemTime,
    /// Monotonic time at the same capture point: use it for elapsed times.
    pub monotonic_timestamp: Instant,
    /// The data link the frame came on.
    pub link: DataLink,
    /// The I-Am's parameters, as decoded: the device identifier, max APDU
    /// length accepted, segmentation supported and vendor identifier.
    pub i_am: IAmRequest,
    /// Who put the frame on this node's link: the tap's peer.
    pub link_sender: TapPeer,
    /// The frame's source MAC as the stack takes it: the origin a
    /// forwarded frame carries, else the link sender; on B/IPv6 and
    /// BACnet/SC a VMAC. The router, when [`Self::source_network`] is set.
    pub link_source: MacAddr,
    /// The originating node's IP address and UDP port a BBMD wrote into a
    /// B/IP or B/IPv6 Forwarded-NPDU.
    pub original_source: Option<SocketAddr>,
    /// The frame carried an origin other than its link sender: a BBMD
    /// forwarded it.
    pub forwarded: bool,
    /// SNET and SADR: the device's network and its MAC there, when a router
    /// relayed the I-Am.
    pub source_network: Option<NpduAddress>,
    /// A link broadcast, or a broadcast on a network.
    pub broadcast: bool,
    /// Frames of any kind the subscription lost just before this record,
    /// since the record before it: see [`IAmArrivals::missed`].
    pub missed_before: u64,
}

/// Why an inbound I-Am frame made no record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MalformedIAm(pub String);

impl std::fmt::Display for MalformedIAm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MalformedIAm {}

/// The framing the decoder reads `link` with, for the links it reads.
fn framing(link: DataLink) -> Option<Framing> {
    match link {
        DataLink::Bip => Some(Framing::Bip),
        DataLink::Bip6 => Some(Framing::Bip6),
        DataLink::Sc => Some(Framing::Sc),
        DataLink::Mstp => Some(Framing::Mstp),
        _ => None,
    }
}

/// Whether two socket addresses are the same IP and port, whatever their
/// IPv6 flow label and scope.
fn same_socket(a: SocketAddr, b: SocketAddr) -> bool {
    a.ip() == b.ip() && a.port() == b.port()
}

impl IAmArrival {
    /// Read `frame` as an I-Am arrival: `Ok(None)` for an outbound frame,
    /// one that is not an I-Am or whose headers don't decode, and one on a
    /// link the decoder doesn't read. `missed_before` is the frame's own.
    ///
    /// # Errors
    ///
    /// [`MalformedIAm`] for an inbound I-Am whose parameters don't decode,
    /// or one that names no source (a BACnet/SC message without an
    /// originating VMAC).
    pub fn from_frame(frame: &TappedFrame) -> Result<Option<Self>, MalformedIAm> {
        if frame.direction != TapDirection::Inbound {
            return Ok(None);
        }
        let Some(framing) = framing(frame.link) else {
            return Ok(None);
        };
        let link = link(framing, &frame.bytes);
        let npdu = match link.npdu {
            Some(Npdu::At(range)) => frame.bytes.slice(range),
            Some(Npdu::Encoded { data, .. }) => Bytes::from(data),
            None => return Ok(None),
        };
        let Ok(decoded) = decode_npdu_layer(npdu) else {
            return Ok(None);
        };
        let Some(i_am) = decoded.apdu.and_then(|apdu| apdu.i_am) else {
            return Ok(None);
        };
        let i_am = i_am.map_err(MalformedIAm)?;
        let link_source = match (link.source_mac, frame.peer) {
            (Some(mac), _) => mac,
            (None, TapPeer::Udp(SocketAddr::V4(sender))) => {
                MacAddr::from_slice(&encode_bip_mac(sender.ip().octets(), sender.port()))
            }
            _ => return Err(MalformedIAm("the frame names no source".into())),
        };
        let forwarded = match (link.origin, frame.peer) {
            (Some(origin), TapPeer::Udp(sender)) => !same_socket(origin, sender),
            _ => false,
        };
        let source_network =
            decoded
                .source_network
                .zip(decoded.source_address)
                .map(|(network, mac_address)| NpduAddress {
                    network,
                    mac_address,
                });
        Ok(Some(Self {
            timestamp: frame.timestamp,
            monotonic_timestamp: frame.monotonic_timestamp,
            link: frame.link,
            i_am,
            link_sender: frame.peer,
            link_source,
            original_source: link.origin,
            forwarded,
            source_network,
            broadcast: link.broadcast || decoded.broadcast,
            missed_before: frame.missed_before,
        }))
    }

    /// The device's address as the client addresses it: its network and
    /// MAC there when a router relayed the I-Am, else its MAC on this link.
    /// `None` when those name no one station (an empty or broadcast MAC, a
    /// local MAC of a length no link here uses).
    pub fn device_address(&self) -> Option<DeviceAddress> {
        match &self.source_network {
            Some(routed) => DeviceAddress::new(routed.network, &routed.mac_address).ok(),
            None => DeviceAddress::local(&self.link_source).ok(),
        }
    }

    /// The router that relayed the I-Am, on this link: [`Self::link_source`]
    /// when the NPDU names a source network.
    pub fn next_hop_router(&self) -> Option<&[u8]> {
        self.source_network
            .as_ref()
            .map(|_| self.link_source.as_slice())
    }

    /// The BBMD that forwarded the I-Am: [`Self::link_sender`] when
    /// [`Self::forwarded`].
    pub fn forwarding_bbmd(&self) -> Option<TapPeer> {
        self.forwarded.then_some(self.link_sender)
    }

    /// Whether the I-Am started at `own`: the origin a forwarded frame
    /// carries, else the UDP sender, as the stack's own-echo rule judges it.
    fn comes_from(&self, own: SocketAddr) -> bool {
        match (self.original_source, self.link_sender) {
            (Some(origin), _) | (None, TapPeer::Udp(origin)) => same_socket(origin, own),
            _ => false,
        }
    }
}

/// The I-Am arrivals on one frame-tap subscription. Dropping it
/// unsubscribes.
pub struct IAmArrivals {
    receiver: FrameTapReceiver,
    own: Option<SocketAddr>,
    /// Frames lost ahead of the frames read since the last record.
    gap: u64,
    malformed: u64,
}

impl std::fmt::Debug for IAmArrivals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IAmArrivals")
            .field("missed", &self.missed())
            .field("malformed", &self.malformed)
            .finish()
    }
}

impl IAmArrivals {
    /// The arrivals on `receiver`, a subscription to a transport's tap.
    /// Its capacity counts frames of every kind, not only I-Ams.
    pub fn new(receiver: FrameTapReceiver) -> Self {
        Self {
            receiver,
            own: None,
            gap: 0,
            malformed: 0,
        }
    }

    /// Leave out this node's own I-Am come back to it: one whose origin is
    /// `own`, the transport's B/IP or B/IPv6 socket address.
    pub fn with_own_address(self, own: SocketAddr) -> Self {
        Self {
            own: Some(own),
            ..self
        }
    }

    /// Poll for the next record. Frames that are no record are read and
    /// passed over in the same poll; cancellation before readiness loses
    /// no record.
    pub fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<IAmArrival>> {
        loop {
            match self.receiver.poll_recv(cx) {
                Poll::Ready(Some(frame)) => {
                    if let Some(arrival) = self.take(&frame) {
                        return Poll::Ready(Some(arrival));
                    }
                }
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }

    /// The next record, waiting for one; `None` once the transport that
    /// owns the tap is dropped and the queue is empty.
    pub async fn recv(&mut self) -> Option<IAmArrival> {
        std::future::poll_fn(|cx| self.poll_recv(cx)).await
    }

    /// The next record among the frames queued now, without waiting.
    pub fn try_recv(&mut self) -> Option<IAmArrival> {
        while let Some(frame) = self.receiver.try_recv() {
            if let Some(arrival) = self.take(&frame) {
                return Some(arrival);
            }
        }
        None
    }

    /// Frames of every kind lost to the full queue so far: the sum of the
    /// records' [`IAmArrival::missed_before`], and the frames lost since
    /// the last record.
    pub fn missed(&self) -> u64 {
        self.receiver.dropped()
    }

    /// Inbound I-Am frames that made no record (see [`MalformedIAm`]).
    pub fn malformed(&self) -> u64 {
        self.malformed
    }

    fn take(&mut self, frame: &TappedFrame) -> Option<IAmArrival> {
        self.gap = self.gap.saturating_add(frame.missed_before);
        match IAmArrival::from_frame(frame) {
            Ok(Some(mut arrival)) if !self.own.is_some_and(|own| arrival.comes_from(own)) => {
                arrival.missed_before = std::mem::take(&mut self.gap);
                Some(arrival)
            }
            Ok(_) => None,
            Err(_) => {
                self.malformed = self.malformed.saturating_add(1);
                None
            }
        }
    }
}

#[cfg(test)]
#[path = "arrival_tests.rs"]
mod tests;
