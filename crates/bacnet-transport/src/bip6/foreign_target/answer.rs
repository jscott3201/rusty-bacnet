//! The BBMD's answers to a B/IPv6 foreign device's registrations (#2388).
//!
//! A BBMD answers each Register-Foreign-Device with a BVLC-Result: success
//! once its foreign device table has the entry, or the
//! Register-Foreign-Device NAK when it won't take it (Annex U.4.5.3, the
//! codes in U.2.1.1). A result carries no request identifier, so one counts
//! as a registration's answer only while that registration is outstanding:
//! it came from the BBMD in use, after a registration went there and before
//! the next one did.
//!
//! - The NAK's code names the request it refuses, so it is unambiguous.
//! - A success could also answer a Delete-Foreign-Device-Table-Entry
//!   (U.4.5.4), the only other request this node sends a BBMD that success
//!   answers: Distribute-Broadcast-To-Network and Address-Resolution draw a
//!   result only to refuse them, and Virtual-Address-Resolution draws an
//!   ACK. So the release in flight (#2338) takes a success from its BBMD
//!   first. A late success to an earlier registration of the same BBMD can
//!   still pass for the outstanding one's, as on B/IP.
//! - A registration still outstanding when the next one goes is
//!   unanswered: B/IPv6 renews at half the time-to-live and retries on that
//!   schedule, whatever the answer.
//!
//! A move to another BBMD, the wait for a BBMD's host name and a stop clear
//! what the old BBMD said; the refusal total stays.

use std::net::SocketAddrV6;

use bacnet_bip_codec::ipv6::Bvlc6ResultCode;
use tokio::time::Instant;
use tracing::{debug, info};

use super::{Bip6ForeignStatus, ForeignTarget, State};

/// What the BBMD in use made of this device's latest registration there
/// (#2388), as
/// [`Bip6ForeignStatus::last_answer`](crate::bip6::Bip6ForeignStatus::last_answer)
/// reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Bip6RegistrationAnswer {
    /// The BBMD answered with success: it holds this device's entry.
    Held,
    /// The BBMD answered with the Register-Foreign-Device NAK: it won't
    /// take this device. Registrations go on at the usual renewal.
    Refused,
    /// The next registration went before the BBMD answered this one.
    Unanswered,
}

/// The registrations' answers, kept under the target's lock.
#[derive(Debug, Default)]
pub(super) struct Answers {
    /// The BBMD the latest registration went to, while it waits for its
    /// answer.
    outstanding: Option<SocketAddrV6>,
    /// The BBMD's latest answer, and when it was recorded.
    last: Option<(Bip6RegistrationAnswer, Instant)>,
    /// Registrations to the BBMD in use that went unanswered in a row.
    unanswered: u64,
    /// Register-Foreign-Device NAKs taken as answers, from every BBMD.
    refused: u64,
}

/// Whether `a` and `b` are one B/IPv6 address. Neither a zone index nor a
/// flow label is part of one.
fn same(a: SocketAddrV6, b: SocketAddrV6) -> bool {
    a.ip() == b.ip() && a.port() == b.port()
}

impl Answers {
    /// The BBMD in use is `bbmd` now: a registration to another one is no
    /// longer outstanding, and what the BBMD before said is gone.
    pub(super) fn moved(&mut self, bbmd: Option<SocketAddrV6>) {
        if !self
            .outstanding
            .zip(bbmd)
            .is_some_and(|(sent, now)| same(sent, now))
        {
            self.outstanding = None;
        }
        self.last = None;
        self.unanswered = 0;
    }

    /// Forget what this start's BBMD said: the transport stopped, or starts
    /// again.
    pub(super) fn reset(&mut self) {
        self.outstanding = None;
        self.last = None;
        self.unanswered = 0;
    }

    /// Report the latest answer, how long before `now` it was recorded,
    /// the registrations unanswered in a row and the refusals in `status`.
    pub(super) fn fill(&self, status: &mut Bip6ForeignStatus, now: Instant) {
        status.last_answer = self.last.map(|(answer, _)| answer);
        status.last_answer_age = self.last.map(|(_, at)| now.saturating_duration_since(at));
        status.unanswered = self.unanswered;
        status.registrations_refused = self.refused;
    }
}

impl State {
    /// A Register-Foreign-Device went to `bbmd` at `now`. The one before,
    /// if still outstanding, went unanswered. A pass that read a BBMD the
    /// device has since left starts nothing: the move's own registration
    /// follows at once. One sent while the device still waits for its
    /// BBMD's name is the start's, to the address it found (#2365).
    fn registration_sent(&mut self, bbmd: SocketAddrV6, now: Instant) {
        let left = self
            .config
            .bbmd
            .address()
            .is_some_and(|in_use| !same(in_use, bbmd));
        if left {
            return;
        }
        let answers = &mut self.answers;
        if answers.outstanding.replace(bbmd).is_some() {
            answers.unanswered = answers.unanswered.saturating_add(1);
            answers.last = Some((Bip6RegistrationAnswer::Unanswered, now));
            debug!(%bbmd, "B/IPv6 BBMD left a foreign-device registration unanswered");
        }
    }

    /// `peer` sent a BVLC-Result with `code` at `now`: the answer to the
    /// registration outstanding, if it went to `peer` and `code` is a
    /// success or the Register-Foreign-Device NAK.
    fn registration_answered(&mut self, peer: SocketAddrV6, code: Bvlc6ResultCode, now: Instant) {
        let answers = &mut self.answers;
        let Some(bbmd) = answers.outstanding.filter(|sent| same(*sent, peer)) else {
            return;
        };
        let answer = if code == Bvlc6ResultCode::SUCCESSFUL_COMPLETION {
            Bip6RegistrationAnswer::Held
        } else if code == Bvlc6ResultCode::REGISTER_FOREIGN_DEVICE_NAK {
            answers.refused = answers.refused.saturating_add(1);
            info!(%bbmd, "The B/IPv6 BBMD refused this foreign device's registration; it is sent again at the next renewal");
            Bip6RegistrationAnswer::Refused
        } else {
            return;
        };
        answers.outstanding = None;
        answers.unanswered = 0;
        answers.last = Some((answer, now));
    }
}

impl ForeignTarget {
    /// A Register-Foreign-Device went to `bbmd`.
    pub(in crate::bip6) fn registration_sent(&self, bbmd: SocketAddrV6) {
        let mut state = self.lock();
        state.registrations_sent = state.registrations_sent.saturating_add(1);
        state.registration_sent(bbmd, Instant::now());
    }

    /// `peer` sent a BVLC-Result with `code`: the answer to the release in
    /// flight (#2338) if it takes it, else perhaps to the registration
    /// outstanding.
    pub(in crate::bip6) fn result(&self, peer: SocketAddrV6, code: Bvlc6ResultCode) {
        let mut state = self.lock();
        if !state.release_answered(peer, code) {
            state.registration_answered(peer, code, Instant::now());
        }
    }
}
