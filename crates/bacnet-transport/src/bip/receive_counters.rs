//! What the B/IP receive path drops, counted by reason (#1624), and the
//! throttles on the warnings it could otherwise repeat for every datagram.

use std::io;
use std::net::SocketAddrV4;
use std::sync::atomic::{AtomicU64, Ordering};

use bacnet_bip_codec::BvllError;
use bacnet_bip_core::node::Discard;
use tracing::{debug, warn};

use crate::warn_throttle::WarnThrottle;

/// Datagrams a B/IP transport's receive path dropped since the transport was
/// created, by reason. The totals survive stop and restart.
///
/// A dropped datagram counts once, under the first check it fails. Three
/// drops are counted elsewhere: a datagram whose UDP source is a group
/// address, in [`group_source_drops`](super::BipTransport::group_source_drops);
/// a Forwarded-NPDU whose origin is one, in
/// [`forwarded_group_origin_drops`](super::BipTransport::forwarded_group_origin_drops);
/// and a well-formed BVLC-Result, Read-BDT-ACK or Read-FDT-ACK that no
/// request was waiting for, in the `unmatched_responses` of
/// [`bvlc_client_snapshot`](super::BipTransport::bvlc_client_snapshot).
/// Each field is read on its own, so a snapshot taken while datagrams arrive
/// need not add up across fields.
///
/// It is `#[non_exhaustive]`, so a later counter isn't a breaking change:
/// read its fields, or start from `Default` in a test.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReceiveCounters {
    /// Failed socket reads. Most drop one datagram and reading goes on: one
    /// larger than the receive buffer, one without usable address metadata,
    /// and on Windows the receive errors it reports for an earlier send. Any
    /// other error closes the socket: a broadcast listener closes alone, and
    /// on the socket every send leaves from, the transport stops receiving.
    pub receive_errors: u64,
    /// Datagrams a broadcast listener read but doesn't keep, in per-address
    /// mode ([`set_share_port_by_address`](super::BipTransport::set_share_port_by_address)),
    /// whatever they hold: one not sent to this link's broadcast address,
    /// such as a unicast for a local address no socket on the port is bound
    /// to or another subnet's broadcast, or a broadcast that came in on
    /// another interface. Routine on a shared port or a multihomed host.
    pub listener_mismatch: u64,
    /// Datagrams shorter than the 4-octet BVLL header.
    pub too_short: u64,
    /// Datagrams whose BVLC type octet is not B/IP's (0x81), such as other
    /// traffic sent to the port.
    pub not_bip: u64,
    /// Datagrams whose BVLC Length field differs from the datagram's length:
    /// cut short, or with octets after the message.
    pub length_mismatch: u64,
    /// Frames whose BVLC function doesn't fit the address they were sent to:
    /// an Original-Broadcast-NPDU not sent to a broadcast address, or any
    /// function but it and Forwarded-NPDU not sent to this node's own
    /// address (Annex J.2, J.4).
    pub destination_mismatch: u64,
    /// BBMD management requests (Read-BDT, Read-FDT, Register-Foreign-Device
    /// and Delete-FDT-Entry) discarded unanswered over the management rate
    /// limit.
    pub rate_limited: u64,
    /// Frames whose BVLC function Annex J doesn't define.
    pub unknown_function: u64,
    /// Frames whose payload doesn't fit their BVLC function's layout. A
    /// malformed BBMD request still draws its NAK. A malformed BVLC-Result,
    /// Read-BDT-ACK or Read-FDT-ACK is also counted in the
    /// `malformed_responses` of
    /// [`bvlc_client_snapshot`](super::BipTransport::bvlc_client_snapshot),
    /// whether or not a request was waiting for it, and a malformed ACK
    /// still reaches a request waiting for it.
    pub malformed_payload: u64,
    /// Frames from this node's own B/IP address, and Forwarded-NPDUs naming
    /// it as their origin: mostly its own broadcasts coming back, which is
    /// routine.
    pub own_echo: u64,
    /// Requests this node refuses with a NAK, Write-BDT and the five only a
    /// BBMD serves, dropped unanswered instead for their source: one no node
    /// sends from (the unspecified address, port 0 or a group address), or
    /// this node's own address, which never sends it a request (#1622). A
    /// malformed one counts in `malformed_payload` only.
    pub unanswerable_request: u64,
    /// Forwarded-NPDUs a BBMD refused because their sender is not in its
    /// Broadcast Distribution Table.
    pub forwarded_not_from_peer: u64,
    /// Forwarded-NPDUs a foreign device refused because they didn't come
    /// from its BBMD by unicast, the one way a BBMD sends a foreign device
    /// the network's broadcasts (#1622). A node in normal mode takes them
    /// from any sender.
    pub forwarded_untrusted: u64,
    /// NPDUs dropped because the channel to the network layer was full: the
    /// network layer is falling behind. A BBMD still forwards a broadcast it
    /// couldn't hand up.
    pub npdu_channel_full: u64,
    /// NPDUs dropped because the network layer had stopped reading and
    /// closed its end of the channel, as while the stack shuts down.
    pub npdu_channel_closed: u64,
}

/// Why the receive path dropped a datagram: one per [`ReceiveCounters`]
/// field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DropReason {
    /// [`ReceiveCounters::receive_errors`].
    ReceiveError,
    /// [`ReceiveCounters::listener_mismatch`].
    ListenerMismatch,
    /// [`ReceiveCounters::too_short`].
    TooShort,
    /// [`ReceiveCounters::not_bip`].
    NotBip,
    /// [`ReceiveCounters::length_mismatch`].
    LengthMismatch,
    /// [`ReceiveCounters::destination_mismatch`].
    DestinationMismatch,
    /// [`ReceiveCounters::rate_limited`].
    RateLimited,
    /// [`ReceiveCounters::unknown_function`].
    UnknownFunction,
    /// [`ReceiveCounters::malformed_payload`].
    MalformedPayload,
    /// [`ReceiveCounters::own_echo`].
    OwnEcho,
    /// [`ReceiveCounters::unanswerable_request`].
    UnanswerableRequest,
    /// [`ReceiveCounters::forwarded_not_from_peer`].
    ForwardedNotFromPeer,
    /// [`ReceiveCounters::forwarded_untrusted`].
    ForwardedUntrusted,
    /// [`ReceiveCounters::npdu_channel_full`].
    NpduChannelFull,
    /// [`ReceiveCounters::npdu_channel_closed`].
    NpduChannelClosed,
}

impl DropReason {
    /// Why a datagram failed to decode, in its header
    /// ([`BvllFrame::decode`](bacnet_bip_codec::BvllFrame::decode)) or its
    /// payload ([`BvllFrame::message`](bacnet_bip_codec::BvllFrame::message)).
    /// Every error the codec has today is named; one it adds later counts as
    /// a malformed payload until it is filed here.
    pub(super) fn of_decode_error(error: &BvllError) -> Self {
        match error {
            BvllError::TooShort { .. } => Self::TooShort,
            BvllError::NotBip { .. } => Self::NotBip,
            BvllError::LengthMismatch { .. } => Self::LengthMismatch,
            BvllError::UnknownFunction { .. } => Self::UnknownFunction,
            BvllError::PayloadLength { .. }
            | BvllError::TableLength { .. }
            | BvllError::AddressLength { .. } => Self::MalformedPayload,
            // Encoding errors, which decoding never returns.
            BvllError::TooLong { .. }
            | BvllError::UndefinedResultCode { .. }
            | BvllError::BufferExhausted { .. } => Self::MalformedPayload,
            _ => Self::MalformedPayload,
        }
    }

    /// The counter of a datagram the node engine discarded, one for one, or
    /// `None` where another counter keeps it: a group sender, in
    /// [`GroupDrops`](super::groups::GroupDrops).
    pub(super) fn of_discard(discard: Discard) -> Option<Self> {
        Some(match discard {
            // The receive buffer cut it: reading fails before the node.
            Discard::Truncated => Self::ReceiveError,
            Discard::ListenerMismatch { .. } => Self::ListenerMismatch,
            Discard::TooShort => Self::TooShort,
            Discard::NotBip => Self::NotBip,
            Discard::LengthMismatch => Self::LengthMismatch,
            // The transport always passes the destination its socket
            // reported, so the node never finds it unknown.
            Discard::DestinationMismatch | Discard::UnknownDestination => Self::DestinationMismatch,
            Discard::RateLimited => Self::RateLimited,
            Discard::UnknownFunction => Self::UnknownFunction,
            Discard::MalformedPayload => Self::MalformedPayload,
            Discard::OwnEcho => Self::OwnEcho,
            Discard::UnanswerableSource => Self::UnanswerableRequest,
            Discard::ForwardedUntrusted => Self::ForwardedUntrusted,
            Discard::GroupSource | Discard::GroupOrigin => return None,
            // A reason the core has and this transport has no counter for
            // yet: counted with the payloads it couldn't use.
            _ => Self::MalformedPayload,
        })
    }
}

/// The receive path's drop counts, one per [`DropReason`], which the
/// transport keeps across restarts and lends to each receive loop, and the
/// throttles on its two repeating warnings: a full channel, and a failed
/// Forwarded-NPDU send.
#[derive(Debug, Default)]
pub(super) struct ReceiveDrops {
    receive_errors: AtomicU64,
    listener_mismatch: AtomicU64,
    too_short: AtomicU64,
    not_bip: AtomicU64,
    length_mismatch: AtomicU64,
    destination_mismatch: AtomicU64,
    rate_limited: AtomicU64,
    unknown_function: AtomicU64,
    malformed_payload: AtomicU64,
    own_echo: AtomicU64,
    unanswerable_request: AtomicU64,
    forwarded_not_from_peer: AtomicU64,
    forwarded_untrusted: AtomicU64,
    npdu_channel_full: AtomicU64,
    npdu_channel_closed: AtomicU64,
    channel_full: WarnThrottle,
    forward_failed: WarnThrottle,
}

impl ReceiveDrops {
    fn counter(&self, reason: DropReason) -> &AtomicU64 {
        match reason {
            DropReason::ReceiveError => &self.receive_errors,
            DropReason::ListenerMismatch => &self.listener_mismatch,
            DropReason::TooShort => &self.too_short,
            DropReason::NotBip => &self.not_bip,
            DropReason::LengthMismatch => &self.length_mismatch,
            DropReason::DestinationMismatch => &self.destination_mismatch,
            DropReason::RateLimited => &self.rate_limited,
            DropReason::UnknownFunction => &self.unknown_function,
            DropReason::MalformedPayload => &self.malformed_payload,
            DropReason::OwnEcho => &self.own_echo,
            DropReason::UnanswerableRequest => &self.unanswerable_request,
            DropReason::ForwardedNotFromPeer => &self.forwarded_not_from_peer,
            DropReason::ForwardedUntrusted => &self.forwarded_untrusted,
            DropReason::NpduChannelFull => &self.npdu_channel_full,
            DropReason::NpduChannelClosed => &self.npdu_channel_closed,
        }
    }

    /// Count one datagram dropped for `reason`.
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
                frame, "B/IP: NPDU channel full; dropped incoming frames since the last warning"
            ),
            None => debug!(frame, "B/IP: NPDU channel full, dropping an incoming frame"),
        }
        dropped
    }

    /// Log a Forwarded-NPDU the receive path failed to send while
    /// forwarding a broadcast: to the local subnet, or, without a fan-out
    /// worker, to a BDT peer or foreign device (#1643). It warns at most once
    /// per [`WARNING_INTERVAL`](crate::warn_throttle::WARNING_INTERVAL), with
    /// the failures since the last warning, and logs the rest at debug. A
    /// send that fails tends to fail for every datagram, as when a route is
    /// missing, and no counter keeps these failures, so the warning stays.
    /// Returns the number reported, when it warns.
    pub(super) fn forward_failed(&self, error: &io::Error, dest: SocketAddrV4) -> Option<u64> {
        let failed = self.forward_failed.note();
        match failed {
            Some(failed) => warn!(
                %error,
                %dest,
                failed,
                "B/IP: failed to send a Forwarded-NPDU; failures since the last warning"
            ),
            None => debug!(%error, %dest, "B/IP: failed to send a Forwarded-NPDU"),
        }
        failed
    }

    /// The counts so far.
    pub(super) fn snapshot(&self) -> ReceiveCounters {
        let get = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        ReceiveCounters {
            receive_errors: get(&self.receive_errors),
            listener_mismatch: get(&self.listener_mismatch),
            too_short: get(&self.too_short),
            not_bip: get(&self.not_bip),
            length_mismatch: get(&self.length_mismatch),
            destination_mismatch: get(&self.destination_mismatch),
            rate_limited: get(&self.rate_limited),
            unknown_function: get(&self.unknown_function),
            malformed_payload: get(&self.malformed_payload),
            own_echo: get(&self.own_echo),
            unanswerable_request: get(&self.unanswerable_request),
            forwarded_not_from_peer: get(&self.forwarded_not_from_peer),
            forwarded_untrusted: get(&self.forwarded_untrusted),
            npdu_channel_full: get(&self.npdu_channel_full),
            npdu_channel_closed: get(&self.npdu_channel_closed),
        }
    }
}
