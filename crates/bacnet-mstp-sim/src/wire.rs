//! Bit timing on the simulated wire.

use bacnet_mstp_core::{Duration, Instant};

/// Bit times one octet occupies: a start bit, eight data bits and a stop bit,
/// with no parity (Clause 9.2.3).
pub const BITS_PER_OCTET: u64 = 10;

/// Converts bit and octet counts to virtual time at one baud rate.
///
/// Octet boundaries are measured from the start of a transmission and rounded
/// up to a whole microsecond each, so rounding never accumulates over a long
/// frame: contiguous octet `i` of a transmission that starts at `t` occupies
/// `t + boundary(i) .. t + boundary(i + 1)`. A captured extra gap shifts both
/// boundaries by `i * gap` without changing this original character grid.
///
/// A node whose UART runs off its nominal rate sends on a grid of its own
/// ([`Self::with_rate_error`]); the limits below stay at the nominal rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireTiming {
    baud: u32,
    /// The bit rate's error, in parts per million: positive is fast.
    ppm: i32,
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
        Self { baud, ppm: 0 }
    }

    /// The same nominal rate, sent `ppm` parts per million fast (or slow,
    /// when negative): the grid a node with that UART error sends on.
    ///
    /// # Panics
    ///
    /// If `ppm` is outside [`MAX_SKEW_PPM`](crate::MAX_SKEW_PPM) either
    /// way.
    #[must_use]
    pub const fn with_rate_error(self, ppm: i32) -> Self {
        assert!(
            ppm.abs() <= crate::MAX_SKEW_PPM,
            "a rate error outside the one percent Clause 9.2.3 allows"
        );
        Self {
            baud: self.baud,
            ppm,
        }
    }

    /// The baud rate.
    #[must_use]
    pub const fn baud(self) -> u32 {
        self.baud
    }

    /// `bits` bit times at this grid's rate, rounded up to a whole
    /// microsecond.
    #[must_use]
    pub fn bits(self, bits: u64) -> Duration {
        if self.ppm == 0 {
            return Duration::from_micros((bits * 1_000_000).div_ceil(u64::from(self.baud)));
        }
        // bits / (baud * (1 + ppm / 10^6)) seconds, in microseconds.
        let rate =
            u128::from(self.baud) * u128::try_from(1_000_000 + i64::from(self.ppm)).unwrap_or(1);
        let micros = (u128::from(bits) * 1_000_000_000_000).div_ceil(rate);
        Duration::from_micros(u64::try_from(micros).unwrap_or(u64::MAX))
    }

    /// Time from the start of a contiguous transmission to the start of its octet
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

    /// Largest whole-nanosecond hold after the final stop bit's end.
    ///
    /// 135-2020 9.2.3 counts a maximum 15 bit times from the stop bit's
    /// beginning; 9.5.3 counts from its end. This model uses their conservative
    /// intersection: at most 14 bit times after the end. The upper bound is
    /// rounded down, unlike character boundaries, so it cannot be widened.
    #[must_use]
    pub fn max_post_drive(self) -> Duration {
        Duration::from_nanos(14_000_000_000 / u64::from(self.baud))
    }

    /// Largest whole-nanosecond extra gap after a non-final stop bit.
    /// The 20-bit idle limit includes the stop bit (135-2020 9.2.3), leaving
    /// at most 19 bits after its end. Round down so the limit is not widened.
    #[must_use]
    pub fn max_inter_octet_gap(self) -> Duration {
        Duration::from_nanos(19_000_000_000 / u64::from(self.baud))
    }

    pub(crate) fn validate_inter_octet_gap(self, gap: Duration) {
        assert!(
            u128::from(gap.as_nanos()) * u128::from(self.baud) <= 19_000_000_000,
            "inter-octet gap exceeds 19 bit times after the stop bit"
        );
    }

    /// Duration of a frame with a uniform extra gap after each non-final
    /// stop bit. Use this physical duration when budgeting a scenario's
    /// transmit watchdog; the simulator does not enlarge core defaults.
    ///
    /// # Panics
    ///
    /// If `gap` exceeds [`Self::max_inter_octet_gap`].
    #[must_use]
    pub fn duration_with_gap(self, octets: usize, gap: Duration) -> Duration {
        self.schedule(Instant::ZERO, gap).end(octets) - Instant::ZERO
    }

    pub(crate) fn schedule(self, start: Instant, gap: Duration) -> WireSchedule {
        self.validate_inter_octet_gap(gap);
        WireSchedule {
            timing: self,
            start,
            gap,
        }
    }

    pub(crate) fn validate_post_drive(self, hold: Duration) {
        assert!(
            u128::from(hold.as_nanos()) * u128::from(self.baud) <= 14_000_000_000,
            "post-drive hold exceeds 14 bit times after the final stop bit"
        );
    }

    /// When a contiguous transmission of `octets` starting at `start` ends.
    #[must_use]
    pub fn end_of(self, start: Instant, octets: usize) -> Instant {
        start + self.boundary(octets)
    }
}

/// One immutable transmission schedule. All boundaries use the original
/// character grid plus the count of preceding gaps, never rounded strides.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WireSchedule {
    timing: WireTiming,
    pub start: Instant,
    gap: Duration,
}

impl WireSchedule {
    fn gaps(self, count: usize) -> Duration {
        Duration::from_nanos(
            self.gap
                .as_nanos()
                .saturating_mul(u64::try_from(count).unwrap_or(u64::MAX)),
        )
    }

    pub(crate) fn octet_start(self, index: usize) -> Instant {
        self.start + self.timing.boundary(index) + self.gaps(index)
    }

    pub(crate) fn octet_end(self, index: usize) -> Instant {
        self.start + self.timing.boundary(index + 1) + self.gaps(index)
    }

    pub(crate) fn end(self, octets: usize) -> Instant {
        self.start + self.timing.boundary(octets) + self.gaps(octets.saturating_sub(1))
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

    #[test]
    fn a_rate_error_scales_the_grid_and_keeps_the_limits() {
        let nominal = WireTiming::new(9_600);
        let fast = nominal.with_rate_error(10_000);
        let slow = nominal.with_rate_error(-10_000);
        // 10 bits at 9696 and at 9504 baud.
        assert_eq!(fast.octet(), Duration::from_micros(1_032));
        assert_eq!(slow.octet(), Duration::from_micros(1_053));
        assert_eq!(fast.boundary(96), Duration::from_micros(99_010));
        assert_eq!(slow.boundary(95), Duration::from_micros(99_958));
        assert_eq!(nominal.with_rate_error(0), nominal);
        assert_eq!(fast.max_post_drive(), nominal.max_post_drive());
        assert_eq!(slow.max_inter_octet_gap(), nominal.max_inter_octet_gap());
    }

    #[test]
    fn post_drive_limit_rounds_down_and_rejects_the_next_nanosecond() {
        for (baud, max_ns) in [(10_000, 1_400_000), (115_200, 121_527), (9_600, 1_458_333)] {
            let timing = WireTiming::new(baud);
            assert_eq!(timing.max_post_drive(), Duration::from_nanos(max_ns));
            timing.validate_post_drive(Duration::ZERO);
            timing.validate_post_drive(Duration::from_nanos(max_ns));
            assert!(std::panic::catch_unwind(|| {
                timing.validate_post_drive(Duration::from_nanos(max_ns + 1));
            })
            .is_err());
            assert!(std::panic::catch_unwind(|| {
                timing.validate_post_drive(timing.bits(15));
            })
            .is_err());
        }
        // Rounding the 14-bit upper bound up to a character's microsecond
        // granularity would admit 122 us at 115200, exceeding the bound.
        assert!(std::panic::catch_unwind(|| {
            WireTiming::new(115_200).validate_post_drive(Duration::from_micros(122));
        })
        .is_err());
    }

    #[test]
    fn gap_bounds_and_maximum_frame_times_use_independent_integer_oracles() {
        for (baud, gap_ns, standard_ns, extended_ns) in [
            (9_600, 1_979_166, 1_541_666_660, 4_577_603_490),
            (38_400, 494_791, 385_416_410, 1_144_400_365),
            (115_200, 164_930, 128_472_300, 381_466_950),
        ] {
            let timing = WireTiming::new(baud);
            let gap = Duration::from_nanos(gap_ns);
            assert_eq!(timing.max_inter_octet_gap(), gap);
            assert_eq!(
                timing.duration_with_gap(511, gap),
                Duration::from_nanos(standard_ns)
            );
            assert_eq!(
                timing.duration_with_gap(1516, gap),
                Duration::from_nanos(extended_ns)
            );
            assert_eq!(timing.duration_with_gap(1, gap), timing.octet());
            assert_eq!(timing.duration_with_gap(0, gap), Duration::ZERO);
            assert_eq!(
                timing.duration_with_gap(1516, Duration::ZERO),
                timing.boundary(1516)
            );
            assert!(std::panic::catch_unwind(|| {
                timing.validate_inter_octet_gap(Duration::from_nanos(gap_ns + 1));
            })
            .is_err());
            assert!(std::panic::catch_unwind(|| {
                timing.validate_inter_octet_gap(timing.bits(19));
            })
            .is_err());
        }
        assert_eq!(
            WireTiming::new(10_000).max_inter_octet_gap(),
            Duration::from_micros(1_900)
        );
        assert!(std::panic::catch_unwind(|| {
            WireTiming::new(10_000).validate_inter_octet_gap(Duration::from_nanos(1_900_001));
        })
        .is_err());
    }
}
