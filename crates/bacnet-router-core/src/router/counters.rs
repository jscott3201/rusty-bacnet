//! What the router counts.

/// Counts of what the router dropped, sent and was refused. They saturate,
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
    pub address_length_drops: u64,
    /// Frames dropped for a DADR beside DNET 0xFFFF (#1379).
    pub global_broadcast_dadr_drops: u64,
    /// Broadcasts dropped for carrying an APDU other than an
    /// Unconfirmed-Request (#1491).
    pub broadcast_pdu_type_drops: u64,
    /// Frames dropped for a DADR that is a group address on its delivery
    /// port: an APDU other than an Unconfirmed-Request, a network message,
    /// or anything for a [`Group::Refuse`](super::Group::Refuse) address
    /// (#1504, #1677). An Unconfirmed-Request sent as the port's broadcast
    /// instead isn't counted.
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
    /// Protected network messages [`Host::authorize`](super::Host::authorize)
    /// refused.
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
    /// router's own check (R1), or refused by the port as too long.
    pub egress_too_long: u64,
    /// Sends the caller reported refused.
    pub refused: u64,
    /// Actions dropped because a call came before they were polled.
    pub undrained: u64,
}

impl Counters {
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
    };
}

/// Add one to a count, saturating.
pub(super) fn bump(count: &mut u64) {
    *count = count.saturating_add(1);
}
