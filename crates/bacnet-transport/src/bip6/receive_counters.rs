//! What the B/IPv6 receive path drops, counted by reason, the replies it
//! holds back over its quotas, and the throttles on the warnings it could
//! otherwise repeat for every datagram (#1643). The B/IP transport keeps its
//! own set (`bip::ReceiveCounters`, #1624): the two drop for different
//! reasons.

use std::sync::atomic::{AtomicU64, Ordering};

use tracing::{debug, error, warn};

use super::{Bip6Vmac, Bvlc6ResultCode, Bvll6Error};
use crate::warn_throttle::WarnThrottle;

/// Datagrams a B/IPv6 transport's receive path dropped since the transport
/// was created, by reason, and the replies it held back over its quotas. The
/// totals survive stop and restart.
///
/// A dropped datagram counts once, under the first check it fails. A
/// Forwarded-NPDU whose origin is a group address is counted in
/// [`forwarded_group_origin_drops`](super::Bip6Transport::forwarded_group_origin_drops)
/// instead. A message the receive path handles is not a drop, even when it
/// calls for nothing: a request to resolve another node's VMAC, whether
/// forwarded or not, an ACK, or a BVLC-Result. The last two fields
/// count replies, not datagrams: the request they answer was handled. Each
/// field is read on its own, so a snapshot taken while datagrams arrive need
/// not add up across fields. What the VMAC collision probe reads during
/// `start`, before the receive loop runs, is not counted.
///
/// It is `#[non_exhaustive]`, so a later counter isn't a breaking change:
/// read its fields, or start from `Default` in a test.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Bip6ReceiveCounters {
    /// Failed socket reads, and datagrams without usable address metadata.
    /// Most drop one datagram and reading goes on: one larger than the
    /// receive buffer, one without usable metadata, and on Windows the
    /// receive errors it reports for an earlier send. Any other error stops
    /// the receive loop.
    pub receive_errors: u64,
    /// Datagrams the socket read for another link: in normal mode, one that
    /// arrived on another interface or was sent to an address that is
    /// neither the selected one nor a BACnet group. The socket is bound to
    /// the wildcard address, so this is routine on a host with more than one
    /// interface or address.
    pub link_mismatch: u64,
    /// Datagrams shorter than the 4-octet BVLL header.
    pub too_short: u64,
    /// Datagrams whose BVLC type octet is not B/IPv6's (0x82), such as other
    /// traffic sent to the port.
    pub not_bip6: u64,
    /// Datagrams whose BVLC Length field differs from the datagram's length:
    /// cut short, or with octets after the message.
    pub length_mismatch: u64,
    /// Frames whose BVLC function Annex U doesn't define, function 0x0B
    /// among them.
    pub unknown_function: u64,
    /// Frames whose payload doesn't fit their BVLC function's layout. A
    /// malformed Register-Foreign-Device, Delete-Foreign-Device-Table-Entry
    /// or Distribute-Broadcast-To-Network still draws the NAK a well-formed
    /// one would (#1638). One refused that NAK for its source is counted
    /// here only, not under `unanswerable_request` too.
    pub malformed_payload: u64,
    /// Original-Unicast-NPDUs, Address-Resolution-ACKs and
    /// Virtual-Address-Resolution-ACKs whose destination VMAC is another
    /// node's (Annex U.3).
    pub destination_vmac_mismatch: u64,
    /// Frames whose BVLC function doesn't fit the address they were sent to:
    /// an Original-Unicast-NPDU, an ACK, or a request only this node answers
    /// sent anywhere but this node's own address, or an
    /// Original-Broadcast-NPDU or Address-Resolution sent anywhere but a
    /// BACnet group.
    pub destination_mismatch: u64,
    /// Forwarded-NPDUs that came by a path this node doesn't take them from:
    /// in normal mode anything but a BACnet group, and for a foreign device
    /// anything but unicast from its configured BBMD.
    pub untrusted_forwarded_npdu: u64,
    /// Forwarded-Address-Resolutions that came by a path this node doesn't
    /// answer: for a foreign device anything but unicast from its configured
    /// BBMD, and otherwise anything but a BACnet group from an answerable
    /// source.
    pub untrusted_forwarded_resolution: u64,
    /// Forwarded-NPDUs whose NPDU is too short to start one or doesn't
    /// decode as one.
    pub malformed_forwarded_npdu: u64,
    /// Forwarded-NPDUs whose origin, and Forwarded-Address-Resolutions for
    /// this node whose asker, is an address no answer could reach without a
    /// scope ID: unspecified, loopback, link-local, IPv4-mapped or port 0.
    pub unusable_address: u64,
    /// Requests this node would answer, refused for their source: one no
    /// node sends from (a group, the unspecified address, an IPv4-mapped
    /// address or port 0), or this node's own address, which never sends it
    /// a request.
    pub unanswerable_request: u64,
    /// Original-Unicast-NPDUs and Original-Broadcast-NPDUs from this node's
    /// own address, and Forwarded-NPDUs that name it as their origin: mostly
    /// its own broadcasts coming back, directly or through a BBMD, which is
    /// routine.
    pub own_echo: u64,
    /// Frames from another address that name this node's VMAC as their
    /// sender's: Original-Unicast-NPDUs, Original-Broadcast-NPDUs,
    /// Forwarded-NPDUs (by their origin's VMAC) and Address-Resolution-ACKs
    /// and Virtual-Address-Resolution-ACKs. Another node uses this node's
    /// VMAC, or forges it; each also feeds the throttled collision warning.
    pub own_vmac_source: u64,
    /// Original-Unicast-NPDUs and Original-Broadcast-NPDUs from a source no
    /// answer could reach as one node (a group, the unspecified address, an
    /// IPv4-mapped address or port 0). The transport never learns such a
    /// source, so a reply to its VMAC would reach whoever holds that VMAC,
    /// or wait on a resolution.
    pub unanswerable_source: u64,
    /// NPDUs dropped because the channel to the network layer was full: the
    /// network layer is falling behind.
    pub npdu_channel_full: u64,
    /// NPDUs dropped because the network layer had stopped reading and
    /// closed its end of the channel, as while the stack shuts down.
    pub npdu_channel_closed: u64,
    /// NAKs for the requests only a BBMD serves, held back over the NAK
    /// quota, whether the request was well-formed or not. A malformed one
    /// is also counted in `malformed_payload`.
    pub naks_suppressed: u64,
    /// Address-Resolution-ACKs answering a Forwarded-Address-Resolution,
    /// held back over their own quota.
    pub far_acks_suppressed: u64,
}

/// Why the receive path dropped a datagram or held back a reply: one per
/// [`Bip6ReceiveCounters`] field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DropReason {
    /// [`Bip6ReceiveCounters::receive_errors`].
    ReceiveError,
    /// [`Bip6ReceiveCounters::link_mismatch`].
    LinkMismatch,
    /// [`Bip6ReceiveCounters::too_short`].
    TooShort,
    /// [`Bip6ReceiveCounters::not_bip6`].
    NotBip6,
    /// [`Bip6ReceiveCounters::length_mismatch`].
    LengthMismatch,
    /// [`Bip6ReceiveCounters::unknown_function`].
    UnknownFunction,
    /// [`Bip6ReceiveCounters::malformed_payload`].
    MalformedPayload,
    /// [`Bip6ReceiveCounters::destination_vmac_mismatch`].
    DestinationVmacMismatch,
    /// [`Bip6ReceiveCounters::destination_mismatch`].
    DestinationMismatch,
    /// [`Bip6ReceiveCounters::untrusted_forwarded_npdu`].
    UntrustedForwardedNpdu,
    /// [`Bip6ReceiveCounters::untrusted_forwarded_resolution`].
    UntrustedForwardedResolution,
    /// [`Bip6ReceiveCounters::malformed_forwarded_npdu`].
    MalformedForwardedNpdu,
    /// [`Bip6ReceiveCounters::unusable_address`].
    UnusableAddress,
    /// [`Bip6ReceiveCounters::unanswerable_request`].
    UnanswerableRequest,
    /// [`Bip6ReceiveCounters::own_echo`].
    OwnEcho,
    /// [`Bip6ReceiveCounters::own_vmac_source`].
    OwnVmacSource,
    /// [`Bip6ReceiveCounters::unanswerable_source`].
    UnanswerableSource,
    /// [`Bip6ReceiveCounters::npdu_channel_full`].
    NpduChannelFull,
    /// [`Bip6ReceiveCounters::npdu_channel_closed`].
    NpduChannelClosed,
    /// [`Bip6ReceiveCounters::naks_suppressed`].
    NakSuppressed,
    /// [`Bip6ReceiveCounters::far_acks_suppressed`].
    FarAckSuppressed,
}

impl DropReason {
    /// Why a datagram failed to decode, in its header
    /// ([`Bvll6Frame::decode`](super::Bvll6Frame::decode)) or its payload
    /// ([`Bvll6Frame::message`](super::Bvll6Frame::message)). Every error the
    /// codec has today is named; one it adds later counts as a malformed
    /// payload until it is filed here.
    pub(super) fn of_decode_error(error: &Bvll6Error) -> Self {
        match error {
            Bvll6Error::TooShort { .. } => Self::TooShort,
            Bvll6Error::NotBip6 { .. } => Self::NotBip6,
            Bvll6Error::LengthMismatch { .. } => Self::LengthMismatch,
            Bvll6Error::UnknownFunction { .. } => Self::UnknownFunction,
            Bvll6Error::PayloadLength { .. } | Bvll6Error::AddressLength { .. } => {
                Self::MalformedPayload
            }
            // Encoding errors, which decoding never returns.
            Bvll6Error::TooLong { .. }
            | Bvll6Error::UndefinedResultCode { .. }
            | Bvll6Error::BufferExhausted { .. } => Self::MalformedPayload,
            _ => Self::MalformedPayload,
        }
    }
}

/// The receive path's counts, one per [`DropReason`], which the transport
/// keeps across restarts and lends to each receive loop, and the throttles
/// on its three repeating warnings: a full channel, a VMAC collision and a
/// BVLC-Result NAK.
#[derive(Debug, Default)]
pub(super) struct ReceiveDrops {
    receive_errors: AtomicU64,
    link_mismatch: AtomicU64,
    too_short: AtomicU64,
    not_bip6: AtomicU64,
    length_mismatch: AtomicU64,
    unknown_function: AtomicU64,
    malformed_payload: AtomicU64,
    destination_vmac_mismatch: AtomicU64,
    destination_mismatch: AtomicU64,
    untrusted_forwarded_npdu: AtomicU64,
    untrusted_forwarded_resolution: AtomicU64,
    malformed_forwarded_npdu: AtomicU64,
    unusable_address: AtomicU64,
    unanswerable_request: AtomicU64,
    own_echo: AtomicU64,
    own_vmac_source: AtomicU64,
    unanswerable_source: AtomicU64,
    npdu_channel_full: AtomicU64,
    npdu_channel_closed: AtomicU64,
    naks_suppressed: AtomicU64,
    far_acks_suppressed: AtomicU64,
    channel_full: WarnThrottle,
    vmac_collision: WarnThrottle,
    result_nak: WarnThrottle,
}

impl ReceiveDrops {
    fn counter(&self, reason: DropReason) -> &AtomicU64 {
        match reason {
            DropReason::ReceiveError => &self.receive_errors,
            DropReason::LinkMismatch => &self.link_mismatch,
            DropReason::TooShort => &self.too_short,
            DropReason::NotBip6 => &self.not_bip6,
            DropReason::LengthMismatch => &self.length_mismatch,
            DropReason::UnknownFunction => &self.unknown_function,
            DropReason::MalformedPayload => &self.malformed_payload,
            DropReason::DestinationVmacMismatch => &self.destination_vmac_mismatch,
            DropReason::DestinationMismatch => &self.destination_mismatch,
            DropReason::UntrustedForwardedNpdu => &self.untrusted_forwarded_npdu,
            DropReason::UntrustedForwardedResolution => &self.untrusted_forwarded_resolution,
            DropReason::MalformedForwardedNpdu => &self.malformed_forwarded_npdu,
            DropReason::UnusableAddress => &self.unusable_address,
            DropReason::UnanswerableRequest => &self.unanswerable_request,
            DropReason::OwnEcho => &self.own_echo,
            DropReason::OwnVmacSource => &self.own_vmac_source,
            DropReason::UnanswerableSource => &self.unanswerable_source,
            DropReason::NpduChannelFull => &self.npdu_channel_full,
            DropReason::NpduChannelClosed => &self.npdu_channel_closed,
            DropReason::NakSuppressed => &self.naks_suppressed,
            DropReason::FarAckSuppressed => &self.far_acks_suppressed,
        }
    }

    /// Count one datagram dropped, or one reply held back, for `reason`.
    pub(super) fn count(&self, reason: DropReason) {
        self.counter(reason).fetch_add(1, Ordering::Relaxed);
    }

    /// Count an NPDU dropped because the network layer's channel is full,
    /// and warn at most once per
    /// [`WARNING_INTERVAL`](crate::warn_throttle::WARNING_INTERVAL) with the
    /// number dropped since the last warning, so that a sustained overload
    /// shows without flooding the log; the rest log at debug. `frame` names
    /// the kind of frame dropped. Returns the number reported, when it warns.
    pub(super) fn npdu_channel_full(&self, frame: &'static str) -> Option<u64> {
        self.count(DropReason::NpduChannelFull);
        let dropped = self.channel_full.note();
        match dropped {
            Some(dropped) => warn!(
                dropped,
                frame, "B/IPv6: NPDU channel full; dropped incoming frames since the last warning"
            ),
            None => debug!(
                frame,
                "B/IPv6: NPDU channel full, dropping an incoming frame"
            ),
        }
        dropped
    }

    /// Log a frame from another address that uses this node's own VMAC: a
    /// collision, or a forgery. It warns at most once per interval, with the
    /// frames seen since the last warning, since anyone can send one and a
    /// flood must not flood the log; the rest log at debug. Returns the
    /// number reported, when it warns.
    pub(super) fn vmac_collision(&self, vmac: Bip6Vmac) -> Option<u64> {
        let seen = self.vmac_collision.note();
        match seen {
            Some(seen) => warn!(
                ?vmac,
                seen,
                "BIP6 VMAC collision detected! Another node sends from our VMAC \
                 (frames since the last warning)"
            ),
            None => debug!(?vmac, "BIP6 VMAC collision: another frame from our VMAC"),
        }
        seen
    }

    /// Log a BVLC-Result NAK, as a foreign device's BBMD sends to refuse a
    /// registration. It logs at error at most once per interval, with the
    /// NAKs since the last one, since anyone can send one; the rest log at
    /// debug. A BBMD's answers to re-registration come far less often than
    /// that, so each of them still logs at error. Returns the number
    /// reported, when it logs at error.
    pub(super) fn result_nak(&self, code: Bvlc6ResultCode) -> Option<u64> {
        let naks = self.result_nak.note();
        match naks {
            Some(naks) => error!(
                code = code.to_raw(),
                naks, "BIP6: BVLC-Result NAK (NAKs since the last report)"
            ),
            None => debug!(code = code.to_raw(), "BIP6: BVLC-Result NAK"),
        }
        naks
    }

    /// The counts so far.
    pub(super) fn snapshot(&self) -> Bip6ReceiveCounters {
        let get = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        Bip6ReceiveCounters {
            receive_errors: get(&self.receive_errors),
            link_mismatch: get(&self.link_mismatch),
            too_short: get(&self.too_short),
            not_bip6: get(&self.not_bip6),
            length_mismatch: get(&self.length_mismatch),
            unknown_function: get(&self.unknown_function),
            malformed_payload: get(&self.malformed_payload),
            destination_vmac_mismatch: get(&self.destination_vmac_mismatch),
            destination_mismatch: get(&self.destination_mismatch),
            untrusted_forwarded_npdu: get(&self.untrusted_forwarded_npdu),
            untrusted_forwarded_resolution: get(&self.untrusted_forwarded_resolution),
            malformed_forwarded_npdu: get(&self.malformed_forwarded_npdu),
            unusable_address: get(&self.unusable_address),
            unanswerable_request: get(&self.unanswerable_request),
            own_echo: get(&self.own_echo),
            own_vmac_source: get(&self.own_vmac_source),
            unanswerable_source: get(&self.unanswerable_source),
            npdu_channel_full: get(&self.npdu_channel_full),
            npdu_channel_closed: get(&self.npdu_channel_closed),
            naks_suppressed: get(&self.naks_suppressed),
            far_acks_suppressed: get(&self.far_acks_suppressed),
        }
    }
}
