//! What the B/IPv6 transport's send side did to find its peers (U.5): the
//! resolutions it ran, the requests it sent for them, and the sends it
//! refused or dropped while they ran. The receive path keeps its own counts
//! ([`Bip6ReceiveCounters`](super::Bip6ReceiveCounters)), which are about
//! datagrams received.

use std::sync::atomic::{AtomicU64, Ordering};

/// The send side's resolution counts since the transport was created. The
/// totals survive stop and restart. Each field is read on its own, so a
/// snapshot taken while sends run need not add up across fields.
///
/// A resolution that ends counts once: resolved, timed out or refused. A
/// check that a quiet peer is still there (a send starts one after five
/// minutes of silence from the peer) is not a resolution: its queries count
/// in `revalidations_sent` and its failure in `revalidations_failed`. A send
/// that joins a check, because the peer's entry went meanwhile, makes it a
/// resolution too, counted as one.
///
/// It is `#[non_exhaustive]`, so a later counter isn't a breaking change:
/// read its fields, or start from `Default` in a test.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Bip6ResolutionCounters {
    /// Resolutions started: sends to a VMAC the table had no address for.
    pub resolutions_started: u64,
    /// Resolutions completed by learning the VMAC's address.
    pub resolutions_resolved: u64,
    /// Resolutions that nothing answered in time.
    pub resolutions_timed_out: u64,
    /// Resolutions that failed after the configured BBMD refused to pass an
    /// Address-Resolution on (Address-Resolution NAK), as for a foreign
    /// device it has no registration for.
    pub resolutions_refused: u64,
    /// Address-Resolution messages sent, first attempts and retries.
    pub address_resolutions_sent: u64,
    /// Sends refused at once because their VMAC failed to resolve a moment
    /// ago.
    pub sends_unreachable: u64,
    /// Sends refused because a bound on resolutions, waiting sends or
    /// queued NPDUs was reached.
    pub sends_over_limit: u64,
    /// Queued NPDUs dropped because their VMAC did not resolve, or because
    /// the transport stopped first.
    pub queued_dropped: u64,
    /// Virtual-Address-Resolution messages sent.
    pub virtual_resolutions_sent: u64,
    /// Virtual-Address-Resolutions answered.
    pub virtual_resolved: u64,
    /// Virtual-Address-Resolutions nothing answered in time.
    pub virtual_timed_out: u64,
    /// Virtual-Address-Resolutions the peer refused.
    pub virtual_refused: u64,
    /// Virtual-Address-Resolutions sent to a quiet peer's address on file,
    /// to check it is still there.
    pub revalidations_sent: u64,
    /// Peers removed because a check that they were still there went
    /// unanswered.
    pub revalidations_failed: u64,
}

/// The counts behind [`Bip6ResolutionCounters`].
#[derive(Debug, Default)]
pub(super) struct ResolutionCounts {
    resolutions_started: AtomicU64,
    resolutions_resolved: AtomicU64,
    resolutions_timed_out: AtomicU64,
    resolutions_refused: AtomicU64,
    address_resolutions_sent: AtomicU64,
    sends_unreachable: AtomicU64,
    sends_over_limit: AtomicU64,
    queued_dropped: AtomicU64,
    virtual_resolutions_sent: AtomicU64,
    virtual_resolved: AtomicU64,
    virtual_timed_out: AtomicU64,
    virtual_refused: AtomicU64,
    revalidations_sent: AtomicU64,
    revalidations_failed: AtomicU64,
}

fn add(counter: &AtomicU64, n: u64) {
    counter.fetch_add(n, Ordering::Relaxed);
}

impl ResolutionCounts {
    pub(super) fn started(&self) {
        add(&self.resolutions_started, 1);
    }
    pub(super) fn resolved(&self) {
        add(&self.resolutions_resolved, 1);
    }
    pub(super) fn timed_out(&self) {
        add(&self.resolutions_timed_out, 1);
    }
    pub(super) fn refused(&self) {
        add(&self.resolutions_refused, 1);
    }
    pub(super) fn address_resolution_sent(&self) {
        add(&self.address_resolutions_sent, 1);
    }
    pub(super) fn unreachable(&self) {
        add(&self.sends_unreachable, 1);
    }
    pub(super) fn over_limit(&self) {
        add(&self.sends_over_limit, 1);
    }
    pub(super) fn queued_dropped(&self, n: usize) {
        add(&self.queued_dropped, n as u64);
    }
    pub(super) fn virtual_sent(&self) {
        add(&self.virtual_resolutions_sent, 1);
    }
    pub(super) fn virtual_resolved(&self) {
        add(&self.virtual_resolved, 1);
    }
    pub(super) fn virtual_timed_out(&self) {
        add(&self.virtual_timed_out, 1);
    }
    pub(super) fn virtual_refused(&self) {
        add(&self.virtual_refused, 1);
    }
    pub(super) fn revalidation_sent(&self) {
        add(&self.revalidations_sent, 1);
    }
    pub(super) fn revalidation_failed(&self) {
        add(&self.revalidations_failed, 1);
    }

    /// The counts so far.
    pub(super) fn snapshot(&self) -> Bip6ResolutionCounters {
        let get = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        Bip6ResolutionCounters {
            resolutions_started: get(&self.resolutions_started),
            resolutions_resolved: get(&self.resolutions_resolved),
            resolutions_timed_out: get(&self.resolutions_timed_out),
            resolutions_refused: get(&self.resolutions_refused),
            address_resolutions_sent: get(&self.address_resolutions_sent),
            sends_unreachable: get(&self.sends_unreachable),
            sends_over_limit: get(&self.sends_over_limit),
            queued_dropped: get(&self.queued_dropped),
            virtual_resolutions_sent: get(&self.virtual_resolutions_sent),
            virtual_resolved: get(&self.virtual_resolved),
            virtual_timed_out: get(&self.virtual_timed_out),
            virtual_refused: get(&self.virtual_refused),
            revalidations_sent: get(&self.revalidations_sent),
            revalidations_failed: get(&self.revalidations_failed),
        }
    }
}
