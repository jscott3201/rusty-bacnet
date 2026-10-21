//! [`NpduLimits`]: the longest NPDU a link carries.

use crate::MAX_NPDU;

/// The longest NPDU a link carries, in octets, to one node and to every
/// node on it (Table 6-1).
///
/// A transport reports it for its link, and a router holds one for each of
/// its ports and refuses what doesn't fit the port a message leaves by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NpduLimits {
    /// To one node.
    pub unicast: u16,
    /// To every node on the link.
    pub broadcast: u16,
}

impl NpduLimits {
    /// [`MAX_NPDU`] each way: what B/IP, B/IPv6, Ethernet and MS/TP with
    /// extended frames carry (Table 6-1).
    pub const STANDARD: Self = Self::both(MAX_NPDU as u16);

    /// The same limit both ways.
    #[must_use]
    pub const fn both(octets: u16) -> Self {
        Self {
            unicast: octets,
            broadcast: octets,
        }
    }

    /// The limit for a broadcast, or for a unicast.
    #[must_use]
    pub const fn get(self, broadcast: bool) -> usize {
        if broadcast {
            self.broadcast as usize
        } else {
            self.unicast as usize
        }
    }
}

/// [`NpduLimits::STANDARD`].
impl Default for NpduLimits {
    fn default() -> Self {
        Self::STANDARD
    }
}
