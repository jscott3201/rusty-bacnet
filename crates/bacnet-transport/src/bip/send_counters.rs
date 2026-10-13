//! What the B/IP send path refuses, counted by reason (#1622).

use std::sync::atomic::{AtomicU64, Ordering};

use bacnet_bip_core::node::SendRefusal;

/// NPDUs a B/IP transport refused to send since it was created, by reason.
/// Each refusal also fails the send with an error. The totals survive stop
/// and restart.
///
/// It is `#[non_exhaustive]`, so a later counter isn't a breaking change:
/// read its fields, or start from `Default` in a test.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SendCounters {
    /// NPDUs longer than a B/IP data link carries, 1497 octets (Table 6-1):
    /// a unicast or a broadcast, the application's own or one a router
    /// forwards onto this port. A BBMD passing on traffic it received
    /// forwards it as it came, unchecked and uncounted here.
    pub npdu_too_long: u64,
}

/// The send path's refusal counts, which the transport keeps across
/// restarts and lends to each start's node.
#[derive(Debug, Default)]
pub(super) struct SendRefusals {
    npdu_too_long: AtomicU64,
}

impl SendRefusals {
    /// Count `refusal`, if a counter keeps it. A full hold is counted with
    /// the foreign device's registration instead
    /// ([`ForeignRegistrationSnapshot::broadcasts_hold_full`](super::ForeignRegistrationSnapshot::broadcasts_hold_full)).
    pub(super) fn count(&self, refusal: SendRefusal) {
        if let SendRefusal::TooLong { .. } = refusal {
            self.npdu_too_long.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The counts so far.
    pub(super) fn snapshot(&self) -> SendCounters {
        SendCounters {
            npdu_too_long: self.npdu_too_long.load(Ordering::Relaxed),
        }
    }
}
