use super::*;
use std::time::Duration;

fn policy() -> ScHubBroadcastRatePolicy {
    ScHubBroadcastRatePolicy {
        sender_burst: 3,
        sender_per_second: 2,
        global_burst: 5,
        global_per_second: 4,
    }
}

#[test]
fn bounds_reject_zero_and_overflow_in_every_field() {
    for index in 0..4 {
        for invalid in [0, u64::MAX / TOKEN + 1, u64::MAX] {
            let mut p = policy();
            let fields = [
                &mut p.sender_burst,
                &mut p.sender_per_second,
                &mut p.global_burst,
                &mut p.global_per_second,
            ];
            *fields.into_iter().nth(index).unwrap() = invalid;
            assert!(matches!(HubBudget::new(p), Err(Error::Encoding(message))
                if message.starts_with("hub broadcast ")));
        }
    }
    let maximum = u64::MAX / TOKEN;
    assert!(HubBudget::new(ScHubBroadcastRatePolicy {
        sender_burst: maximum,
        sender_per_second: maximum,
        global_burst: maximum,
        global_per_second: maximum,
    })
    .is_ok());
    assert_eq!(
        ScHubBroadcastRatePolicy::default(),
        ScHubBroadcastRatePolicy {
            sender_burst: 1024,
            sender_per_second: 128,
            global_burst: 4096,
            global_per_second: 512,
        }
    );
}

#[test]
fn bucket_exact_burst_fractional_refill_and_idle_cap() {
    let now = Instant::now();
    let mut bucket = Bucket::new(3, 2, now);
    for _ in 0..3 {
        assert!(bucket.take(now));
    }
    assert!(!bucket.take(now));
    // Rejected requests preserve fractional credit rather than restart refill.
    for millis in 1..500 {
        assert!(!bucket.take(now + Duration::from_millis(millis)));
    }
    assert!(!bucket.take(now + Duration::from_millis(500) - Duration::from_nanos(1)));
    assert!(bucket.take(now + Duration::from_millis(500)));
    assert!(!bucket.take(now + Duration::from_millis(500)));
    assert!(bucket.take(now + Duration::from_secs(1)));
    let later = now + Duration::from_secs(86_400);
    for _ in 0..3 {
        assert!(bucket.take(later));
    }
    assert!(!bucket.take(later));
    // A stale concurrent sample must not move the global bucket clock backward.
    assert!(!bucket.take(now));
    assert!(!bucket.take(later));
}

#[test]
fn nondivisor_refill_preserves_nanotoken_remainders() {
    let now = Instant::now();
    let mut bucket = Bucket::new(1, 3, now);
    assert!(bucket.take(now));
    assert!(!bucket.take(now + Duration::from_nanos(333_333_333)));
    assert!(bucket.take(now + Duration::from_nanos(333_333_334)));
    assert!(!bucket.take(now + Duration::from_nanos(666_666_666)));
    // The first refill hit the capacity ceiling and discarded two nanotokens.
    assert!(!bucket.take(now + Duration::from_nanos(666_666_667)));
    assert!(bucket.take(now + Duration::from_nanos(666_666_668)));
}

#[test]
fn maximum_arithmetic_stays_bounded_after_long_idle() {
    let now = Instant::now();
    let maximum = u64::MAX / TOKEN;
    let mut bucket = Bucket::new(maximum, maximum, now);
    assert!(bucket.take(now));
    assert!(bucket.take(now + Duration::from_secs(1_000_000)));
    assert_eq!(bucket.credit, (maximum - 1) * TOKEN);
}

#[test]
fn sender_isolation_global_accounting_and_unicast_bypass() {
    let hub = Arc::new(HubBudget::new(policy()).unwrap());
    let now = Instant::now();
    let mut a = SenderBudget::new(hub.clone(), now);
    let mut b = SenderBudget::new(hub.clone(), now);
    for _ in 0..3 {
        assert!(a.admit_at(HubRelayTarget::Broadcast, now));
    }
    for _ in 0..1000 {
        assert!(!a.admit_at(HubRelayTarget::Broadcast, now));
    }
    for _ in 0..2 {
        assert!(b.admit_at(HubRelayTarget::Broadcast, now));
    }
    assert!(!b.admit_at(HubRelayTarget::Broadcast, now));
    assert!(!b.admit_at(HubRelayTarget::Broadcast, now));
    for _ in 0..1000 {
        assert!(a.admit_at(HubRelayTarget::Unicast([42; 6]), now));
        assert!(b.admit_at(HubRelayTarget::Unicast([43; 6]), now));
    }
    assert_eq!(
        hub.drop_counts(),
        ScHubBroadcastDropCounts {
            sender_exhausted: 1001,
            global_exhausted: 1,
        }
    );
    // Neither failed global admissions nor per-sender floods borrow future tokens.
    assert!(a.admit_at(HubRelayTarget::Broadcast, now + Duration::from_millis(500)));
    assert!(b.admit_at(HubRelayTarget::Broadcast, now + Duration::from_millis(500)));
}

#[test]
fn concurrent_senders_share_one_fixed_global_bucket() {
    let hub = Arc::new(
        HubBudget::new(ScHubBroadcastRatePolicy {
            sender_burst: 100,
            sender_per_second: 1,
            global_burst: 37,
            global_per_second: 1,
        })
        .unwrap(),
    );
    let now = Instant::now();
    let admitted: usize = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..16)
            .map(|_| {
                let hub = hub.clone();
                scope.spawn(move || {
                    let mut sender = SenderBudget::new(hub, now);
                    (0..100)
                        .filter(|_| sender.admit_at(HubRelayTarget::Broadcast, now))
                        .count()
                })
            })
            .collect();
        threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .sum()
    });
    assert_eq!(admitted, 37);
    assert_eq!(
        hub.drop_counts(),
        ScHubBroadcastDropCounts {
            sender_exhausted: 0,
            global_exhausted: 1600 - 37,
        }
    );
}

#[test]
fn drop_counters_saturate_without_affecting_admission() {
    let hub = Arc::new(
        HubBudget::new(ScHubBroadcastRatePolicy {
            sender_burst: 2,
            global_burst: 1,
            ..policy()
        })
        .unwrap(),
    );
    hub.sender_drops.store(u64::MAX - 1, Ordering::Relaxed);
    hub.global_drops.store(u64::MAX - 1, Ordering::Relaxed);
    let now = Instant::now();
    for connection in 0..3 {
        let mut sender = SenderBudget::new(hub.clone(), now);
        assert_eq!(
            sender.admit_at(HubRelayTarget::Broadcast, now),
            connection == 0
        );
        for _ in 0..4 {
            assert!(!sender.admit_at(HubRelayTarget::Broadcast, now));
        }
    }
    assert_eq!(
        hub.drop_counts(),
        ScHubBroadcastDropCounts {
            sender_exhausted: u64::MAX,
            global_exhausted: u64::MAX,
        }
    );
    let mut sender = SenderBudget::new(hub, now);
    assert!(sender.admit_at(HubRelayTarget::Broadcast, now + Duration::from_secs(1)));
}
