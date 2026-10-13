//! Recognizing this station's own echo among the octets it reads, on a port
//! that declares [`Echo::Full`](super::Echo::Full).
//!
//! After each frame the filter hunts for the frame's octets in what is read.
//! Octets that could begin the echo are held back; once the whole frame has
//! been matched they are dropped, as the echo. An octet that does not fit
//! makes the filter pass on its oldest held octets, as few as it can, until
//! what it still holds plus the new octet could again begin the echo; when
//! nothing does, the new octet passes on too. So a stray octet before the
//! echo (a peer's trailing pad octet, say) passes on and the echo behind it
//! is still dropped, while a peer's frame that only starts the way the echo
//! does (every frame starts with the same preamble) passes on whole. An
//! octet is dropped only as part of a complete copy of the frame just sent.
//!
//! Passed-on octets reach the core stamped with the time of the read that
//! released them, which keeps stamps monotonic even when a deadline ran in
//! between, and is late only in the safe direction.
//!
//! The hunt runs in phases:
//!
//! - **Grace.** From the frame's end until the grace runs out. The frame is
//!   reported complete at the first octet that passes on, or at the end of
//!   the grace, and the hunt goes on.
//! - **Residual.** After a grace that ended with part of the echo held: the
//!   rest of the echo is still hunted for and dropped as it arrives.
//! - **Probe.** After a grace with nothing held: one more grace in which a
//!   late echo is still dropped; at its end what is held passes on.
//!
//! Silence ends the residual hunt: more than T_frame_abort since the last
//! octet read (the probe has its own deadline instead). Held octets that
//! have waited longer than T_frame_abort for the octet that would release
//! them are dropped, as a stale fragment of the echo: a receiver would have
//! abandoned that fragment anyway, and having matched the frame, it cannot
//! be a whole frame of a peer's. Before this station tries to start a
//! frame, octets held in probe mode pass on (they may be a peer's); a
//! confirmed start then ends the hunt, dropping what the grace or residual
//! phases still hold as the echo's continuation.

use tokio::time::{Duration, Instant};

/// What to do after one octet read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Pushed {
    /// Held octets dropped as a stale fragment of the echo, before this
    /// octet was looked at.
    pub(super) stale: usize,
    /// How many of the oldest held octets to pass on, in order
    /// ([`EchoFilter::held`]).
    pub(super) release: usize,
    /// Whether the octet itself passes on, after them.
    pub(super) pass: bool,
    /// The octet completed the echo, which is dropped.
    pub(super) echo: bool,
    /// The first octets of this hunt to pass on while the echo was expected
    /// (not in probe mode): the echo differed from what was sent, or other
    /// octets came with it.
    pub(super) first_mismatch: bool,
}

impl Pushed {
    /// Whether anything passes on to the core.
    pub(super) fn passes(&self) -> bool {
        self.release > 0 || self.pass
    }
}

/// What a timer or a new start does to the hunt.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Ended {
    /// Held octets dropped as stale.
    pub(super) stale: usize,
    /// Held octets to pass on.
    pub(super) release: usize,
    /// The grace ended with none of the echo held: count it missing.
    pub(super) missing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Off,
    /// Within the grace, which ends at the instant held.
    Grace(Instant),
    /// After the grace, part of the echo held.
    Residual,
    /// After a grace with none of the echo held, until the instant held.
    Probe(Instant),
}

/// The echo filter for the frame this station sent last.
#[derive(Debug)]
pub(super) struct EchoFilter {
    /// The frame's octets.
    expected: Vec<u8>,
    /// The held octets are the first `matched` of `expected`.
    matched: usize,
    phase: Phase,
    grace: Duration,
    frame_abort: Duration,
    /// When the last octet was read, or the hunt began. While octets are
    /// held, the newest of them was read then.
    last_read: Instant,
    /// Octets passed on in this hunt while the echo was expected.
    mismatched: bool,
}

impl EchoFilter {
    pub(super) fn new(frame_abort: Duration) -> Self {
        let now = Instant::now();
        Self {
            expected: Vec::new(),
            matched: 0,
            phase: Phase::Off,
            grace: Duration::ZERO,
            frame_abort,
            last_read: now,
            mismatched: false,
        }
    }

    /// Start hunting for the echo of `frame`, which ended at `ended`.
    pub(super) fn start(&mut self, frame: &[u8], ended: Instant, grace: Duration) {
        self.expected.clear();
        self.expected.extend_from_slice(frame);
        self.matched = 0;
        self.phase = Phase::Grace(ended + grace);
        self.grace = grace;
        self.last_read = ended;
        self.mismatched = false;
    }

    /// The first `count` held octets, oldest first.
    pub(super) fn held(&self, count: usize) -> &[u8] {
        &self.expected[..count]
    }

    /// When the hunt next needs attention without an octet: the end of the
    /// grace or of the probe.
    pub(super) fn deadline(&self) -> Option<Instant> {
        match self.phase {
            Phase::Grace(end) | Phase::Probe(end) => Some(end),
            Phase::Off | Phase::Residual => None,
        }
    }

    /// Filter one octet read at `now`.
    pub(super) fn push(&mut self, octet: u8, now: Instant) -> Pushed {
        let mut pushed = Pushed::default();
        if self.phase == Phase::Off {
            pushed.pass = true;
            return pushed;
        }
        let silent = now.saturating_duration_since(self.last_read) > self.frame_abort;
        if self.matched > 0 && silent {
            pushed.stale = std::mem::take(&mut self.matched);
        }
        self.last_read = now;
        // The probe has its own deadline; only the residual hunt ends with
        // silence.
        if silent && self.phase == Phase::Residual {
            self.phase = Phase::Off;
            self.matched = 0;
            pushed.pass = true;
            return pushed;
        }
        if self.expected.get(self.matched) == Some(&octet) {
            self.matched += 1;
            if self.matched == self.expected.len() {
                self.phase = Phase::Off;
                self.matched = 0;
                pushed.echo = true;
            }
            return pushed;
        }
        let shift = self.shift(octet);
        if shift <= self.matched {
            pushed.release = shift;
            self.matched = self.matched + 1 - shift;
        } else {
            pushed.release = self.matched;
            pushed.pass = true;
            self.matched = 0;
        }
        if !matches!(self.phase, Phase::Probe(_)) && !self.mismatched {
            self.mismatched = true;
            pushed.first_mismatch = true;
        }
        pushed
    }

    /// The fewest held octets to pass on so that the rest, then `octet`,
    /// could begin the echo again; one more than are held means none can,
    /// and `octet` passes on too.
    fn shift(&self, octet: u8) -> usize {
        let held = self.matched;
        (1..=held)
            .find(|&shift| {
                let rest = held - shift;
                self.expected[shift..held] == self.expected[..rest]
                    && self.expected.get(rest) == Some(&octet)
            })
            .unwrap_or(held + 1)
    }

    /// The grace or the probe ran out at `now`.
    pub(super) fn timer(&mut self, now: Instant) -> Ended {
        let mut ended = Ended::default();
        match self.phase {
            Phase::Grace(end) if now >= end => {
                if self.matched == 0 {
                    // Missing only if nothing at all came where the echo
                    // should have: octets passed on are a mismatch instead.
                    ended.missing = !self.mismatched;
                    self.phase = Phase::Probe(end + self.grace);
                } else {
                    self.phase = Phase::Residual;
                }
            }
            Phase::Probe(end) if now >= end => ended = self.end_probe(now),
            _ => {}
        }
        ended
    }

    /// Before this station tries to start a frame: octets held in probe
    /// mode may be a peer's, so they pass on now, as line activity. The
    /// other phases are left alone until a start is confirmed: a start can
    /// still be refused, and what they hold may yet prove to be a peer's.
    pub(super) fn before_start(&mut self, now: Instant) -> Ended {
        match self.phase {
            Phase::Probe(_) => self.end_probe(now),
            Phase::Off | Phase::Grace(_) | Phase::Residual => Ended::default(),
        }
    }

    /// This station's start was confirmed: octets still held in the grace or
    /// residual phases were the last echo's continuation. End the hunt and
    /// return how many octets were dropped.
    pub(super) fn confirmed_start(&mut self) -> usize {
        self.phase = Phase::Off;
        std::mem::take(&mut self.matched)
    }

    /// End the probe: release what it holds, or drop it if it has waited
    /// longer than T_frame_abort.
    fn end_probe(&mut self, now: Instant) -> Ended {
        let mut ended = Ended::default();
        let held = std::mem::take(&mut self.matched);
        if held > 0 {
            if now.saturating_duration_since(self.last_read) > self.frame_abort {
                ended.stale = held;
            } else {
                ended.release = held;
            }
        }
        self.phase = Phase::Off;
        ended
    }
}

#[cfg(test)]
#[path = "echo/tests.rs"]
mod tests;
