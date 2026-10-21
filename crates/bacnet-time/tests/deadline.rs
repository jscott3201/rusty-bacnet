//! `earliest`: optional deadlines merged with `None` as no deadline.

use bacnet_time::{earliest, Instant};

const AT: Option<Instant> = Some(Instant::from_millis(5));

/// `Option::min` orders `None` first, so it loses a set deadline when the
/// other is unset; `earliest` keeps it, whichever side it is on.
#[test]
fn earliest_keeps_the_set_deadline_that_option_min_drops() {
    assert_eq!(Option::min(None, AT), None);
    assert_eq!(Option::min(AT, None), None);
    assert_eq!(earliest([None, AT]), AT);
    assert_eq!(earliest([AT, None]), AT);
}

#[test]
fn earliest_is_the_smallest_set_deadline() {
    let three = Some(Instant::from_millis(3));
    assert_eq!(earliest([AT, three]), three);
    assert_eq!(earliest([three, AT]), three);
    assert_eq!(earliest([AT, None, three, None]), three);
    assert_eq!(earliest([AT, AT]), AT);
    assert_eq!(earliest::<Instant>([None, None]), None);
    assert_eq!(earliest::<Instant>([]), None);
}

/// Any ordered time works, a host's `std::time::Instant` included.
#[test]
fn earliest_takes_a_host_instant() {
    let now = std::time::Instant::now();
    let later = now + std::time::Duration::from_millis(1);
    assert_eq!(earliest([None, Some(later), Some(now)]), Some(now));
}
