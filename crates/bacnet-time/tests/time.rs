//! The public API: each constructor, accessor and operation, at zero, at
//! `u64::MAX` and at the rounding edges of each unit.

use bacnet_time::{Duration, DurationTooLong, Instant};

const MAX: u64 = u64::MAX;

#[test]
fn constructors_scale_exactly() {
    assert_eq!(Duration::from_nanos(7).as_nanos(), 7);
    assert_eq!(Duration::from_micros(7).as_nanos(), 7_000);
    assert_eq!(Duration::from_millis(7).as_nanos(), 7_000_000);
    assert_eq!(Duration::from_secs(7).as_nanos(), 7_000_000_000);
    assert_eq!(Instant::from_nanos(7).as_nanos(), 7);
    assert_eq!(Instant::from_micros(7).as_nanos(), 7_000);
    assert_eq!(Instant::from_millis(7).as_nanos(), 7_000_000);
    assert_eq!(Instant::from_secs(7).as_nanos(), 7_000_000_000);
    assert_eq!(Duration::from_secs(0), Duration::ZERO);
    assert!(Duration::ZERO.is_zero() && !Duration::from_nanos(1).is_zero());
    assert_eq!(Instant::from_millis(0), Instant::ZERO);
}

#[test]
fn constructors_saturate_one_past_the_largest_exact_value() {
    for (unit, scale) in [
        ("micros", 1_000),
        ("millis", 1_000_000),
        ("secs", 1_000_000_000),
    ] {
        let largest = MAX / scale;
        let construct = |count: u64| match unit {
            "micros" => (
                Duration::from_micros(count),
                Duration::try_from_micros(count),
                Instant::from_micros(count),
            ),
            "millis" => (
                Duration::from_millis(count),
                Duration::try_from_millis(count),
                Instant::from_millis(count),
            ),
            _ => (
                Duration::from_secs(count),
                Duration::try_from_secs(count),
                Instant::from_secs(count),
            ),
        };
        let (exact, checked, instant) = construct(largest);
        assert_eq!(exact.as_nanos(), largest * scale, "{unit}");
        assert_eq!(checked, Some(exact), "{unit}");
        assert_eq!(instant.as_nanos(), largest * scale, "{unit}");
        for over in [largest + 1, MAX] {
            let (saturated, checked, instant) = construct(over);
            assert_eq!(saturated, Duration::MAX, "{unit} {over}");
            assert_eq!(checked, None, "{unit} {over}");
            assert_eq!(instant, Instant::MAX, "{unit} {over}");
        }
    }
}

#[test]
fn accessors_round_toward_zero() {
    for (scale, accessor) in [
        (1_000, Duration::as_micros as fn(Duration) -> u64),
        (1_000_000, Duration::as_millis),
        (1_000_000_000, Duration::as_secs),
    ] {
        assert_eq!(accessor(Duration::ZERO), 0, "{scale}");
        assert_eq!(accessor(Duration::from_nanos(scale - 1)), 0, "{scale}");
        assert_eq!(accessor(Duration::from_nanos(scale)), 1, "{scale}");
        assert_eq!(accessor(Duration::from_nanos(2 * scale - 1)), 1, "{scale}");
        assert_eq!(accessor(Duration::from_nanos(2 * scale)), 2, "{scale}");
        assert_eq!(accessor(Duration::MAX), MAX / scale, "{scale}");
    }
    assert_eq!(Duration::MAX.as_micros(), 18_446_744_073_709_551);
    assert_eq!(Duration::MAX.as_millis(), 18_446_744_073_709);
    assert_eq!(Duration::MAX.as_secs(), 18_446_744_073);
}

#[test]
fn instant_accessors_count_from_the_epoch() {
    let t = Instant::from_nanos(3_999_999_999);
    assert_eq!(t.as_nanos(), 3_999_999_999);
    assert_eq!(t.as_micros(), 3_999_999);
    assert_eq!(t.as_millis(), 3_999);
    assert_eq!(t.as_secs(), 3);
    assert_eq!(Instant::MAX.as_secs(), MAX / 1_000_000_000);
    assert_eq!(Instant::ZERO.as_micros(), 0);
}

#[test]
fn each_unit_round_trips() {
    for count in [0, 1, 999, 1_000, 123_456_789] {
        assert_eq!(Duration::from_micros(count).as_micros(), count);
        assert_eq!(Duration::from_millis(count).as_millis(), count);
        assert_eq!(Duration::from_secs(count).as_secs(), count);
        assert_eq!(Instant::from_micros(count).as_micros(), count);
        assert_eq!(Instant::from_millis(count).as_millis(), count);
        assert_eq!(Instant::from_secs(count).as_secs(), count);
    }
}

#[test]
fn duration_arithmetic_is_checked_or_saturating() {
    let (a, b) = (Duration::from_nanos(7), Duration::from_nanos(3));
    assert_eq!(a.checked_add(b), Some(Duration::from_nanos(10)));
    assert_eq!(a + b, Duration::from_nanos(10));
    assert_eq!(Duration::MAX.checked_add(Duration::from_nanos(1)), None);
    assert_eq!(Duration::MAX + Duration::from_nanos(1), Duration::MAX);
    assert_eq!(
        Duration::from_nanos(MAX - 1).saturating_add(Duration::from_nanos(1)),
        Duration::MAX
    );
    assert_eq!(a.checked_sub(b), Some(Duration::from_nanos(4)));
    assert_eq!(a - b, Duration::from_nanos(4));
    assert_eq!(b.checked_sub(a), None);
    assert_eq!(b - a, Duration::ZERO);
    assert_eq!(a.checked_sub(a), Some(Duration::ZERO));
    let mut c = a;
    c += b;
    c -= Duration::from_nanos(1);
    assert_eq!(c, Duration::from_nanos(9));
    c -= Duration::MAX;
    assert_eq!(c, Duration::ZERO);
    c += Duration::MAX;
    c += Duration::MAX;
    assert_eq!(c, Duration::MAX);
}

#[test]
fn multiplication_is_checked_or_saturating() {
    let a = Duration::from_micros(1_042);
    assert_eq!(a.saturating_mul(511), Duration::from_micros(532_462));
    assert_eq!(a.checked_mul(511), Some(Duration::from_micros(532_462)));
    assert_eq!(a.saturating_mul(0), Duration::ZERO);
    assert_eq!(a.saturating_mul(1), a);
    let half = Duration::from_nanos(MAX / 2);
    assert_eq!(half.checked_mul(2), Some(Duration::from_nanos(MAX - 1)));
    assert_eq!(Duration::from_nanos(MAX / 2 + 1).checked_mul(2), None);
    assert_eq!(
        Duration::from_nanos(MAX / 2 + 1).saturating_mul(2),
        Duration::MAX
    );
    assert_eq!(Duration::MAX.saturating_mul(u32::MAX), Duration::MAX);
    assert_eq!(Duration::MAX.checked_mul(1), Some(Duration::MAX));
}

#[test]
fn remainders_take_out_whole_divisors() {
    let cycle = Duration::from_millis(1_280);
    assert_eq!(
        Duration::from_millis(3_000).checked_rem(cycle),
        Some(Duration::from_millis(440))
    );
    assert_eq!(cycle.checked_rem(cycle), Some(Duration::ZERO));
    assert_eq!(
        Duration::from_nanos(1).checked_rem(cycle),
        Some(Duration::from_nanos(1))
    );
    assert_eq!(
        Duration::MAX.checked_rem(Duration::from_nanos(1_000)),
        Some(Duration::from_nanos(MAX % 1_000))
    );
    assert_eq!(
        Duration::MAX.checked_rem(Duration::MAX),
        Some(Duration::ZERO)
    );
    assert_eq!(cycle.checked_rem(Duration::ZERO), None);
    assert_eq!(Duration::ZERO.checked_rem(Duration::ZERO), None);
}

#[test]
fn instant_arithmetic_is_checked_or_saturating() {
    let t = Instant::from_nanos(10);
    let d = Duration::from_nanos(4);
    assert_eq!(t.checked_add(d), Some(Instant::from_nanos(14)));
    assert_eq!(t + d, Instant::from_nanos(14));
    assert_eq!(Instant::MAX.checked_add(Duration::from_nanos(1)), None);
    assert_eq!(Instant::MAX + d, Instant::MAX);
    assert_eq!(Instant::MAX.checked_add(Duration::ZERO), Some(Instant::MAX));
    assert_eq!(
        Instant::from_nanos(MAX - 1).saturating_add(Duration::MAX),
        Instant::MAX
    );
    assert_eq!(t.checked_sub(d), Some(Instant::from_nanos(6)));
    assert_eq!(t - d, Instant::from_nanos(6));
    assert_eq!(t.checked_sub(Duration::from_nanos(10)), Some(Instant::ZERO));
    assert_eq!(t.checked_sub(Duration::from_nanos(11)), None);
    assert_eq!(t - Duration::from_nanos(11), Instant::ZERO);
    assert_eq!(Instant::ZERO.saturating_sub(Duration::MAX), Instant::ZERO);
    let mut u = t;
    u += d;
    u -= Duration::from_nanos(1);
    assert_eq!(u, Instant::from_nanos(13));
    u -= Duration::MAX;
    assert_eq!(u, Instant::ZERO);
    u += Duration::MAX;
    u += d;
    assert_eq!(u, Instant::MAX);
}

#[test]
fn the_time_between_instants_is_checked_or_saturating() {
    let (early, late) = (Instant::from_nanos(4), Instant::from_nanos(10));
    assert_eq!(
        late.checked_duration_since(early),
        Some(Duration::from_nanos(6))
    );
    assert_eq!(late - early, Duration::from_nanos(6));
    assert_eq!(late.checked_duration_since(late), Some(Duration::ZERO));
    assert_eq!(early.checked_duration_since(late), None);
    assert_eq!(early.saturating_duration_since(late), Duration::ZERO);
    assert_eq!(early - late, Duration::ZERO);
    assert_eq!(Instant::MAX - Instant::ZERO, Duration::MAX);
}

#[test]
fn ordering_follows_the_timeline() {
    assert!(Instant::ZERO < Instant::from_nanos(1));
    assert!(Instant::from_micros(1) < Instant::from_nanos(1_001));
    assert!(Duration::from_millis(1) > Duration::from_micros(999));
    assert_eq!(Duration::default(), Duration::ZERO);
}

#[test]
fn min_and_max_are_const_and_match_ord() {
    const SHORT: Duration = Duration::from_millis(100).max(Duration::from_millis(30));
    const EARLY: Instant = Instant::from_secs(2).min(Instant::from_millis(1_500));
    assert_eq!(SHORT, Duration::from_millis(100));
    assert_eq!(EARLY, Instant::from_millis(1_500));
    let values = [0, 1, 1_000, MAX - 1, MAX];
    for a in values {
        for b in values {
            let (x, y) = (Duration::from_nanos(a), Duration::from_nanos(b));
            assert_eq!(x.min(y), Ord::min(x, y), "{a} {b}");
            assert_eq!(x.max(y), Ord::max(x, y), "{a} {b}");
            let (s, t) = (Instant::from_nanos(a), Instant::from_nanos(b));
            assert_eq!(s.min(t), Ord::min(s, t), "{a} {b}");
            assert_eq!(s.max(t), Ord::max(s, t), "{a} {b}");
        }
    }
}

#[test]
fn core_durations_convert_exactly_or_are_refused() {
    for nanos in [0, 1, 999_999_999, 1_000_000_000, 1_000_000_001, MAX] {
        let ours = Duration::from_nanos(nanos);
        let core = core::time::Duration::from_nanos(nanos);
        assert_eq!(core::time::Duration::from(ours), core, "{nanos}");
        assert_eq!(Duration::try_from(core), Ok(ours), "{nanos}");
        assert_eq!(Duration::saturating_from_core(core), ours, "{nanos}");
    }
    // One nanosecond past ours, the next whole second, and core's own
    // maximum are refused, or clamped by the saturating form.
    let past = core::time::Duration::from_nanos(MAX) + core::time::Duration::from_nanos(1);
    for long in [
        past,
        core::time::Duration::new(MAX / 1_000_000_000 + 1, 0),
        core::time::Duration::MAX,
    ] {
        assert!(
            matches!(Duration::try_from(long), Err(DurationTooLong { .. })),
            "{long:?}"
        );
        assert_eq!(
            Duration::saturating_from_core(long),
            Duration::MAX,
            "{long:?}"
        );
    }
    const FROM_CORE: Duration = Duration::saturating_from_core(core::time::Duration::from_secs(3));
    assert_eq!(FROM_CORE, Duration::from_secs(3));
    let too_long = Duration::try_from(core::time::Duration::MAX).unwrap_err();
    assert_eq!(
        too_long.to_string(),
        "duration longer than u64::MAX nanoseconds (about 584 years)"
    );
}

#[test]
fn debug_prints_nanoseconds_with_the_unit() {
    for (nanos, text) in [
        (0, "0"),
        (1_500, "1500"),
        (999_999_999, "999999999"),
        (1_000_000_000, "1000000000"),
        (1_000_000_007, "1000000007"),
        (1_000_000_000_000_000_000, "1000000000000000000"),
        (1_000_000_000_000_000_005, "1000000000000000005"),
        (1_000_000_005_000_000_000, "1000000005000000000"),
        (MAX, "18446744073709551615"),
    ] {
        assert_eq!(
            format!("{:?}", Duration::from_nanos(nanos)),
            format!("Duration({text}ns)")
        );
        assert_eq!(
            format!("{:?}", Instant::from_nanos(nanos)),
            format!("Instant({text}ns)")
        );
    }
    assert_eq!(
        format!("{:?}", Some(Duration::from_micros(3))),
        "Some(Duration(3000ns))"
    );
}

proptest::proptest! {
    #[test]
    fn accessors_match_division(nanos in proptest::prelude::any::<u64>()) {
        let d = Duration::from_nanos(nanos);
        proptest::prop_assert_eq!(d.as_micros(), nanos / 1_000);
        proptest::prop_assert_eq!(d.as_millis(), nanos / 1_000_000);
        proptest::prop_assert_eq!(d.as_secs(), nanos / 1_000_000_000);
        let t = Instant::from_nanos(nanos);
        proptest::prop_assert_eq!(t.as_micros(), nanos / 1_000);
        proptest::prop_assert_eq!(t.as_millis(), nanos / 1_000_000);
        proptest::prop_assert_eq!(t.as_secs(), nanos / 1_000_000_000);
    }

    #[test]
    fn remainders_match_division(
        nanos in proptest::prelude::any::<u64>(),
        divisor in 1u64..,
    ) {
        proptest::prop_assert_eq!(
            Duration::from_nanos(nanos).checked_rem(Duration::from_nanos(divisor)),
            Some(Duration::from_nanos(nanos % divisor))
        );
    }

    #[test]
    fn core_round_trips(nanos in proptest::prelude::any::<u64>()) {
        let ours = Duration::from_nanos(nanos);
        let core = core::time::Duration::from(ours);
        proptest::prop_assert_eq!(core, core::time::Duration::from_nanos(nanos));
        proptest::prop_assert_eq!(Duration::try_from(core), Ok(ours));
        proptest::prop_assert_eq!(Duration::saturating_from_core(core), ours);
    }

    #[test]
    fn core_durations_past_ours_are_refused(
        secs in (MAX / 1_000_000_000)..=MAX,
        subsec in 0u32..1_000_000_000,
    ) {
        let core = core::time::Duration::new(secs, subsec);
        let fits = core.as_nanos() <= u128::from(MAX);
        proptest::prop_assert_eq!(Duration::try_from(core).is_ok(), fits);
        if !fits {
            proptest::prop_assert_eq!(Duration::saturating_from_core(core), Duration::MAX);
        }
    }

    #[test]
    fn debug_matches_the_decimal_count(nanos in proptest::prelude::any::<u64>()) {
        proptest::prop_assert_eq!(
            format!("{:?}", Duration::from_nanos(nanos)),
            format!("Duration({nanos}ns)")
        );
        proptest::prop_assert_eq!(
            format!("{:?}", Instant::from_nanos(nanos)),
            format!("Instant({nanos}ns)")
        );
    }
}
