//! Division without a 64-bit division routine.

/// `(n / d, n % d)` for a nonzero `d`, by binary long division: shifts,
/// compares and subtractions only, so a 32-bit target links no 64-bit
/// division routine for it.
///
/// It lines `d` up with the top bit of `n`, then takes one bit of the
/// quotient per step, from the top: one step for a quotient below 2, and at
/// most 64. Dividing by one of the unit sizes takes about 30 steps for a
/// time of a few hours.
///
/// `d` must be nonzero; every caller divides by a nonzero constant or checks
/// first. With a zero `d` it returns `(0, n)` rather than loop or panic.
pub(crate) const fn div_rem(n: u64, d: u64) -> (u64, u64) {
    if d == 0 || n < d {
        return (0, n);
    }
    // `n >= d`, so `n` has no more leading zeros than `d`, and shifting `d`
    // left by the difference keeps all of its bits.
    let shift = d.leading_zeros() - n.leading_zeros();
    let mut step = d << shift;
    let mut rest = n;
    let mut quotient = 0;
    let mut steps = shift + 1;
    while steps > 0 {
        quotient <<= 1;
        if rest >= step {
            rest -= step;
            quotient |= 1;
        }
        step >>= 1;
        steps -= 1;
    }
    (quotient, rest)
}

#[cfg(test)]
mod tests {
    use super::div_rem;

    fn native(n: u64, d: u64) -> (u64, u64) {
        (n / d, n % d)
    }

    #[test]
    fn matches_native_division_at_the_edges() {
        let divisors = [
            1,
            2,
            3,
            7,
            1_000,
            1_000_000,
            1_000_000_000,
            1 << 31,
            (1 << 32) - 1,
            1 << 32,
            (1 << 63) - 1,
            1 << 63,
            u64::MAX - 1,
            u64::MAX,
        ];
        for d in divisors {
            for n in [
                0,
                1,
                d - 1,
                d,
                d.wrapping_add(1),
                d.saturating_mul(2),
                d.saturating_mul(2).saturating_sub(1),
                (1 << 63) - 1,
                1 << 63,
                u64::MAX - 1,
                u64::MAX,
            ] {
                assert_eq!(div_rem(n, d), native(n, d), "{n} / {d}");
            }
        }
    }

    #[test]
    fn a_zero_divisor_leaves_the_dividend() {
        assert_eq!(div_rem(0, 0), (0, 0));
        assert_eq!(div_rem(u64::MAX, 0), (0, u64::MAX));
    }

    proptest::proptest! {
        #[test]
        fn matches_native_division(n in proptest::prelude::any::<u64>(), d in 1u64..) {
            proptest::prop_assert_eq!(div_rem(n, d), native(n, d));
        }

        #[test]
        fn matches_native_division_by_short_divisors(
            n in proptest::prelude::any::<u64>(),
            d in 1u64..(1 << 32),
        ) {
            proptest::prop_assert_eq!(div_rem(n, d), native(n, d));
        }
    }
}
