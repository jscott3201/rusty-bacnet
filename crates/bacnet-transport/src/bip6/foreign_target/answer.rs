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
//! - A registration still outstanding when its renewal goes is unanswered:
//!   B/IPv6 renews at half the time-to-live and retries on that schedule,
//!   whatever the answer. One an activation's immediate registration
//!   supersedes is unanswered only once the BBMD has been silent for
//!   [`ANSWER_TIMEOUT`] since the oldest registration the chain of
//!   supersedes left unanswered: a younger one may still be answered, so
//!   NO ANSWER doesn't flicker on, while activations faster than the
//!   renewal can't hide a silent BBMD for good.
//!
//! A registration is outstanding from just before it is sent, under the
//! lock the re-registration pass reads the target with, so an answer that
//! beats the send's return still finds it. The one it replaces counts as
//! unanswered only once the send has gone out; a send that fails puts that
//! one back. A move to another BBMD, the wait for a BBMD's host name and a
//! stop clear what the old BBMD said; the refusal total stays.

use std::net::SocketAddrV6;
use std::time::Duration;

use bacnet_bip_codec::ipv6::Bvlc6ResultCode;
use tokio::time::Instant;
use tracing::debug;

use super::release::RELEASE_TIMEOUT;
use super::{Bip6ForeignStatus, ForeignTarget, State};

/// How long the BBMD may stay silent before a registration an activation
/// supersedes counts as unanswered: the release's wait, B/IP's default
/// response timeout (3 s).
pub(super) const ANSWER_TIMEOUT: Duration = RELEASE_TIMEOUT;

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
    /// Its renewal went before the BBMD answered it, or an activation's
    /// registration replaced it with the BBMD silent for 3 s or more.
    Unanswered,
}

/// Why a registration goes out, which says what became of the one before
/// if that one is still outstanding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::bip6) enum Cause {
    /// The renewal, the retry of the one before: that one went unanswered.
    Renewal,
    /// A change registers at once, superseding the one before: an
    /// activation, a lookup that moved the BBMD, or a start.
    Change,
}

/// A registration awaiting its answer.
#[derive(Clone, Copy, Debug)]
struct Mark {
    /// The BBMD it went to.
    bbmd: SocketAddrV6,
    /// Since when the BBMD has been silent: when this registration went,
    /// or the oldest one it superseded unanswered went.
    since: Instant,
}

/// The registrations' answers, kept under the target's lock.
#[derive(Debug, Default)]
pub(super) struct Answers {
    /// The latest registration, while it waits for its answer.
    outstanding: Option<Mark>,
    /// The registration the outstanding one replaced, while the send of the
    /// outstanding one is out, and whether it went unanswered: counted once
    /// that send goes, and put back if it fails.
    replaced: Option<(Mark, bool)>,
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
            .is_some_and(|(sent, now)| same(sent.bbmd, now))
        {
            self.outstanding = None;
        }
        self.replaced = None;
        self.last = None;
        self.unanswered = 0;
    }

    /// Forget what this start's BBMD said: the transport stopped, or starts
    /// again.
    pub(super) fn reset(&mut self) {
        self.outstanding = None;
        self.replaced = None;
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
    /// A Register-Foreign-Device is about to go to `bbmd` at `now`, for
    /// `cause`: it is outstanding from here, before the send. The one
    /// before, if still outstanding, went unanswered when this is its
    /// renewal or the BBMD has been silent for [`ANSWER_TIMEOUT`], and was
    /// superseded otherwise, its silence carried on; either way it is
    /// settled when the send goes ([`ForeignTarget::registration_sent`]).
    /// A start's registration to a BBMD the device has since left starts
    /// nothing; one while the device still waits for its BBMD's name goes
    /// to the address the start found (#2365).
    pub(super) fn registration_going(&mut self, bbmd: SocketAddrV6, cause: Cause, now: Instant) {
        let left = self
            .config
            .bbmd
            .address()
            .is_some_and(|in_use| !same(in_use, bbmd));
        if left {
            return;
        }
        let answers = &mut self.answers;
        let previous = answers.outstanding;
        let unanswered = previous.is_some_and(|before| {
            cause == Cause::Renewal || now.saturating_duration_since(before.since) >= ANSWER_TIMEOUT
        });
        let since = match previous {
            Some(before) if !unanswered => before.since,
            _ => now,
        };
        answers.outstanding = Some(Mark { bbmd, since });
        answers.replaced = previous.map(|before| (before, unanswered));
    }

    /// `peer` sent a BVLC-Result with `code` at `now`: the answer to the
    /// registration outstanding, if it went to `peer` and `code` is a
    /// success or the Register-Foreign-Device NAK.
    fn registration_answered(&mut self, peer: SocketAddrV6, code: Bvlc6ResultCode, now: Instant) {
        let answers = &mut self.answers;
        if !answers
            .outstanding
            .is_some_and(|sent| same(sent.bbmd, peer))
        {
            return;
        }
        let answer = if code == Bvlc6ResultCode::SUCCESSFUL_COMPLETION {
            Bip6RegistrationAnswer::Held
        } else if code == Bvlc6ResultCode::REGISTER_FOREIGN_DEVICE_NAK {
            // The receive loop logs the NAK itself.
            answers.refused = answers.refused.saturating_add(1);
            Bip6RegistrationAnswer::Refused
        } else {
            return;
        };
        answers.outstanding = None;
        answers.replaced = None;
        answers.unanswered = 0;
        answers.last = Some((answer, now));
    }
}

impl ForeignTarget {
    /// A start's Register-Foreign-Device is about to go to `bbmd`.
    pub(in crate::bip6) fn registration_going(&self, bbmd: SocketAddrV6) {
        self.lock()
            .registration_going(bbmd, Cause::Change, Instant::now());
    }

    /// The Register-Foreign-Device about to go went out: the one it
    /// replaced, if that went unanswered, is counted now.
    pub(in crate::bip6) fn registration_sent(&self) {
        let mut state = self.lock();
        state.registrations_sent = state.registrations_sent.saturating_add(1);
        let answers = &mut state.answers;
        if let Some((before, true)) = answers.replaced.take() {
            answers.unanswered = answers.unanswered.saturating_add(1);
            answers.last = Some((Bip6RegistrationAnswer::Unanswered, Instant::now()));
            debug!(bbmd = %before.bbmd, "B/IPv6 BBMD left a foreign-device registration unanswered");
        }
    }

    /// The Register-Foreign-Device to `bbmd` couldn't be sent: it awaits no
    /// answer, and the one it replaced awaits its own again.
    pub(in crate::bip6) fn registration_not_sent(&self, bbmd: SocketAddrV6) {
        let answers = &mut self.lock().answers;
        let replaced = answers.replaced.take();
        if answers
            .outstanding
            .is_some_and(|sent| same(sent.bbmd, bbmd))
        {
            answers.outstanding = replaced.map(|(before, _)| before);
        }
    }

    /// Forget what the BBMD said, once the stopping transport's tasks have
    /// ended, so no pass in flight leaves a registration outstanding.
    pub(in crate::bip6) fn forget_answers(&self) {
        self.lock().answers.reset();
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
