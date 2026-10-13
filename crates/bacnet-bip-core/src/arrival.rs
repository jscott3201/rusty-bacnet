//! How a datagram reached this node.

/// How a datagram reached this node: addressed to it, or to the subnet's
/// broadcast address. A BBMD's rule for a peer's Forwarded-NPDU depends on
/// it (Annex J.4.5), and a B/IP node classifies every datagram it receives
/// this way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrival {
    /// Addressed to this node.
    Unicast,
    /// Addressed to the subnet's broadcast address, so every node on the
    /// subnet received it too.
    Broadcast,
}
