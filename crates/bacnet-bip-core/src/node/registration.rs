//! A foreign device's registration lifecycle (Annex J.5.2): when to send
//! the next Register-Foreign-Device, and what the answers prove.
//!
//! The state machine knows times and outcomes only, no addresses or wire
//! types: the [`Node`](super::Node) sends the attempts and matches their
//! answers.
//!
//! Two things are kept apart: the newest attempt's outcome, and the lease,
//! which only a success proves. A pending attempt, a refusal or a timeout
//! changes the outcome but never erases a proven lease.
//!
//! The lease is conservative. A BBMD keeps a registration for its
//! time-to-live from when it took the request, plus a grace period (Annex
//! J.5.2.3). Locally, a success can't say which request it answers, so the
//! lease starts at the earliest send it could be for, and ends one
//! time-to-live later, without the grace period. Every send precedes its
//! arrival, so the local view never outlasts the BBMD's entry, on the
//! assumption [`Lease`] states.

use bacnet_bip_codec::BvlcResultCode;
use bacnet_time::{Duration, Instant};

use super::config::{Foreign, Retry};

/// A registration the BBMD has proven: from `since` until `until`.
///
/// A success names no request, so `since` is the earliest send it could
/// answer: any attempt sent since the last answer, and any sent less than
/// the answer window before the success, in case it is a late or repeated
/// copy. The answer window is three response timeouts, but no longer than
/// the renewal interval, so never shorter than one response timeout.
///
/// `until` is never later than the end of the BBMD's entry, grace period
/// left out, for every valid time-to-live, as long as every result, and
/// every copy of one, reaches the node less than the answer window after
/// its request was sent, and nothing else changes the entry (in foreign
/// mode the node refuses manual requests that could). A BBMD that loses its
/// table, as on a restart, ends the entry early; the
/// Distribute-Broadcast-To-Network NAK that follows is taken as a hint to
/// register again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Lease {
    /// The earliest send the proving success could answer.
    pub since: Instant,
    /// `since` plus the time-to-live; the lease is live before it.
    pub until: Instant,
}

impl Lease {
    /// Whether the lease is live at `now`.
    #[must_use]
    pub fn is_live_at(&self, now: Instant) -> bool {
        now < self.until
    }
}

/// What the newest registration attempt came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttemptOutcome {
    /// Admitted, and not answered yet.
    Pending,
    /// The BBMD answered with success.
    Accepted,
    /// The BBMD answered with the Register-Foreign-Device NAK.
    Refused,
    /// No answer by the response timeout.
    TimedOut,
    /// Encoding or sending it failed.
    LocalError,
    /// The caller gave it up.
    Cancelled,
}

/// A change in a foreign device's registration, reported once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegistrationEvent {
    /// A success while no lease was live: registered from `since` until
    /// `until`.
    Registered {
        /// The lease's start.
        since: Instant,
        /// The lease's end.
        until: Instant,
    },
    /// A success while a lease was live: it now ends at `until`.
    Renewed {
        /// The lease's new end.
        until: Instant,
    },
    /// The BBMD refused an attempt. A lease still live stays.
    Refused {
        /// The result code it refused with.
        code: BvlcResultCode,
    },
    /// An attempt ended without an answer: it timed out, or sending it
    /// failed.
    Unanswered,
    /// The lease ended with nothing newer proven.
    Lapsed,
}

/// A foreign device's registration as the node sees it at one instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RegistrationSnapshot {
    /// The newest proven lease, live or not.
    pub lease: Option<Lease>,
    /// Whether that lease is live.
    pub registered: bool,
    /// Its time left, while live.
    pub lease_remaining: Option<Duration>,
    /// Whether an automatic attempt is waiting for its answer.
    pub attempt_in_flight: bool,
    /// When the next automatic attempt is due; `None` while one is in
    /// flight.
    pub next_attempt: Option<Instant>,
    /// What the newest automatic attempt came to.
    pub last_outcome: Option<AttemptOutcome>,
    /// Automatic attempts in a row that ended unanswered or couldn't start.
    pub consecutive_failures: u32,
    /// Whether the first automatic attempt has ended, which ends the hold
    /// on broadcasts.
    pub first_resolved: bool,
    /// Broadcasts held for the first attempt.
    pub held: u16,
}

/// What a foreign device does with a broadcast it is asked to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// Send it now.
    Send,
    /// Keep it: the first registration attempt is outstanding. An
    /// [`Action::ReleaseHeld`](super::Action::ReleaseHeld) or
    /// [`Action::DiscardHeld`](super::Action::DiscardHeld) follows.
    Hold,
}

/// What [`Lifecycle::admit_broadcast`] decided, for counting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BroadcastDecision {
    Send { lapsed: bool },
    Hold,
    Full,
}

/// The fate of the broadcasts held for the first attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HeldVerdict {
    pub(crate) release: bool,
    pub(crate) count: u16,
}

/// Starts of recent sent attempts, newest last, in a small ring.
#[derive(Debug, Clone, Copy)]
struct Recent {
    starts: [Instant; Recent::LEN],
    len: u8,
    next: u8,
    /// The newest start pushed out of the ring.
    evicted: Option<Instant>,
}

impl Recent {
    const LEN: usize = 4;

    const fn new() -> Self {
        Self {
            starts: [Instant::ZERO; Self::LEN],
            len: 0,
            next: 0,
            evicted: None,
        }
    }

    fn push(&mut self, start: Instant) {
        let slot = &mut self.starts[usize::from(self.next)];
        if usize::from(self.len) == Self::LEN {
            self.evicted = Some(*slot);
        } else {
            self.len += 1;
        }
        *slot = start;
        self.next = (self.next + 1) & (Self::LEN as u8 - 1);
    }

    /// The earliest start after `floor`, or of all with no floor. When one
    /// that late may have left the ring, `floor` itself (or the epoch),
    /// which is earlier than it.
    fn earliest_after(&self, floor: Option<Instant>) -> Option<Instant> {
        let inside = |start: Instant| floor.is_none_or(|floor| start > floor);
        if self.evicted.is_some_and(inside) {
            return Some(floor.unwrap_or(Instant::ZERO));
        }
        self.starts[..usize::from(self.len)]
            .iter()
            .copied()
            .filter(|start| inside(*start))
            .min()
    }
}

/// The automatic attempt in flight.
#[derive(Debug, Clone, Copy)]
struct Attempt {
    /// When it was admitted, before its send.
    start: Instant,
}

/// The registration state machine.
#[derive(Debug, Clone)]
pub(crate) struct Lifecycle {
    ttl: Duration,
    renewal: Duration,
    timeout: Duration,
    retry: Retry,
    hold_limit: u16,
    lease: Option<Lease>,
    /// The lease's end was reported.
    lapsed: bool,
    /// The earliest start among attempts sent since the last answer.
    unanswered_since: Option<Instant>,
    recent: Recent,
    attempt: Option<Attempt>,
    last_start: Option<Instant>,
    last_outcome: Option<AttemptOutcome>,
    failures: u32,
    next: Option<Instant>,
    first_resolved: bool,
    held: u16,
}

impl Lifecycle {
    /// The lifecycle of `foreign`, its first attempt due at `now`.
    pub(crate) fn new(foreign: &Foreign, now: Instant) -> Self {
        Self {
            ttl: foreign.ttl_duration(),
            renewal: foreign.effective_renewal(),
            timeout: foreign.effective_response_timeout(),
            retry: foreign.retry,
            hold_limit: foreign.hold,
            lease: None,
            lapsed: false,
            unanswered_since: None,
            recent: Recent::new(),
            attempt: None,
            last_start: None,
            last_outcome: None,
            failures: 0,
            next: Some(now),
            first_resolved: false,
            held: 0,
        }
    }

    /// How long after its request a success, or a copy of it, may still
    /// arrive: three response timeouts, but no longer than the renewal
    /// interval. The response timeout is never longer than the renewal, so
    /// this is at least one response timeout. Any attempt sent less than
    /// this before a success could be the one it answers, so the lease
    /// starts no later than the earliest of them. An attempt one renewal
    /// back is never counted, so a renewal doesn't pull its lease back to
    /// its predecessor's.
    pub(crate) fn answer_window(&self) -> Duration {
        let three = self.timeout.saturating_mul(2).saturating_add(self.timeout);
        three.min(self.renewal)
    }

    /// When the next attempt is due, while none is in flight.
    pub(crate) fn due_at(&self) -> Option<Instant> {
        self.next.filter(|_| self.attempt.is_none())
    }

    /// When the lease ends, unless that was already reported.
    pub(crate) fn lapse_at(&self) -> Option<Instant> {
        self.lease.filter(|_| !self.lapsed).map(|lease| lease.until)
    }

    /// The lease's end has come: report it once.
    pub(crate) fn lapse(&mut self) {
        self.lapsed = true;
    }

    /// An attempt was admitted at `now`.
    pub(crate) fn admitted(&mut self, now: Instant) {
        self.attempt = Some(Attempt { start: now });
        self.next = None;
        self.last_start = Some(now);
        self.last_outcome = Some(AttemptOutcome::Pending);
    }

    /// The attempt in flight was sent.
    pub(crate) fn sent(&mut self) {
        if let Some(attempt) = self.attempt {
            self.unanswered_since.get_or_insert(attempt.start);
            self.recent.push(attempt.start);
        }
    }

    /// The attempt in flight was answered with success at `now`. The event
    /// to report, unless the proven lease has already ended.
    pub(crate) fn accepted(
        &mut self,
        now: Instant,
    ) -> (Option<RegistrationEvent>, Option<HeldVerdict>) {
        let start = self.attempt.take().map_or(now, |attempt| attempt.start);
        // Sent less than the window before now: after the floor, or any
        // when the window reaches back past the epoch.
        let floor = now.checked_sub(self.answer_window());
        let since = [self.unanswered_since, self.recent.earliest_after(floor)]
            .into_iter()
            .flatten()
            .fold(start, Instant::min);
        let until = since + self.ttl;
        let was_live = self.lease.is_some_and(|lease| lease.is_live_at(now));
        self.lease = Some(Lease { since, until });
        self.lapsed = until <= now;
        self.unanswered_since = None;
        self.failures = 0;
        // Renew one interval into the lease, which may already be due; at
        // renewal, as today, one interval after this attempt.
        self.next = Some(
            match self.retry {
                Retry::Backoff => since,
                Retry::AtRenewal => start,
            } + self.renewal,
        );
        self.last_outcome = Some(AttemptOutcome::Accepted);
        let event = match (self.lapsed, was_live) {
            (true, _) => None,
            (false, true) => Some(RegistrationEvent::Renewed { until }),
            (false, false) => Some(RegistrationEvent::Registered { since, until }),
        };
        (event, self.resolve_first(true))
    }

    /// The attempt in flight was refused at `now`. The lease stays, and the
    /// next attempt comes one renewal interval after this one started.
    pub(crate) fn refused(&mut self, now: Instant) -> Option<HeldVerdict> {
        let start = self.attempt.take().map_or(now, |attempt| attempt.start);
        self.unanswered_since = None;
        self.failures = 0;
        self.next = Some(start + self.renewal);
        self.last_outcome = Some(AttemptOutcome::Refused);
        self.resolve_first(false)
    }

    /// An attempt ended unanswered at `now`, or (with no attempt in flight)
    /// couldn't start because its exchange was busy. `outcome` is `None`
    /// for the latter, which leaves the newest outcome as it was.
    pub(crate) fn failed(
        &mut self,
        now: Instant,
        outcome: Option<AttemptOutcome>,
    ) -> Option<HeldVerdict> {
        let start = self.attempt.take().map_or(now, |attempt| attempt.start);
        self.failures = self.failures.saturating_add(1);
        let at_renewal = start + self.renewal;
        self.next = Some(match self.retry {
            Retry::AtRenewal => at_renewal,
            Retry::Backoff => at_renewal.min(now + self.backoff()),
        });
        if outcome.is_some() {
            self.last_outcome = outcome;
        }
        self.resolve_first(true)
    }

    /// One response timeout, doubled for each failure in a row after the
    /// first, and no longer than the renewal interval.
    fn backoff(&self) -> Duration {
        let mut wait = self.timeout;
        for _ in 1..self.failures {
            if wait >= self.renewal {
                break;
            }
            wait = wait.saturating_mul(2);
        }
        wait.min(self.renewal)
    }

    /// The BBMD refused a broadcast as from an unregistered device (Annex
    /// J.4.5): register again soon, but no sooner than one response
    /// timeout after the last attempt started. Nothing changes while an
    /// attempt is in flight.
    pub(crate) fn hint(&mut self, now: Instant) {
        if self.attempt.is_some() {
            return;
        }
        let earliest = self
            .last_start
            .map_or(now, |start| (start + self.timeout).max(now));
        if self.next.is_some_and(|next| next > earliest) {
            self.next = Some(earliest);
        }
    }

    /// Whether to send, hold or refuse a broadcast at `now`.
    pub(crate) fn admit_broadcast(&mut self, now: Instant) -> BroadcastDecision {
        if !self.first_resolved && self.hold_limit > 0 {
            if self.held >= self.hold_limit {
                return BroadcastDecision::Full;
            }
            self.held += 1;
            return BroadcastDecision::Hold;
        }
        let lapsed = !self.lease.is_some_and(|lease| lease.is_live_at(now));
        BroadcastDecision::Send { lapsed }
    }

    /// The first attempt has ended: what becomes of the held broadcasts.
    fn resolve_first(&mut self, release: bool) -> Option<HeldVerdict> {
        if self.first_resolved {
            return None;
        }
        self.first_resolved = true;
        let count = core::mem::take(&mut self.held);
        (count > 0).then_some(HeldVerdict { release, count })
    }

    pub(crate) fn snapshot(&self, now: Instant) -> RegistrationSnapshot {
        let registered = self.lease.is_some_and(|lease| lease.is_live_at(now));
        RegistrationSnapshot {
            lease: self.lease,
            registered,
            lease_remaining: self
                .lease
                .filter(|_| registered)
                .map(|lease| lease.until - now),
            attempt_in_flight: self.attempt.is_some(),
            next_attempt: self.due_at(),
            last_outcome: self.last_outcome,
            consecutive_failures: self.failures,
            first_resolved: self.first_resolved,
            held: self.held,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Instant, Recent};

    fn at(secs: u64) -> Instant {
        Instant::from_secs(secs)
    }

    #[test]
    fn the_ring_answers_with_the_earliest_start_after_the_floor() {
        let mut recent = Recent::new();
        assert_eq!(recent.earliest_after(None), None);
        for start in [10, 20, 30] {
            recent.push(at(start));
        }
        assert_eq!(recent.earliest_after(None), Some(at(10)));
        assert_eq!(recent.earliest_after(Some(at(10))), Some(at(20)));
        assert_eq!(recent.earliest_after(Some(at(9))), Some(at(10)));
        assert_eq!(recent.earliest_after(Some(at(30))), None);
    }

    #[test]
    fn a_start_pushed_out_of_the_ring_falls_back_to_the_floor() {
        let mut recent = Recent::new();
        for start in [10, 20, 30, 40, 50] {
            recent.push(at(start));
        }
        // 10 left the ring. Inside the window, the floor stands in for it.
        assert_eq!(recent.earliest_after(Some(at(5))), Some(at(5)));
        assert_eq!(recent.earliest_after(None), Some(Instant::ZERO));
        // Outside it, the ring answers alone.
        assert_eq!(recent.earliest_after(Some(at(10))), Some(at(20)));
        assert_eq!(recent.earliest_after(Some(at(25))), Some(at(30)));
        recent.push(at(60));
        assert_eq!(recent.earliest_after(Some(at(15))), Some(at(15)));
        assert_eq!(recent.earliest_after(Some(at(20))), Some(at(30)));
    }
}
