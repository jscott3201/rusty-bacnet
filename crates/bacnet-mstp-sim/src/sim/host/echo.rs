//! Simulator-owned echo policy, modeled from
//! `bacnet-transport/src/mstp/echo.rs` at fffd440c. This does not execute the
//! production filter. Paired public-transport fixtures check observable
//! peer delivery and counters to detect policy drift.
//! Uses virtual core time; preserves Grace, Residual and Probe semantics.

use bacnet_mstp_core::{Duration, Instant};

/// What to do after one octet read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Pushed {
    /// Held octets dropped as a stale fragment of the echo, before this
    /// octet was looked at.
    pub(crate) stale: usize,
    /// How many of the oldest held octets to pass on, in order
    /// ([`EchoFilter::held`]).
    pub(crate) release: usize,
    /// Whether the octet itself passes on, after them.
    pub(crate) pass: bool,
    /// The octet completed the echo, which is dropped.
    pub(crate) echo: bool,
    /// The first octets of this hunt to pass on while the echo was expected
    /// (not in probe mode): the echo differed from what was sent, or other
    /// octets came with it.
    pub(crate) first_mismatch: bool,
}

impl Pushed {
    /// Whether anything passes on to the core.
    pub(crate) fn passes(&self) -> bool {
        self.release > 0 || self.pass
    }
}

/// What a timer or a new start does to the hunt.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Ended {
    /// Held octets dropped as stale.
    pub(crate) stale: usize,
    /// Held octets to pass on.
    pub(crate) release: usize,
    /// The grace ended with none of the echo held: count it missing.
    pub(crate) missing: bool,
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
pub(crate) struct EchoFilter {
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
    pub(crate) fn new(frame_abort: Duration) -> Self {
        let now = Instant::ZERO;
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
    pub(crate) fn start(&mut self, frame: &[u8], ended: Instant, grace: Duration) {
        self.expected.clear();
        self.expected.extend_from_slice(frame);
        self.matched = 0;
        self.phase = Phase::Grace(ended + grace);
        self.grace = grace;
        self.last_read = ended;
        self.mismatched = false;
    }

    /// The first `count` held octets, oldest first.
    pub(crate) fn held(&self, count: usize) -> &[u8] {
        &self.expected[..count]
    }

    /// When the hunt next needs attention without an octet: the end of the
    /// grace or of the probe.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        match self.phase {
            Phase::Grace(end) | Phase::Probe(end) => Some(end),
            Phase::Off | Phase::Residual => None,
        }
    }

    /// Filter one octet read at `now`.
    pub(crate) fn push(&mut self, octet: u8, now: Instant) -> Pushed {
        let mut pushed = Pushed::default();
        if self.phase == Phase::Off {
            pushed.pass = true;
            return pushed;
        }
        let silent = (now - self.last_read) > self.frame_abort;
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
    pub(crate) fn timer(&mut self, now: Instant) -> Ended {
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
    pub(crate) fn before_start(&mut self, now: Instant) -> Ended {
        match self.phase {
            Phase::Probe(_) => self.end_probe(now),
            Phase::Off | Phase::Grace(_) | Phase::Residual => Ended::default(),
        }
    }

    /// This station's start was confirmed: octets still held in the grace or
    /// residual phases were the last echo's continuation. End the hunt and
    /// return how many octets were dropped.
    pub(crate) fn confirmed_start(&mut self) -> usize {
        self.phase = Phase::Off;
        std::mem::take(&mut self.matched)
    }

    /// End the probe: release what it holds, or drop it if it has waited
    /// longer than T_frame_abort.
    fn end_probe(&mut self, now: Instant) -> Ended {
        let mut ended = Ended::default();
        let held = std::mem::take(&mut self.matched);
        if held > 0 {
            if (now - self.last_read) > self.frame_abort {
                ended.stale = held;
            } else {
                ended.release = held;
            }
        }
        self.phase = Phase::Off;
        ended
    }
}
