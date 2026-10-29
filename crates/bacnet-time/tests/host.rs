//! `HostClock`: a host's `std::time::Instant` on the timeline, both ways.
//! Every instant here is one host instant plus or minus a fixed offset, so
//! nothing waits on the real clock.
#![cfg(feature = "std")]

use std::sync::OnceLock;
use std::time::{Duration as StdDuration, Instant as StdInstant};

use bacnet_time::{Duration, HostClock, Instant};

const MIDPOINT: u64 = 1 << 63;

const OFFSETS: [StdDuration; 4] = [
    StdDuration::from_nanos(1),
    StdDuration::from_nanos(100),
    StdDuration::from_nanos(123_456_789),
    StdDuration::from_secs(91),
];

/// The nanoseconds from `earlier` to `later` as the host clock measures
/// them: Windows counts 100 ns ticks, so an offset may round.
fn nanos(later: StdInstant, earlier: StdInstant) -> u64 {
    u64::try_from(later.duration_since(earlier).as_nanos()).unwrap()
}

#[test]
fn a_clock_from_an_origin_maps_it_to_zero_and_later_instants_exactly() {
    let origin = StdInstant::now();
    let clock = HostClock::new(origin);
    assert_eq!(clock.origin(), origin);
    assert_eq!(clock.instant(origin), Instant::ZERO);
    assert_eq!(clock.to_std(Instant::ZERO), Some(origin));
    for offset in OFFSETS {
        let later = origin + offset;
        let at = clock.instant(later);
        assert_eq!(at.as_nanos(), nanos(later, origin));
        assert_eq!(clock.to_std(at), Some(later));
    }
}

#[test]
fn a_clock_from_an_origin_clamps_earlier_instants_to_zero() {
    let origin = StdInstant::now();
    let clock = HostClock::new(origin);
    for offset in OFFSETS {
        if let Some(earlier) = origin.checked_sub(offset) {
            assert_eq!(clock.instant(earlier), Instant::ZERO);
        }
    }
}

/// The BBMD's case: a registration backdated before the clock's origin
/// keeps its time, both ways.
#[test]
fn a_centred_clock_maps_instants_either_side_of_its_origin_exactly() {
    let origin = StdInstant::now();
    let clock = HostClock::centred(origin);
    assert_eq!(clock.instant(origin).as_nanos(), MIDPOINT);
    assert_eq!(clock.to_std(clock.instant(origin)), Some(origin));
    for offset in OFFSETS {
        let later = origin + offset;
        assert_eq!(
            clock.instant(later).as_nanos(),
            MIDPOINT + nanos(later, origin)
        );
        assert_eq!(clock.to_std(clock.instant(later)), Some(later));
        if let Some(earlier) = origin.checked_sub(offset) {
            assert_eq!(
                clock.instant(earlier).as_nanos(),
                MIDPOINT - nanos(origin, earlier)
            );
            assert_eq!(clock.to_std(clock.instant(earlier)), Some(earlier));
            assert!(clock.instant(earlier) <= clock.instant(origin));
        }
    }
}

#[test]
fn far_instants_saturate_at_the_ends_of_the_timeline() {
    let origin = StdInstant::now();
    // About 600 years: past Instant::MAX from the origin, and past either
    // end from the middle. A host clock that can't hold it skips the check.
    let far = StdDuration::from_secs(600 * 365 * 86_400);
    if let Some(later) = origin.checked_add(far) {
        assert_eq!(HostClock::new(origin).instant(later), Instant::MAX);
        assert_eq!(HostClock::centred(origin).instant(later), Instant::MAX);
    }
    if let Some(earlier) = origin.checked_sub(far) {
        assert_eq!(HostClock::centred(origin).instant(earlier), Instant::ZERO);
    }
}

/// A fixed host instant, and a source that reads 5 ms after it.
static BASE: OnceLock<StdInstant> = OnceLock::new();

fn base() -> StdInstant {
    *BASE.get_or_init(StdInstant::now)
}

fn five_ms_later() -> StdInstant {
    base() + StdDuration::from_millis(5)
}

#[test]
fn now_reads_the_source_the_host_chose() {
    let clock = HostClock::new(base()).reading(five_ms_later);
    assert_eq!(clock.now(), Instant::from_millis(5));
    assert_eq!(clock.origin(), base());
    let centred = HostClock::centred(base()).reading(five_ms_later);
    assert_eq!(
        centred.now(),
        Instant::from_nanos(MIDPOINT) + Duration::from_millis(5)
    );
}

#[test]
fn now_reads_the_std_clock_by_default() {
    let origin = StdInstant::now();
    let clock = HostClock::new(origin);
    let now = clock.now();
    let after = StdInstant::now();
    assert!(now <= clock.instant(after));
    assert!(
        clock
            .to_std(now)
            .is_some_and(|at| at >= origin && at <= after)
    );
}
