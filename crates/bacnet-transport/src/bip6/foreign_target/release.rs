//! The release of the BBMD a B/IPv6 foreign device left (#2338): with
//! [`Bip6Transport::set_release_previous_bbmd`](crate::bip6::Bip6Transport::set_release_previous_bbmd)
//! on, a move to another BBMD also asks the one left to delete this
//! device's entry, rather than let it run out with its time-to-live and
//! grace period (Annex U.4.5.4, U.4.5.5). It is B/IP's release (#2307) in
//! shape: one Delete-Foreign-Device-Table-Entry naming this device's own
//! B/IPv6 address, sent after the first registration with the new BBMD and
//! given up after 3 seconds, best effort, counted, and never holding up or
//! failing the move.
//!
//! B/IP runs it in `bacnet-bip-core`'s node engine. B/IPv6's registration
//! doesn't run on that node, which speaks B/IP's IPv4 BVLL: this
//! transport's own re-registration task sends each registration, and the
//! receive loop records its answer ([`super::answer`], #2388). So the
//! release lives here, beside the moves that call for it.
//!
//! One release is queued at a time, and one awaits its answer: a move
//! while one is still queued drops it unsent, and a release sent while an
//! earlier one waits ends that wait. Either counts as unanswered, as does
//! one still out when the transport stops. A BVLC-Result names no request,
//! so any success from the old BBMD while the release waits counts as its
//! answer, as in B/IP's node engine, ahead of any registration's: a late
//! answer to a renewal sent just before the move, or to the registration
//! after a move back there, can pass for one.

use std::net::SocketAddrV6;
use std::time::Duration;

use bacnet_bip_codec::ipv6::{Bip6Address, Bvlc6ResultCode};
use tokio::time::Instant;
use tracing::{debug, info};

use super::super::port::Bip6ForeignDeviceConfig;
use super::{ForeignTarget, State};

/// What came of a B/IPv6 foreign device's releases of the BBMDs it moved
/// away from (#2338), asked for by
/// [`Bip6Transport::set_release_previous_bbmd`](crate::bip6::Bip6Transport::set_release_previous_bbmd).
/// Each release ends once, as released, refused or unanswered; one still
/// queued or awaiting its answer is in `releases` only. The totals survive
/// a restart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Bip6ReleaseCounters {
    /// Releases asked for: one for each move to another BBMD while the
    /// option was on.
    pub releases: u64,
    /// Of those, the ones the old BBMD answered with success. A success
    /// names no request, so a late answer to a renewal sent to it just
    /// before the move can be counted here too.
    pub released: u64,
    /// Of those, the ones it refused with a Delete-Foreign-Device-Table-Entry
    /// NAK. Its entry then stays until its time-to-live runs out.
    pub refused: u64,
    /// Of those, the ones not answered within 3 seconds, not sent (dropped
    /// by a later move, or a local send failure), or still out when the
    /// transport stopped. The old entry then stays until its time-to-live
    /// runs out, unless the release took effect unseen.
    pub unanswered: u64,
}

/// How long a release's answer is waited for: B/IP's default response
/// timeout (`bacnet_bip_core::node::DEFAULT_RESPONSE_TIMEOUT`).
const RELEASE_TIMEOUT: Duration = Duration::from_secs(3);

/// The release in flight: the BBMD asked and when its answer is due.
#[derive(Clone, Copy, Debug)]
pub(super) struct Awaiting {
    bbmd: SocketAddrV6,
    due: Instant,
}

impl State {
    /// Make `config` the registration in effect. A move to another BBMD
    /// drops a release still queued, and, with the release on, queues one
    /// of the BBMD left. A device that waited for its BBMD's name left
    /// none (#2365). What the BBMD left said goes with it (#2388).
    pub(super) fn take(&mut self, config: Bip6ForeignDeviceConfig) {
        let left = self.config.bbmd.address();
        self.config = config;
        if left == self.config.bbmd.address() {
            return;
        }
        self.answers.moved(self.config.bbmd.address());
        if let Some(dropped) = self.queued.take() {
            self.unanswered(dropped, "dropped unsent by a later move");
        }
        if let (true, Some(left)) = (self.release_previous, left) {
            self.releases.releases = self.releases.releases.saturating_add(1);
            self.queued = Some(left);
        }
    }

    /// The transport stopped: the releases still out end unanswered.
    pub(super) fn end_releases(&mut self) {
        if let Some(bbmd) = self.queued.take() {
            self.unanswered(bbmd, "dropped unsent as the transport stopped");
        }
        if let Some(awaiting) = self.awaiting.take() {
            self.unanswered(awaiting.bbmd, "unconfirmed as the transport stopped");
        }
    }

    fn unanswered(&mut self, bbmd: SocketAddrV6, why: &str) {
        self.releases.unanswered = self.releases.unanswered.saturating_add(1);
        info!(%bbmd, why, "The B/IPv6 BBMD this foreign device left did not confirm deleting its entry; it lapses on its own");
    }
}

impl ForeignTarget {
    /// Release the BBMD a move leaves from now on, or not.
    pub(in crate::bip6) fn set_release_previous(&self, release: bool) {
        self.lock().release_previous = release;
    }

    /// What came of the releases so far.
    pub(in crate::bip6) fn release_counters(&self) -> Bip6ReleaseCounters {
        self.lock().releases
    }

    /// For the re-registration task's pass at `now`: the registration in
    /// effect, and the release queued, if any, as the BBMD to ask and the
    /// entry to name, now awaiting its answer. Read in one lock, so the
    /// release never goes to the BBMD the registration does.
    pub(in crate::bip6) fn pass(
        &self,
        now: Instant,
    ) -> (Bip6ForeignDeviceConfig, Option<(SocketAddrV6, Bip6Address)>) {
        let mut state = self.lock();
        let config = state.config.clone();
        let Some(bbmd) = state.queued.take() else {
            return (config, None);
        };
        // Stopping: the release ends with the transport.
        let Some(own) = state.own else {
            state.unanswered(bbmd, "dropped unsent as the transport stopped");
            return (config, None);
        };
        let due = now + RELEASE_TIMEOUT;
        if let Some(earlier) = state.awaiting.replace(Awaiting { bbmd, due }) {
            state.unanswered(earlier.bbmd, "no answer before the next release");
        }
        (
            config,
            Some((bbmd, Bip6Address::new(*own.ip(), own.port()))),
        )
    }

    /// The release to `bbmd` couldn't be sent.
    pub(in crate::bip6) fn release_not_sent(&self, bbmd: SocketAddrV6) {
        let mut state = self.lock();
        if state.awaiting.is_some_and(|awaiting| awaiting.bbmd == bbmd) {
            state.awaiting = None;
            state.unanswered(bbmd, "not sent");
        }
    }

    /// When the release in flight is given up, if one is.
    pub(in crate::bip6) fn release_due(&self) -> Option<Instant> {
        self.lock().awaiting.map(|awaiting| awaiting.due)
    }

    /// Give the release in flight up if its answer was due by `now`.
    pub(in crate::bip6) fn release_expired(&self, now: Instant) {
        let mut state = self.lock();
        if let Some(awaiting) = state.awaiting.filter(|awaiting| awaiting.due <= now) {
            state.awaiting = None;
            state.unanswered(awaiting.bbmd, "no answer within 3 s");
        }
    }
}

impl State {
    /// `peer` sent a BVLC-Result with `code`: the answer to the release in
    /// flight, if `peer` is its BBMD and `code` a success or the
    /// Delete-Foreign-Device-Table-Entry NAK (U.4.5.4). Whether it was.
    pub(super) fn release_answered(&mut self, peer: SocketAddrV6, code: Bvlc6ResultCode) -> bool {
        let Some(awaiting) = self.awaiting else {
            return false;
        };
        let bbmd = awaiting.bbmd;
        if *peer.ip() != *bbmd.ip() || peer.port() != bbmd.port() {
            return false;
        }
        if code == Bvlc6ResultCode::SUCCESSFUL_COMPLETION {
            self.awaiting = None;
            self.releases.released = self.releases.released.saturating_add(1);
            debug!(%bbmd, "The B/IPv6 BBMD this foreign device left deleted its entry");
        } else if code == Bvlc6ResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK {
            self.awaiting = None;
            self.releases.refused = self.releases.refused.saturating_add(1);
            info!(%bbmd, "The B/IPv6 BBMD this foreign device left refused to delete its entry; it lapses on its own");
        } else {
            return false;
        }
        true
    }
}
