//! Bit timing on the simulated wire.

use bacnet_mstp_core::{Duration, Instant};

/// Bit times one octet occupies: a start bit, eight data bits and a stop bit,
/// with no parity (Clause 9.2.3).
pub const BITS_PER_OCTET: u64 = 10;

/// Converts bit and octet counts to virtual time at one baud rate.
///
/// Octet boundaries are measured from the start of a transmission and rounded
/// up to a whole microsecond each, so rounding never accumulates over a long
/// frame: octet `i` of a transmission that starts at `t` occupies
/// `t + boundary(i) .. t + boundary(i + 1)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireTiming {
    baud: u32,
}

impl WireTiming {
    /// Timing for `baud` bits per second.
    ///
    /// # Panics
    ///
    /// If `baud` is zero.
    #[must_use]
    pub fn new(baud: u32) -> Self {
        assert!(baud > 0, "the simulated line needs a non-zero baud rate");
        Self { baud }
    }

    /// The baud rate.
    #[must_use]
    pub const fn baud(self) -> u32 {
        self.baud
    }

    /// `bits` bit times, rounded up to a whole microsecond.
    #[must_use]
    pub fn bits(self, bits: u64) -> Duration {
        Duration::from_micros((bits * 1_000_000).div_ceil(u64::from(self.baud)))
    }

    /// Time from the start of a transmission to the start of its octet
    /// `octets` (equally, to the end of the octet before it).
    #[must_use]
    pub fn boundary(self, octets: usize) -> Duration {
        let octets = u64::try_from(octets).unwrap_or(u64::MAX);
        self.bits(octets.saturating_mul(BITS_PER_OCTET))
    }

    /// One octet time, rounded up.
    #[must_use]
    pub fn octet(self) -> Duration {
        self.boundary(1)
    }

    /// When a transmission of `octets` octets that starts at `start` ends.
    #[must_use]
    pub fn end_of(self, start: Instant, octets: usize) -> Instant {
        start + self.boundary(octets)
    }
}

#[cfg(test)]
mod tests {
    use super::WireTiming;
    use bacnet_mstp_core::{Duration, Instant};

    #[test]
    fn boundaries_round_up_without_drift() {
        let timing = WireTiming::new(9_600);
        // 10 bits at 9600 baud is 1041.67 us.
        assert_eq!(timing.octet(), Duration::from_micros(1_042));
        // Three octets are 3125 us exactly; summing rounded octets would give 3126.
        assert_eq!(timing.boundary(3), Duration::from_micros(3_125));
        assert_eq!(timing.boundary(8), Duration::from_micros(8_334));
        assert_eq!(
            timing.end_of(Instant::from_micros(10), 3),
            Instant::from_micros(3_135)
        );
        // 40 bit times, Tturnaround's floor at 38400 baud.
        assert_eq!(
            WireTiming::new(38_400).bits(40),
            Duration::from_micros(1_042)
        );
    }
}
