//! The hosted router's counts: the totals since start of what the engine
//! counts.
//!
//! The engine counts as the portable cores do: `u32` counts it hands over,
//! and starts again, each time they are taken
//! ([`Router::take_counters`](bacnet_router_core::Router::take_counters)).
//! The hosted router takes them each time it lets the engine's lock go and
//! adds them to these totals, so its users read cumulative `u64` counts.

/// What the hosted router's engine has counted since the router started:
/// what it dropped, sent and was refused. Each total saturates at
/// `u64::MAX`. They never change a decision, and name no peer.
///
/// It is `#[non_exhaustive]`, so a new count isn't a breaking change.
/// Outside this crate, build one from `Counters::default()` and set its
/// fields.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Counters {
    /// Frames refused because the link source, the DLEN or the SLEN was
    /// past [`MAX_MAC_LEN`](bacnet_npdu::MAX_MAC_LEN) (#1141,
    /// #1198).
    pub address_length_drops: u64,
    /// Frames dropped for a DADR beside DNET 0xFFFF (#1379).
    pub global_broadcast_dadr_drops: u64,
    /// Broadcasts dropped for carrying an APDU other than an
    /// Unconfirmed-Request (#1491).
    pub broadcast_pdu_type_drops: u64,
    /// Frames dropped for a DADR that is a group address on its delivery
    /// port: an APDU other than an Unconfirmed-Request, a network message,
    /// or anything for a group the port refuses (#1504, #1677). An
    /// Unconfirmed-Request sent as the port's broadcast instead isn't
    /// counted.
    pub group_dadr_drops: u64,
    /// Frames that didn't parse as an NPDU for any other reason.
    pub malformed: u64,
    /// Network messages whose data or addressing the router refused.
    pub bad_messages: u64,
    /// Frames that came on a port the router doesn't have, or whose route
    /// names one.
    pub unknown_port: u64,
    /// Relays not sent because the hop count had run out.
    pub hop_count_drops: u64,
    /// Protected network messages the control policy refused.
    pub denied: u64,
    /// Received rejects that were neither for this router nor routable.
    pub unroutable_rejects: u64,
    /// Rejects this router queued.
    pub rejects: u64,
    /// Who-Is-Router searches for unknown networks queued.
    pub solicitations: u64,
    /// Outputs that couldn't be written: longer than the scratch, or with
    /// a field no NPDU may carry, such as an SNET of 0.
    pub write_failures: u64,
    /// Sends left out because their egress port can't carry them: by the
    /// router's own check, or refused by the port's transport as too long,
    /// at once or later.
    pub egress_too_long: u64,
    /// Sends a port reported refused, at once or later.
    pub refused: u64,
    /// Actions dropped because a call came before they were polled.
    pub undrained: u64,
    /// Router-Busy announcements started for a clogged port, repeats
    /// included ([`RouterOptions::congestion`](super::RouterOptions::congestion), #1697).
    pub busy_announced: u64,
    /// Router-Available announcements started for a port that drained
    /// (#1697).
    pub available_announced: u64,
    /// Global broadcasts, and the network messages the router passes on,
    /// dropped as repeats
    /// ([`RouterOptions::duplicates`](super::RouterOptions::duplicates),
    /// #1698, #1810, #2020).
    pub duplicate_drops: u64,
    /// Global broadcasts and Who-Is-Router queries dropped because their
    /// SNET and SADR name the router itself: its own, come back round a
    /// loop (#2021).
    pub own_source_drops: u64,
    /// Networks a received reject or flow-control message named whose
    /// learned route goes through another router, left unchanged
    /// ([`RouterOptions::next_hop_rejects`](super::RouterOptions::next_hop_rejects), #1698).
    pub not_next_hop: u64,
    /// Requests the router's own application sent through the engine
    /// ([`BACnetRouter::originate`](super::BACnetRouter::originate), #1699).
    pub originated: u64,
}
