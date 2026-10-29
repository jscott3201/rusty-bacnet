//! Route discovery's bounds: cooldown, cap, expiry and cancel. The first
//! case is ported from `bacnet-network`'s `router/rb06_convergence_tests.rs`.

use std::panic::{AssertUnwindSafe, catch_unwind};

use bacnet_router_core::discovery::Discovery;
use bacnet_router_core::router::Options;
use bacnet_router_core::{Duration, Instant};

const BASE: Instant = Instant::from_secs(1_000);
const NANO: Duration = Duration::from_nanos(1);

const fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// [`Discovery::should_solicit`] with the hosted router's cooldown.
fn solicit<const D: usize>(discovery: &mut Discovery<D>, network: u16, now: Instant) -> bool {
    discovery.should_solicit(network, now, Options::HOSTED.discovery_cooldown)
}

/// Every network [`Discovery::expire`] forgets with the hosted router's age.
fn expire<const D: usize>(discovery: &mut Discovery<D>, now: Instant) -> Vec<u16> {
    let mut expired = Vec::new();
    let count = discovery.expire(now, Options::HOSTED.discovery_age, |network| {
        expired.push(network)
    });
    assert_eq!(count, expired.len());
    expired
}

#[test]
fn discovery_coalesces_rate_limits_expires_and_caps() {
    let mut disc: Discovery<256> = Discovery::new();
    assert!(solicit(&mut disc, 3000, BASE));
    assert!(!solicit(&mut disc, 3000, BASE + secs(1)));
    assert_eq!(disc.len(), 1);
    assert!(solicit(&mut disc, 3000, BASE + secs(5)));
    assert!(!solicit(&mut disc, 0, BASE));
    assert!(!solicit(&mut disc, 0xFFFF, BASE));
    for net in 4000..(4000 + 255) {
        assert!(solicit(&mut disc, net, BASE));
    }
    assert_eq!(disc.len(), 256);
    assert!(!solicit(&mut disc, 9999, BASE));
    assert_eq!(expire(&mut disc, BASE + secs(36)).len(), 256);
    assert_eq!(disc.len(), 0);
    assert!(solicit(&mut disc, 9999, BASE + secs(37)));
    disc.cancel();
    assert!(disc.is_cancelled());
    assert_eq!(disc.len(), 0);
    assert!(!solicit(&mut disc, 3000, BASE + secs(100)));
}

#[test]
fn the_cooldown_and_age_are_boundaries() {
    let mut disc: Discovery<4> = Discovery::default();
    assert!(disc.is_empty());
    assert!(solicit(&mut disc, 9, BASE));
    // Inside the cooldown, nothing moves; at its end, the search restarts.
    assert!(!solicit(&mut disc, 9, BASE + secs(5) - NANO));
    assert_eq!(disc.last_solicited(9), Some(BASE));
    assert!(solicit(&mut disc, 9, BASE + secs(5)));
    assert_eq!(disc.last_solicited(9), Some(BASE + secs(5)));
    // The age counts from the latest search, and the age itself still keeps.
    assert!(expire(&mut disc, BASE + secs(35)).is_empty());
    assert_eq!(expire(&mut disc, BASE + secs(35) + NANO), [9]);
    assert_eq!(disc.last_solicited(9), None);
}

#[test]
fn expiry_reports_in_ascending_order_and_keeps_the_rest() {
    let mut disc: Discovery<4> = Discovery::new();
    for (network, at) in [(30, 0), (10, 0), (20, 20), (40, 0)] {
        assert!(solicit(&mut disc, network, BASE + secs(at)));
    }
    assert!(!solicit(&mut disc, 50, BASE));
    assert_eq!(expire(&mut disc, BASE + secs(31)), [10, 30, 40]);
    assert_eq!(disc.len(), 1);
    assert_eq!(disc.last_solicited(20), Some(BASE + secs(20)));
    // Room again for another network.
    assert!(solicit(&mut disc, 50, BASE + secs(31)));
}

#[test]
fn the_timing_comes_with_each_call() {
    let mut disc: Discovery<4> = Discovery::new();
    assert!(disc.should_solicit(9, BASE, secs(1)));
    assert!(disc.should_solicit(9, BASE + secs(1), secs(1)));
    assert!(!disc.should_solicit(9, BASE + secs(2), secs(5)));
    let mut expired = Vec::new();
    disc.expire(BASE + secs(3), secs(2), |network| expired.push(network));
    assert!(expired.is_empty());
    disc.expire(BASE + secs(3) + NANO, secs(2), |network| {
        expired.push(network)
    });
    assert_eq!(expired, [9]);
    assert_eq!(Options::HOSTED.discovery_cooldown, secs(5));
    assert_eq!(Options::HOSTED.discovery_age, secs(30));
}

#[test]
fn a_panicking_expiry_callback_forgets_nothing() {
    // 10 and 30 are old at the sweep and 20 isn't, so forgetting 10 would
    // move 20 down before the call for 30 panics.
    let mut disc: Discovery<4> = Discovery::new();
    let searches = [(10, BASE), (20, BASE + secs(20)), (30, BASE)];
    for (network, at) in searches {
        assert!(solicit(&mut disc, network, at));
    }
    let unwound = catch_unwind(AssertUnwindSafe(|| {
        disc.expire(BASE + secs(31), Options::HOSTED.discovery_age, |network| {
            if network == 30 {
                panic!("callback");
            }
        })
    }));
    assert!(unwound.is_err());
    assert_eq!(disc.len(), 3);
    for (network, at) in searches {
        assert_eq!(disc.last_solicited(network), Some(at));
    }
    assert_eq!(expire(&mut disc, BASE + secs(31)), [10, 30]);
    assert_eq!(disc.len(), 1);
}

#[test]
fn debug_shows_the_searches_only() {
    let mut disc: Discovery<64> = Discovery::new();
    assert_eq!(format!("{disc:?}").matches("Entry").count(), 0);
    assert!(solicit(&mut disc, 9, BASE));
    let text = format!("{disc:?}");
    assert_eq!(text.matches("Entry").count(), 1, "{text}");
    assert!(text.contains("network: 9"), "{text}");
}
