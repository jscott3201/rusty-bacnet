//! What the router counts.

use crate::node::Count;

/// Counts of what the router dropped, sent and was refused since its
/// counters were last taken ([`Router::take_counters`](super::Router::take_counters)).
/// Each is a `u32` that saturates between takes; add them to your own
/// totals, as wide as you need, as the link cores' counts are added. They
/// never change a decision, and name no peer. The table's own counts are
/// [`Table::claim_counters`](crate::table::Table::claim_counters).
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
    /// Frames dropped for a DADR that is a group address on its delivery
    /// port: an APDU other than an Unconfirmed-Request, a network message,
    /// or anything for a [`Group::Refuse`](super::Group::Refuse) address
    /// (#1504, #1677). An Unconfirmed-Request sent as the port's broadcast
    /// instead isn't counted.
    pub group_dadr_drops: u32,
    /// Frames that didn't parse as an NPDU for any other reason.
    pub malformed: u32,
    /// Network messages whose data or addressing the router refused.
    pub bad_messages: u32,
    /// Frames that came on a port the router doesn't have, or whose route
    /// names one.
    pub unknown_port: u32,
    /// Relays not sent because the hop count had run out.
    pub hop_count_drops: u32,
    /// Protected network messages [`Host::authorize`](super::Host::authorize)
    /// refused.
    pub denied: u32,
    /// Received rejects that were neither for this router nor routable.
    pub unroutable_rejects: u32,
    /// Rejects this router queued.
    pub rejects: u32,
    /// Who-Is-Router searches for unknown networks queued.
    pub solicitations: u32,
    /// Outputs that couldn't be written: longer than the scratch, or with
    /// a field no NPDU may carry, such as an SNET of 0.
    pub write_failures: u32,
    /// Sends left out because their egress port can't carry them: by the
    /// router's own check (R1), or refused by the port as too long, at once
    /// or later ([`Router::refused`](super::Router::refused),
    /// [`Router::refused_later`](super::Router::refused_later)).
    pub egress_too_long: u32,
    /// Sends the caller reported refused, at once or later.
    pub refused: u32,
    /// Actions dropped because a call came before they were polled.
    pub undrained: u32,
    /// Router-Busy announcements started for a clogged port, repeats
    /// included (#1697). One may take several messages.
    pub busy_announced: u32,
    /// Router-Available announcements started for a port that drained
    /// (#1697).
    pub available_announced: u32,
    /// Global broadcasts, relayed I-Am-Router, Router-Busy and
    /// Router-Available messages, and Who-Is-Router queries the router
    /// would pass on, dropped as repeats of one seen within the
    /// [`Duplicates`](super::Duplicates) time to live (#1698, #1810,
    /// #2020).
    pub duplicate_drops: u32,
    /// Global broadcasts and Who-Is-Router queries dropped because their
    /// SNET and SADR name the router itself: its own, come back round a
    /// loop (#2021). Counted whether or not
    /// [`Options::duplicates`](super::Options::duplicates) is on.
    pub own_source_drops: u32,
    /// Networks a received reject or flow-control message named whose
    /// learned route goes through another router, left unchanged (#1698):
    /// rejects under [`Options::next_hop_rejects`](super::Options::next_hop_rejects),
    /// Router-Busy and Router-Available always.
    pub not_next_hop: u32,
    /// Requests the router's own application sent through the engine
    /// ([`Router::originate`](super::Router::originate), #1699).
    pub originated: u32,
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

    /// All zero.
    pub(super) const ZERO: Self = Self {
        address_length_drops: 0,
        global_broadcast_dadr_drops: 0,
        broadcast_pdu_type_drops: 0,
        group_dadr_drops: 0,
        malformed: 0,
        bad_messages: 0,
        unknown_port: 0,
        hop_count_drops: 0,
        denied: 0,
        unroutable_rejects: 0,
        rejects: 0,
        solicitations: 0,
        write_failures: 0,
        egress_too_long: 0,
        refused: 0,
        undrained: 0,
        busy_announced: 0,
        available_announced: 0,
        duplicate_drops: 0,
        own_source_drops: 0,
        not_next_hop: 0,
        originated: 0,
    };
}

/// Add one to a count, saturating.
pub(crate) fn bump(count: &mut u32) {
    *count = count.saturating_add(1);
}
