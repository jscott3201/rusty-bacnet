//! Per-node baud and protocol-clock error (#1772).
//!
//! Clause 9.2.3 lets each node's bit rate be off its nominal standard rate
//! by up to one percent either way. A node keeps its protocol timers on a
//! local clock that may be off too. The standard fixes no separate figure
//! for that clock: its SilenceTimer is a coarse timer (a nominal 5 ms
//! resolution, Clause 9.5.2), and a node commonly derives its UART's bit
//! clock and its timers from one oscillator. The simulator caps the clock's
//! error at the same one percent, so a scenario explores the band a node
//! built to Clause 9.2.3 can occupy, and no wider.

use bacnet_mstp_core::{Duration, Instant};

/// The largest error either rate may have, in parts per million: one
/// percent (Clause 9.2.3).
pub const MAX_SKEW_PPM: i32 = 10_000;

const MILLION: i128 = 1_000_000;

/// How far one master's rates are off nominal, in parts per million.
/// Positive is fast. The default is a perfect node.
///
/// One percent is a stress band, far beyond a crystal-driven node, which
/// is within about 100 ppm; it is what an RC oscillator or a fractional
/// baud divider can reach.
///
/// Known limit (#1804): the simulator's deadlines are exact, so two nodes
/// can stay in lockstep where real ones fall out of step through
/// SilenceTimer's coarse resolution and their timers' phase. At 9,600 and
/// 19,200 baud, adjacent masters at opposite clock edges keep colliding
/// at token generation unless their adapters add deadline jitter; at
/// 38,400 baud and above they collide once, then form a ring.
///
/// Only the core's clock is skewed. A [`HostSpec`](crate::HostSpec)
/// adapter's own timers (read latency, drain, echo grace) stay on the
/// line's clock; that combination is untested.
///
/// - `baud_ppm`: its UART's bit rate. A fast UART's bits are shorter, so
///   its frames take less time on the line; receivers take them as sent,
///   since a UART tolerates far more than the two percent two nodes at
///   opposite edges differ by.
/// - `clock_ppm`: the clock its core reads. The core works in its own
///   time: a fast clock reaches each deadline early on the line's clock,
///   a slow one late. Its configured bit-time timers were worked out from
///   the nominal baud rate, so they don't follow `baud_ppm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Skew {
    /// Bit rate error of the node's UART.
    pub baud_ppm: i32,
    /// Rate error of the clock the node's core reads.
    pub clock_ppm: i32,
}

impl Skew {
    /// Both rates off by the same `ppm`, as when one oscillator drives the
    /// UART and the timers.
    #[must_use]
    pub const fn common(ppm: i32) -> Self {
        Self {
            baud_ppm: ppm,
            clock_ppm: ppm,
        }
    }

    pub(crate) fn validate(self) {
        for (what, ppm) in [("baud", self.baud_ppm), ("clock", self.clock_ppm)] {
            assert!(
                ppm.abs() <= MAX_SKEW_PPM,
                "{what} error {ppm} ppm is outside the one percent Clause 9.2.3 allows"
            );
        }
    }
}

/// A node's local clock: it reads the line's time at boot, then runs at its
/// own rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Clock {
    origin: Instant,
    ppm: i32,
}

impl Clock {
    pub(crate) const PERFECT: Self = Self {
        origin: Instant::ZERO,
        ppm: 0,
    };

    /// A clock that reads `origin` at `origin` and runs `ppm` fast.
    pub(crate) fn starting(origin: Instant, ppm: i32) -> Self {
        Self { origin, ppm }
    }

    /// What the clock reads at line time `at`, rounded down. Never earlier
    /// than its origin, and monotonic.
    pub(crate) fn local(self, at: Instant) -> Instant {
        if self.ppm == 0 || at <= self.origin {
            return at;
        }
        let elapsed = i128::from((at - self.origin).as_nanos());
        let scaled = elapsed * (MILLION + i128::from(self.ppm)) / MILLION;
        self.origin + Duration::from_nanos(u64::try_from(scaled).unwrap_or(u64::MAX))
    }

    /// The first line time at which the clock reads `local` or later, so a
    /// deadline serviced then is due on the node's own clock.
    pub(crate) fn line_time(self, local: Instant) -> Instant {
        if self.ppm == 0 || local <= self.origin {
            return local;
        }
        let elapsed = i128::from((local - self.origin).as_nanos());
        let rate = MILLION + i128::from(self.ppm);
        let scaled = (elapsed * MILLION + rate - 1) / rate;
        self.origin + Duration::from_nanos(u64::try_from(scaled).unwrap_or(u64::MAX))
    }

    /// The time between two line instants, as the node's clock measures it.
    pub(crate) fn span(self, from: Instant, to: Instant) -> Duration {
        self.local(to) - self.local(from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SimRng;

    #[test]
    fn a_perfect_clock_reads_the_line() {
        let clock = Clock::starting(Instant::from_millis(7), 0);
        for at in [0, 7_000_000, 123_456_789] {
            let at = Instant::from_nanos(at);
            assert_eq!(clock.local(at), at);
            assert_eq!(clock.line_time(at), at);
        }
    }

    #[test]
    fn edge_clocks_run_one_percent_off() {
        let origin = Instant::from_millis(100);
        let fast = Clock::starting(origin, MAX_SKEW_PPM);
        let slow = Clock::starting(origin, -MAX_SKEW_PPM);
        let later = origin + Duration::from_millis(500);
        assert_eq!(fast.local(later), origin + Duration::from_millis(505));
        assert_eq!(slow.local(later), origin + Duration::from_millis(495));
        // 20 ms on a fast clock is 19.80198... ms of line time.
        let due = origin + Duration::from_millis(20);
        assert_eq!(
            fast.line_time(due),
            origin + Duration::from_nanos(19_801_981)
        );
        assert_eq!(
            slow.line_time(due),
            origin + Duration::from_nanos(20_202_021)
        );
    }

    /// The line time of a deadline is the first instant the clock reads it:
    /// due then, and not a nanosecond before.
    #[test]
    fn line_time_is_the_first_instant_a_deadline_is_due() {
        let mut rng = SimRng::new(1772);
        for _ in 0..10_000 {
            let ppm = i32::try_from(rng.range(0, 2 * MAX_SKEW_PPM as u64)).unwrap() - MAX_SKEW_PPM;
            let origin = Instant::from_nanos(rng.range(0, 1 << 40));
            let clock = Clock::starting(origin, ppm);
            let local = origin + Duration::from_nanos(rng.range(1, 1 << 42));
            let at = clock.line_time(local);
            assert!(clock.local(at) >= local, "{ppm} ppm");
            assert!(
                clock.local(at - Duration::from_nanos(1)) < local,
                "{ppm} ppm"
            );
        }
    }
}
