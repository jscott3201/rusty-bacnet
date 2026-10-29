//! The device schedulers (#2115): batched polls, the backoffs, retries and
//! their cancels, the missing-device choice, what a point reads, the
//! sample stream, and the notification match a routed device relies on.
//! Several port the TUI watch list's scheduler tests.

use std::sync::atomic::Ordering;
use std::time::Duration;

use bacnet_client::client::{COVNotificationDelivery, CovTarget, ReceivedCOVNotification};
use bacnet_services::cov::COVNotificationRequest;
use bacnet_types::MacAddr;
use bacnet_types::constructed::BACnetPropertyValue;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;
use tokio::time::Instant;

use super::test_lab::{LAB, Lab, RP, RPM, SUBSCRIBE_COV, ai, nc, object, oid, settle};
use super::tests::present_value;
use super::*;

fn retrying() -> WatchOptions {
    WatchOptions {
        retry_cov: Backoff {
            first: Duration::from_secs(30),
            longest: Duration::from_secs(300),
        },
        ..WatchOptions::default()
    }
}

/// Acceptance: a device's polled points are read together, one RPM a
/// poll however many points it has, and every poll reaches the samples.
#[tokio::test(start_paused = true)]
async fn polled_points_of_a_device_share_one_request() {
    let lab = Lab::start(WatchOptions::default()).await;
    let leases: Vec<_> = (1..=3)
        .map(|n| lab.hub.watch(object(nc(n))).unwrap())
        .collect();
    let mut samples = leases[0].samples();
    settle().await;
    for lease in &leases {
        assert_eq!(lease.snapshot().source, Some(Source::Poll));
    }
    let from = Instant::now();
    tokio::time::sleep(Duration::from_secs(35)).await;
    // The first reads stood in for a poll; polls went at 10, 20 and 30 s.
    assert_eq!(lab.sent(RPM, from), 3, "{:?}", lab.services());
    for lease in &leases {
        assert_eq!(lease.snapshot().polls, 3);
        assert_eq!(lease.snapshot().state, State::Live);
    }
    let vias: Vec<Via> = std::iter::from_fn(|| samples.try_next())
        .map(|s| s.via)
        .collect();
    assert_eq!(vias, [Via::First, Via::Poll, Via::Poll, Via::Poll]);
    lab.stop().await;
}

/// A batch whose answer would be too large is asked again a row at a time.
#[tokio::test(start_paused = true)]
async fn a_batch_too_large_is_asked_a_row_at_a_time() {
    let lab = Lab::start(WatchOptions::default()).await;
    let leases: Vec<_> = (1..=3)
        .map(|n| lab.hub.watch(object(nc(n))).unwrap())
        .collect();
    settle().await;
    lab.quirks.abort_rpm.store(1, Ordering::Relaxed);
    let from = Instant::now();
    tokio::time::sleep(Duration::from_secs(11)).await;
    assert_eq!(lab.sent(RPM, from), 1 + 3, "{:?}", lab.services());
    assert_eq!(lab.sent(RP, from), 0);
    for lease in &leases {
        assert_eq!(lease.snapshot().polls, 1);
        assert_eq!(lease.snapshot().state, State::Live);
    }
    lab.stop().await;
}

/// A device that refuses RPM is read by ReadProperty, remembered by the
/// engine; under [`Reads::value`] a poll reads only the watched property.
#[tokio::test(start_paused = true)]
async fn a_device_refusing_rpm_is_polled_by_readproperty() {
    let options = WatchOptions {
        reads: Reads::value(),
        ..WatchOptions::default()
    };
    let lab = Lab::start(options).await;
    lab.quirks.refuse_rpm.store(true, Ordering::Relaxed);
    let _leases: Vec<_> = (1..=2)
        .map(|n| lab.hub.watch(object(nc(n))).unwrap())
        .collect();
    settle().await;
    assert!(lab.engine.refused_rpm(LAB));
    let from = Instant::now();
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(lab.sent(RPM, from), 0, "{:?}", lab.services());
    assert_eq!(lab.sent(RP, from), 2, "one property a point");
    lab.stop().await;
}

/// A device that fails a whole batch for one bad object: that poll reads
/// each point's properties one at a time, and RPM is tried again next.
#[tokio::test(start_paused = true)]
async fn a_failed_batch_is_read_one_property_at_a_time() {
    let lab = Lab::start(WatchOptions::default()).await;
    let leases: Vec<_> = (1..=3)
        .map(|n| lab.hub.watch(object(nc(n))).unwrap())
        .collect();
    settle().await;
    lab.quirks.fail_rpm.store(true, Ordering::Relaxed);
    let from = Instant::now();
    tokio::time::sleep(Duration::from_secs(11)).await;
    assert_eq!(lab.sent(RPM, from), 1);
    assert_eq!(lab.sent(RP, from), 3 * POINT_PROPERTIES.len());
    assert!(!lab.engine.refused_rpm(LAB));
    for lease in &leases {
        let snapshot = lease.snapshot();
        assert_eq!((snapshot.polls, snapshot.state), (1, State::Live));
    }
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(lab.sent(RPM, from), 2, "RPM again at the next poll");
    lab.stop().await;
}

/// A device that stops answering is polled after a wait that doubles,
/// per device, not every interval (port of the TUI's silent-device test).
#[tokio::test(start_paused = true)]
async fn a_silent_device_is_polled_with_a_growing_wait() {
    let options = WatchOptions {
        poll_every: Duration::from_secs(1),
        poll_floor: Duration::from_secs(1),
        poll_backoff: Backoff {
            first: Duration::from_secs(2),
            longest: Duration::from_secs(60),
        },
        ..WatchOptions::default()
    };
    let lab = Lab::start(options).await;
    let leases: Vec<_> = (1..=2)
        .map(|n| lab.hub.watch(object(nc(n))).unwrap())
        .collect();
    settle().await;
    lab.quirks.muted.store(true, Ordering::Relaxed);
    let from = Instant::now();
    tokio::time::sleep(Duration::from_secs(300)).await;
    // Waits of 2, 4, ... 60 s after each 1 s timeout: about ten polls,
    // each one RPM for both points, where an interval would make 150.
    let polls = lab.sent(RPM, from);
    assert!((6..=12).contains(&polls), "{polls} polls in 300 s");
    for lease in &leases {
        assert_eq!(lease.snapshot().state, State::Unreachable);
    }
    lab.quirks.muted.store(false, Ordering::Relaxed);
    lab.stop().await;
}

/// An unanswered first subscription is retried after the COV backoff (30
/// s here), reading the point again, and then holds one subscription that
/// a release cancels.
#[tokio::test(start_paused = true)]
async fn an_unanswered_subscription_is_retried_after_its_backoff() {
    let lab = Lab::start(retrying()).await;
    lab.quirks.swallow_cov.store(1, Ordering::Relaxed);
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let snapshot = lease.snapshot();
    assert_eq!(snapshot.source, Some(Source::Poll), "{snapshot:?}");
    assert_eq!(snapshot.state, State::Unreachable);
    assert!(
        matches!(snapshot.why, Some(Why::Refused(_))),
        "{snapshot:?}"
    );
    assert!(snapshot.note.unwrap().starts_with("COV refused"));
    assert!(lab.table().await.is_empty());
    tokio::time::sleep(Duration::from_secs(25)).await;
    assert_eq!(
        lab.sent(SUBSCRIBE_COV, Instant::now() - Duration::from_secs(60)),
        1
    );
    tokio::time::sleep(Duration::from_secs(6)).await;
    let snapshot = lease.snapshot();
    assert_eq!(snapshot.source, Some(Source::Cov), "{snapshot:?}");
    assert_eq!(snapshot.state, State::Live);
    assert_eq!(snapshot.note, None);
    assert_eq!(snapshot.why, None, "subscribed: nothing to explain");
    assert_eq!(lab.table().await.len(), 1);
    // Polled meanwhile; the retry read the point again first.
    let services = lab.services();
    assert_eq!(services.iter().filter(|s| **s == SUBSCRIBE_COV).count(), 2);
    assert_eq!(&services[services.len() - 2..], [RPM, SUBSCRIBE_COV]);
    assert!(
        services.iter().filter(|s| **s == RPM).count() >= 3,
        "{services:?}"
    );
    drop(lease);
    settle().await;
    assert!(lab.table().await.is_empty());
    assert_eq!(lab.hub.stats().tasks, 0);
    lab.stop().await;
}

/// A point released while it waits to retry still cancels the subscription
/// its unanswered request may have made, and the cancel is counted while
/// it runs (port of the TUI's queued-retry removal).
#[tokio::test(start_paused = true)]
async fn a_release_during_a_retry_cancels_what_may_exist() {
    let lab = Lab::start(retrying()).await;
    lab.quirks.swallow_cov.store(1, Ordering::Relaxed);
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    lab.quirks.muted.store(true, Ordering::Relaxed);
    drop(lease);
    settle().await;
    let stats = lab.hub.stats();
    assert_eq!((stats.cancelling, stats.tasks), (1, 1), "{stats:?}");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let stats = lab.hub.stats();
    assert_eq!((stats.cancelling, stats.tasks), (0, 0), "{stats:?}");
    assert_eq!(lab.services(), [RPM, SUBSCRIBE_COV, SUBSCRIBE_COV]);
    lab.quirks.muted.store(false, Ordering::Relaxed);
    lab.stop().await;
}

/// A point released while its start runs is cancelled once the start
/// ends, the subscription it just made included (port of the TUI's
/// in-flight removal).
#[tokio::test(start_paused = true)]
async fn a_release_during_the_start_cancels_after_it() {
    let lab = Lab::start(WatchOptions::default()).await;
    lab.quirks.slow_cov_ack.store(true, Ordering::Relaxed);
    let lease = lab.hub.watch(object(ai(1))).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        lab.services(),
        [RPM, SUBSCRIBE_COV],
        "the answer is still on its way"
    );
    assert_eq!(lab.table().await.len(), 1);
    drop(lease);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(lab.table().await.is_empty());
    assert_eq!(lab.services(), [RPM, SUBSCRIBE_COV, SUBSCRIBE_COV]);
    assert_eq!(lab.hub.stats().tasks, 0);
    lab.stop().await;
}

/// A refusal stops the retries: one SubscribeCOV however long the point
/// is watched.
#[tokio::test(start_paused = true)]
async fn a_refusal_is_not_retried() {
    let lab = Lab::start(retrying()).await;
    let lease = lab.hub.watch(object(nc(1))).unwrap();
    tokio::time::sleep(Duration::from_secs(1_000)).await;
    assert_eq!(
        lab.sent(SUBSCRIBE_COV, Instant::now() - Duration::from_secs(2_000)),
        1
    );
    assert_eq!(lease.snapshot().source, Some(Source::Poll));
    lab.stop().await;
}

/// Under [`MissingDevice::Fail`] a point whose device isn't in the table
/// is given up: nothing is sent, then or later.
#[tokio::test(start_paused = true)]
async fn a_missing_device_can_be_given_up() {
    let options = WatchOptions {
        missing: MissingDevice::Fail,
        ..WatchOptions::default()
    };
    let lab = Lab::start(options).await;
    let point = Point {
        device: 2002,
        ..object(ai(1))
    };
    let lease = lab.hub.watch(point).unwrap();
    settle().await;
    let snapshot = lease.snapshot();
    assert_eq!(snapshot.state, State::Gone);
    assert!(snapshot.note.unwrap().contains("device table"));
    lab.engine.client().add_device(2002, &[2]).await.unwrap();
    tokio::time::sleep(Duration::from_secs(120)).await;
    assert!(lab.services().is_empty(), "{:?}", lab.services());
    assert_eq!(lease.snapshot().polls, 0);
    lab.stop().await;
}

/// What a point reads: [`Reads::value`] asks for the value, Status_Flags,
/// Units and the name in one RPM, then a binary object's state texts.
#[tokio::test(start_paused = true)]
async fn the_value_reads_name_units_and_state_texts_first() {
    let options = WatchOptions {
        reads: Reads::value(),
        ..WatchOptions::default()
    };
    let lab = Lab::start(options).await;
    let bv = lab
        .hub
        .watch(object(oid(ObjectType::BINARY_VALUE, 1)))
        .unwrap();
    let analog = lab.hub.watch(object(ai(1))).unwrap();
    settle().await;
    assert_eq!(
        lab.services(),
        [RPM, RPM, SUBSCRIBE_COV, RPM, SUBSCRIBE_COV],
        "texts for the binary object only"
    );
    let has = |lease: &Lease, property| lease.snapshot().rows.iter().any(|(p, _)| *p == property);
    for property in [
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::STATUS_FLAGS,
        PropertyIdentifier::UNITS,
        PropertyIdentifier::OBJECT_NAME,
        PropertyIdentifier::ACTIVE_TEXT,
        PropertyIdentifier::INACTIVE_TEXT,
    ] {
        assert!(has(&bv, property), "{property:?}");
    }
    assert!(!has(&analog, PropertyIdentifier::ACTIVE_TEXT));
    assert!(
        !has(&analog, PropertyIdentifier::RELIABILITY),
        "only what it reads"
    );
    // The hub's own process identifiers name its subscriptions.
    let mut processes: Vec<u32> = lab
        .table()
        .await
        .iter()
        .map(|entry| entry.recipient.process_identifier)
        .collect();
    processes.sort_unstable();
    assert_eq!(processes, [PROCESS_BASE, PROCESS_BASE + 1]);
    lab.stop().await;
}

/// Every notification reaches the samples though the signal coalesces
/// them, and a reader that falls behind loses the oldest, counted.
#[tokio::test(start_paused = true)]
async fn every_notification_reaches_the_samples() {
    let options = WatchOptions {
        samples: 4,
        ..WatchOptions::default()
    };
    let lab = Lab::start(options).await;
    let lease = lab.hub.watch(object(ai(2))).unwrap();
    settle().await;
    let mut kept = lease.samples();
    let mut slow = lease.samples();
    for step in 0..10u8 {
        lab.server
            .set_present_value_local(&ai(2), PropertyValue::Real(f32::from(step) + 20.0))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        if step < 4 {
            while kept.try_next().is_some() {}
        }
    }
    let mut kept_count = 4;
    while let Some(sample) = kept.try_next() {
        assert_eq!(sample.via, Via::Notification);
        kept_count += 1;
    }
    // Four read early, four held, two lost: ten notifications.
    assert_eq!((kept_count, kept.dropped()), (8, 2));
    let mut last = None;
    let mut count = 0;
    while let Some(sample) = slow.try_next() {
        last = Some(sample);
        count += 1;
    }
    assert_eq!((count, slow.dropped()), (4, 6));
    let rows = last.unwrap().values.unwrap();
    let pv = rows
        .iter()
        .find(|(p, _)| *p == PropertyIdentifier::PRESENT_VALUE)
        .unwrap();
    assert_eq!(
        crate::format::decode_all(pv.1.as_ref().unwrap()).unwrap(),
        [PropertyValue::Real(29.0)]
    );
    assert!(lease.snapshot().coalesced > 0);
    lab.stop().await;
}

/// A coalescing window of zero signals every change.
#[tokio::test(start_paused = true)]
async fn a_zero_window_signals_every_change() {
    let options = WatchOptions {
        coalesce: Duration::ZERO,
        ..WatchOptions::default()
    };
    let lab = Lab::start(options).await;
    let mut lease = lab.hub.watch(object(ai(3))).unwrap();
    settle().await;
    lease.changed().await;
    for step in 0..5u8 {
        lab.server
            .set_present_value_local(&ai(3), PropertyValue::Real(f32::from(step) + 50.0))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
        let signalled = tokio::time::timeout(Duration::ZERO, lease.changed()).await;
        assert!(signalled.is_ok(), "change {step} signalled");
    }
    assert_eq!(present_value(&lease.snapshot()), Some(54.0));
    assert_eq!(lease.snapshot().coalesced, 0);
    lab.stop().await;
}

#[test]
fn the_settings_keep_their_bounds() {
    let defaults = WatchOptions::default();
    assert!(defaults.validate().is_ok());
    assert_eq!(defaults.poll_every, MIN_POLL);
    assert_eq!(defaults.poll_floor, MIN_POLL);
    assert_eq!(defaults.retry_cov, Backoff::fixed(Duration::from_secs(300)));
    assert_eq!(defaults.missing, MissingDevice::Poll);
    assert_eq!(defaults.reads, Reads::points());
    let fast = WatchOptions {
        poll_every: Duration::from_secs(1),
        poll_floor: Duration::from_secs(1),
        ..WatchOptions::default()
    };
    assert!(fast.validate().is_ok(), "a front end may poll every second");
    let under = WatchOptions {
        poll_every: Duration::from_millis(500),
        poll_floor: Duration::from_millis(500),
        ..WatchOptions::default()
    };
    assert!(under.validate().is_err(), "never under a second");
    let no_wait = WatchOptions {
        poll_backoff: Backoff::fixed(Duration::ZERO),
        ..WatchOptions::default()
    };
    assert!(no_wait.validate().is_err());
    let eager = WatchOptions {
        retry_cov: Backoff::fixed(Duration::from_secs(5)),
        ..WatchOptions::default()
    };
    assert!(
        eager.validate().is_err(),
        "no retry sooner than the poll floor"
    );
    for samples in [0, MAX_SAMPLES + 1] {
        let options = WatchOptions {
            samples,
            ..WatchOptions::default()
        };
        assert!(options.validate().is_err(), "{samples}");
    }
    let empty = WatchOptions {
        reads: Reads {
            object: Vec::new(),
            ..Reads::points()
        },
        ..WatchOptions::default()
    };
    assert!(empty.validate().is_err());
}

/// The COV retry backs off to its cap (port of the TUI's backoff test).
#[test]
fn backoffs_double_to_their_cap() {
    let backoff = Backoff {
        first: Duration::from_secs(30),
        longest: Duration::from_secs(300),
    };
    let mut wait = Duration::ZERO;
    let waits: Vec<u64> = (0..6)
        .map(|_| {
            wait = backoff.next(wait);
            wait.as_secs()
        })
        .collect();
    assert_eq!(waits, [30, 60, 120, 240, 300, 300]);
    let fixed = Backoff::fixed(Duration::from_secs(300));
    assert_eq!(fixed.next(Duration::from_secs(300)).as_secs(), 300);
}

/// The renewal margin covers a gate wait and a renewal within half the
/// lifetime (port of the TUI's margin test).
#[test]
fn the_margin_covers_a_gate_wait_and_a_renewal_within_half_the_lifetime() {
    let budget = Duration::from_secs(24);
    assert_eq!(renewal_margin(300, budget), budget * 2);
    assert_eq!(
        renewal_margin(600, Duration::from_secs(10)),
        Duration::from_secs(30)
    );
    assert_eq!(renewal_margin(120, budget), budget * 2);
    assert_eq!(renewal_margin(80, budget), Duration::from_secs(40));
    assert_eq!(renewal_margin(30, budget), Duration::from_secs(15));
    assert_eq!(renewal_margin(60, Duration::ZERO), Duration::from_secs(6));
}

/// A notification from `source` (its MAC, and network and address when
/// routed) by device `instance` about AI 1.
fn from(instance: u32, mac: &[u8], routed: Option<(u16, &[u8])>) -> ReceivedCOVNotification {
    ReceivedCOVNotification {
        notification: COVNotificationRequest {
            subscriber_process_identifier: PROCESS_BASE,
            initiating_device_identifier: oid(ObjectType::DEVICE, instance),
            monitored_object_identifier: ai(1),
            time_remaining: 100,
            list_of_values: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: vec![0x44, 0x42, 0x28, 0, 0],
                priority: None,
            }],
        },
        source_mac: MacAddr::from_slice(mac),
        source_network: routed.map(|(network, _)| network),
        source_address: routed.map(|(_, address)| MacAddr::from_slice(address)),
        delivery: COVNotificationDelivery::Unconfirmed,
    }
}

/// The TUI matches a notification to its row by the initiating device's
/// instance; the hub by where it came from. For PR 2 to rest on the hub's
/// match: every case the instance match takes for a routed device, the
/// address match takes too (through its router, or sent as a local one on
/// the client's own network), and what the instance match rejects (another
/// device's report, another object) it rejects. The hub is stricter where
/// the instance alone would accept a report from the wrong address.
#[test]
fn a_routed_device_is_matched_by_its_address() {
    const DEVICE: u32 = 2001;
    let point = Point {
        device: DEVICE,
        ..object(ai(1))
    };
    let target = CovTarget::routed(&[2], 10, &[3]);
    let belongs =
        |received: &ReceivedCOVNotification| dispatch::belongs(&point, Some(&target), received);
    // Through its router, as the instance match would take it.
    assert!(belongs(&from(DEVICE, &[2], Some((10, &[3])))));
    // On the client's own network, sending from its address as a MAC.
    assert!(belongs(&from(DEVICE, &[3], None)));
    // Another device behind the same router: another address.
    assert!(!belongs(&from(2002, &[2], Some((10, &[4])))));
    // The same address on another network.
    assert!(!belongs(&from(DEVICE, &[2], Some((11, &[3])))));
    // Another object of the right device.
    let mut other = from(DEVICE, &[2], Some((10, &[3])));
    other.notification.monitored_object_identifier = ai(2);
    assert!(!belongs(&other));
    // Stricter than the instance: the right instance from a wrong address.
    assert!(!belongs(&from(DEVICE, &[9], Some((10, &[9])))));
    // No address looked up yet: nothing matches.
    assert!(!dispatch::belongs(
        &point,
        None,
        &from(DEVICE, &[2], Some((10, &[3])))
    ));
}

/// Under [`Reads::value`] a property watch keeps the Status_Flags a
/// notification carries beside its property, so its samples carry the
/// device's flags; under the MCP's reads it keeps the property alone.
#[tokio::test(start_paused = true)]
async fn a_property_watch_keeps_notified_flags_when_it_reads_them() {
    for (reads, flags) in [(Reads::value(), true), (Reads::points(), false)] {
        let options = WatchOptions {
            reads,
            ..WatchOptions::default()
        };
        let lab = Lab::start(options).await;
        let point = Point {
            watched: Watched::Property {
                property: PropertyIdentifier::PRESENT_VALUE,
                index: None,
            },
            ..object(ai(3))
        };
        let lease = lab.hub.watch(point).unwrap();
        settle().await;
        let mut samples = lease.samples();
        lab.server
            .set_present_value_local(&ai(3), PropertyValue::Real(33.0))
            .await
            .unwrap();
        settle().await;
        let sample = samples.try_next().expect("notified");
        assert_eq!(sample.via, Via::Notification);
        let properties: Vec<PropertyIdentifier> =
            sample.values.unwrap().iter().map(|(p, _)| *p).collect();
        let expected = if flags {
            vec![
                PropertyIdentifier::PRESENT_VALUE,
                PropertyIdentifier::STATUS_FLAGS,
            ]
        } else {
            vec![PropertyIdentifier::PRESENT_VALUE]
        };
        assert_eq!(properties, expected);
        drop(lease);
        lab.stop().await;
    }
}
