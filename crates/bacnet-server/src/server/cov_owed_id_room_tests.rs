//! References owed an invoke ID (#2661) that meet an in-flight limit at
//! their look: they keep their places and go as the peer's (or the pool's)
//! reports end, without a reservation tried while there is no room, and a
//! full peer doesn't hold back another peer's references. Also: a split
//! COV-multiple report refused an ID owes its whole context, and a lease or
//! a slot coming back between a pass and its wait still ends the wait.
use super::cov_fanout::owed_gap;
use super::cov_owed_id_tests::{lease_all, owed, owed_counts, pv_of, refusals, subscribed_on};
use super::cov_wire_test_support::*;
use super::warn_spy::WarnSpy;
use super::*;
use bacnet_device_core::cov::{CovNotificationKind, CovPolicy, CovRecipient, CovSubscription};
use bacnet_objects::analog::AnalogValueObject;
use bacnet_types::enums::EngineeringUnits;
use std::sync::atomic::Ordering::Relaxed;

/// A harness whose subscriber may have `per_peer` confirmed notifications
/// in flight.
async fn with_peer_limit(per_peer: usize) -> Harness {
    Harness::start(ServerConfig {
        cov_policy: CovPolicy {
            max_confirmed_in_flight_per_peer: per_peer,
            ..CovPolicy::default()
        },
        ..ServerConfig::default()
    })
    .await
}

/// Reports refused by the peer's in-flight limit so far.
fn throttled(h: &Harness) -> u64 {
    h.server
        .cov_counters
        .notifications_throttled_peer
        .load(Relaxed)
}

/// The harness subscriber's peer, as its in-flight slots count it.
fn peer() -> CovRecipient {
    CovRecipient::from_endpoint(&PEER, None)
}

/// The server's in-flight tracker.
async fn tracker(h: &Harness) -> Arc<crate::cov::CovInFlightTracker> {
    Arc::clone(h.server.cov_table.read().await.in_flight_tracker())
}

/// Two references of one peer owed, the peer allowed one report in flight:
/// once an ID frees, the first goes; a second ID finds the peer full, so
/// the other waits in its place and goes when the first is acknowledged.
#[tokio::test(start_paused = true)]
async fn a_look_that_finds_its_peer_full_keeps_its_wait() {
    let mut h = with_peer_limit(1).await;
    subscribed_on(&mut h, 1, av1()).await;
    subscribed_on(&mut h, 2, av1()).await;
    let mut held = lease_all(&h);
    h.write_local(42.0).await;
    assert_eq!(owed(&h).await.len(), 2);
    drop(held.pop());
    assert_eq!(h.cov_notification().await.subscriber_process_identifier, 1);
    drop(held.pop());
    tokio::time::sleep(Duration::from_secs(1)).await;
    h.no_notification().await;
    assert_eq!(owed(&h).await.len(), 1, "process 2 keeps its place");

    h.ack().await;
    let second = h.cov_notification().await;
    assert_eq!(second.subscriber_process_identifier, 2);
    assert_eq!(pv_of(&second), real(42.0));
    assert_eq!(
        throttled(&h),
        0,
        "no reservation tried while the peer was full"
    );
    assert_eq!(owed_counts(&h), (2, 2, 0));
}

/// A look taken while its peer had room, refused because another report
/// took the peer's slot before its admission (here the test, while the
/// look waits for the database): the reference keeps its place and goes
/// when that report ends. Both forms.
#[tokio::test(start_paused = true)]
async fn a_look_refused_by_a_slot_taken_meanwhile_keeps_its_wait() {
    for multiple in [false, true] {
        let mut h = with_peer_limit(1).await;
        if multiple {
            h.subscribe_specs(true, vec![(av1(), vec![(PV, false)])])
                .await;
            h.notification().await;
            h.ack().await;
            h.settle().await;
        } else {
            subscribed_on(&mut h, 1, av1()).await;
        }
        let mut held = lease_all(&h);
        h.write_local(42.0).await;
        let db = h.server.db.write().await;
        drop(held.pop());
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(owed(&h).await.is_empty(), "taken for its look");
        let slot = tracker(&h)
            .await
            .try_acquire(peer(), 1, &h.server.cov_in_flight)
            .expect("the peer's slot");
        drop(db);
        tokio::time::sleep(Duration::from_secs(1)).await;
        h.no_notification().await;
        assert_eq!(owed(&h).await.len(), 1, "multiple: {multiple}");
        assert_eq!(throttled(&h), 1, "tried once, then waited for room");

        drop(slot);
        let value = if multiple {
            pv_rows(&h.notification().await)[0].0.clone()
        } else {
            pv_of(&h.cov_notification().await)
        };
        assert_eq!(value, real(42.0));
        h.server.stop().await.unwrap();
    }
}

/// A first send refused by the peer's in-flight limit is not queued: it
/// waits for its next fanout, a deliberate throttle (#533; whether it
/// should wait too is open on #2671). Ordinary form.
#[tokio::test(start_paused = true)]
async fn a_first_send_refused_by_the_peer_limit_is_not_owed() {
    let mut h = with_peer_limit(1).await;
    subscribed_on(&mut h, 1, av1()).await;
    let slot = tracker(&h)
        .await
        .try_acquire(peer(), 1, &h.server.cov_in_flight)
        .unwrap();
    h.write_local(42.0).await;
    h.settle().await;
    assert!(owed(&h).await.is_empty(), "a first send is not owed");
    assert_eq!(throttled(&h), 1);
    drop(slot);
    tokio::time::sleep(Duration::from_secs(1)).await;
    h.no_notification().await;
    assert_eq!(owed_counts(&h), (0, 0, 0));
}

/// The same for a confirmed COV-multiple context.
#[tokio::test(start_paused = true)]
async fn a_first_send_refused_by_the_peer_limit_is_not_owed_multiple() {
    let mut h = with_peer_limit(1).await;
    h.subscribe_specs(true, vec![(av1(), vec![(PV, false)])])
        .await;
    h.notification().await;
    h.ack().await;
    h.settle().await;
    let slot = tracker(&h)
        .await
        .try_acquire(peer(), 1, &h.server.cov_in_flight)
        .unwrap();
    h.write_local(42.0).await;
    h.settle().await;
    assert!(owed(&h).await.is_empty(), "a first send is not owed");
    assert_eq!(throttled(&h), 1);
    drop(slot);
    tokio::time::sleep(Duration::from_secs(1)).await;
    h.no_notification().await;
    assert_eq!(owed_counts(&h), (0, 0, 0));
}

/// A first send refused by the pool of 255 is not queued either.
#[tokio::test(start_paused = true)]
async fn a_first_send_refused_by_the_pool_is_not_owed() {
    let mut h = Harness::start(ServerConfig::default()).await;
    subscribed_on(&mut h, 1, av1()).await;
    let tracker = tracker(&h).await;
    let other = CovRecipient::from_endpoint(&[7], None);
    let mut pool: Vec<_> = (0..255)
        .map(|_| {
            tracker
                .try_acquire(other.clone(), usize::MAX, &h.server.cov_in_flight)
                .unwrap()
        })
        .collect();
    h.write_local(42.0).await;
    h.settle().await;
    assert!(owed(&h).await.is_empty());
    drop(pool.pop());
    tokio::time::sleep(Duration::from_secs(1)).await;
    h.no_notification().await;
    assert_eq!(owed_counts(&h), (0, 0, 0));
}

/// A wrong estimate of the room (here, one that admits everything) has
/// looks refused for want of a slot: each waits for room to come back
/// before the next pass, so the attempts stay bounded by what ends, and
/// the reference still goes.
#[tokio::test(start_paused = true)]
async fn a_look_refused_for_room_waits_for_room_before_the_next_pass() {
    let mut h = with_peer_limit(1).await;
    subscribed_on(&mut h, 1, av1()).await;
    subscribed_on(&mut h, 2, av1()).await;
    let mut held = lease_all(&h);
    h.write_local(42.0).await;
    owed_gap::overtake();
    drop(held.pop());
    drop(held.pop());
    // Yields, not sleeps, until the count is checked: a pass that retried
    // at once would keep paused time from advancing, and the test would
    // hang instead of failing here.
    for _ in 0..200 {
        tokio::task::yield_now().await;
    }
    assert_eq!(throttled(&h), 1, "refused once, then waits for room");
    assert_eq!(owed(&h).await.len(), 1);
    assert_eq!(h.cov_notification().await.subscriber_process_identifier, 1);

    // The Ack frees an ID, then the slot: at most a pass each.
    h.ack().await;
    let second = h.cov_notification().await;
    assert_eq!(second.subscriber_process_identifier, 2);
    assert_eq!(pv_of(&second), real(42.0));
    assert!(throttled(&h) <= 2, "{} attempts refused", throttled(&h));
}

/// Six references of one peer owed, two allowed in flight, every ID freed
/// at once: two go, and each Ack lets the next go, in the order owed. No
/// reservation is refused by the peer's limit, however long they wait.
#[tokio::test(start_paused = true)]
async fn a_peers_references_go_as_its_reports_end_in_the_order_owed() {
    let (spy, _guard) = WarnSpy::install();
    let mut h = with_peer_limit(2).await;
    for process in 1..=6 {
        subscribed_on(&mut h, process, av1()).await;
    }
    let held = lease_all(&h);
    h.write_local(42.0).await;
    assert_eq!(owed(&h).await.len(), 6);
    drop(held);
    for _ in 0..200 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        throttled(&h),
        0,
        "the look takes only what the peer has room for"
    );

    let mut served = Vec::new();
    for _ in 0..6 {
        let report = h.cov_notification().await;
        served.push(report.subscriber_process_identifier);
        h.ack().await;
        h.settle().await;
    }
    assert_eq!(served, [1, 2, 3, 4, 5, 6]);
    h.no_notification().await;
    assert_eq!(throttled(&h), 0);
    assert_eq!(refusals(&spy), 6, "each refused an ID once, nothing more");
    assert_eq!(owed_counts(&h), (6, 6, 0));
}

/// A peer that never acknowledges holds back only its own references: a
/// reference of another peer behind them in the queue goes at once.
#[tokio::test(start_paused = true)]
async fn a_full_peer_does_not_hold_back_another_peers_references() {
    let mut h = with_peer_limit(1).await;
    subscribed_on(&mut h, 1, av1()).await;
    subscribed_on(&mut h, 2, av1()).await;
    // Peer B, owed after the harness peer's two.
    h.server
        .cov_table
        .write()
        .await
        .admit_for_test(
            CovSubscription {
                subscriber_mac: MacAddr::from_slice(&[10, 0, 0, 6, 0xBA, 0xC0]),
                subscriber_network: None,
                subscriber_process_identifier: 3,
                monitored_object_identifier: av1(),
                issue_confirmed_notifications: true,
                expires_at: Some(
                    crate::runtime_clock::core_now() + bacnet_time::Duration::from_secs(300),
                ),
                last_notified_observation: None,
                monitored_property: None,
                monitored_property_array_index: None,
                cov_increment: None,
                notification_kind: CovNotificationKind::Single,
                timestamped: false,
            },
            0,
        )
        .unwrap();
    let mut held = lease_all(&h);
    h.write_local(42.0).await;
    let owed_processes: Vec<_> = owed(&h)
        .await
        .iter()
        .map(|key| match key {
            bacnet_device_core::cov::CovSubscriptionKey::Object { process_id, .. } => *process_id,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(owed_processes, [1, 2, 3]);

    held.truncate(held.len() - 3);
    let mut sent = [
        h.cov_notification().await.subscriber_process_identifier,
        h.cov_notification().await.subscriber_process_identifier,
    ];
    sent.sort_unstable();
    assert_eq!(sent, [1, 3], "B's goes beside the harness peer's first");
    h.no_notification().await;
    assert_eq!(owed(&h).await.len(), 1);
}

/// The pool of in-flight slots full when an ID comes back: the reference
/// waits in its place, untried, and goes when a report in flight ends.
#[tokio::test(start_paused = true)]
async fn a_look_that_finds_the_pool_full_waits_for_a_report_to_end() {
    let (spy, _guard) = WarnSpy::install();
    let mut h = Harness::start(ServerConfig::default()).await;
    subscribed_on(&mut h, 1, av1()).await;
    let mut held = lease_all(&h);
    h.write_local(42.0).await;
    let tracker = tracker(&h).await;
    let other = CovRecipient::from_endpoint(&[7], None);
    let mut pool: Vec<_> = (0..255)
        .map(|_| {
            tracker
                .try_acquire(other.clone(), usize::MAX, &h.server.cov_in_flight)
                .expect("a pool slot")
        })
        .collect();
    drop(held.pop());
    tokio::time::sleep(Duration::from_secs(1)).await;
    h.no_notification().await;
    assert_eq!(owed(&h).await.len(), 1);

    drop(pool.pop());
    assert_eq!(pv_of(&h.cov_notification().await), real(42.0));
    let pool_refusals = spy
        .warnings()
        .iter()
        .filter(|warning| warning.message.starts_with("255 confirmed COV"))
        .count();
    assert_eq!(
        pool_refusals, 0,
        "no reservation tried while the pool was full"
    );
}

/// An ID that comes back between a pass that found none and the wait that
/// follows still ends the wait: the wait was made before the pass.
#[tokio::test(start_paused = true)]
async fn an_id_back_between_the_pass_and_the_wait_ends_the_wait() {
    let mut h = Harness::start(ServerConfig::default()).await;
    subscribed_on(&mut h, 1, av1()).await;
    let mut held = lease_all(&h);
    let freed = held.pop().unwrap();
    owed_gap::set(move || drop(freed));
    h.write_local(42.0).await;
    assert_eq!(pv_of(&h.cov_notification().await), real(42.0));
}

/// A peer slot that comes back between a pass that found the peer full and
/// the wait that follows still ends the wait.
#[tokio::test(start_paused = true)]
async fn a_slot_back_between_the_pass_and_the_wait_ends_the_wait() {
    let mut h = with_peer_limit(1).await;
    subscribed_on(&mut h, 1, av1()).await;
    let mut held = lease_all(&h);
    h.write_local(42.0).await;
    h.settle().await;
    let slot = tracker(&h)
        .await
        .try_acquire(peer(), 1, &h.server.cov_in_flight)
        .expect("the peer's slot");
    owed_gap::set(move || drop(slot));
    drop(held.pop());
    assert_eq!(pv_of(&h.cov_notification().await), real(42.0));
}

fn av(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap()
}

/// Lease every invoke ID still free.
fn lease_rest(h: &Harness) -> Vec<super::notification_transactions::NotificationReservation> {
    std::iter::from_fn(|| {
        h.server
            .notification_transactions
            .reserve(
                canonical_direct_peer(&[1]),
                ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
            )
            .ok()
    })
    .collect()
}

/// A confirmed context's follow-up too large for one notification (20
/// objects, a 206-octet subscriber) is refused an ID: the whole context is
/// owed, not only the first part's references, so the later parts go once
/// an ID comes back even when the first part's values went back meanwhile.
#[tokio::test(start_paused = true)]
async fn a_split_report_refused_an_id_owes_its_whole_context() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        for instance in 2..=20 {
            let name = format!("AV-{instance}");
            let object =
                AnalogValueObject::new(instance, &name, EngineeringUnits::DEGREES_CELSIUS).unwrap();
            db.add(Box::new(object)).unwrap();
        }
    })
    .await;
    h.request_max_apdu = 206;
    let specs = (1..=20).map(|i| (av(i), vec![(PV, false)])).collect();
    h.subscribe_with_delay(true, specs, 10).await;
    let mut conveyed = 0;
    while conveyed < 20 {
        conveyed += h.notification().await.list_of_cov_notifications.len();
        h.ack().await;
        h.settle().await;
    }
    h.no_notification().await;
    // One report out, for AV-1; the rest change while it is.
    h.write_local_to(av(1), 1.0).await;
    h.notification().await;
    for i in 2..=20 {
        h.write_local_to(av(i), 5.0).await;
    }
    let mut held = lease_rest(&h);
    let db = h.server.db.write().await;
    h.ack().await;
    h.settle().await;
    // The Ack freed an ID: take it while the follow-up waits for the database.
    held.extend(lease_rest(&h));
    drop(db);
    tokio::time::sleep(Duration::from_secs(1)).await;
    h.no_notification().await;
    assert_eq!(owed(&h).await.len(), 20, "every reference of the context");

    // Some go back to their baselines; the others still differ.
    for i in 2..=5 {
        h.write_local_to(av(i), 0.0).await;
    }
    drop(held.pop());
    let mut reported = std::collections::BTreeSet::new();
    while reported.len() < 15 {
        for item in h.notification().await.list_of_cov_notifications {
            assert!(reported.insert(item.monitored_object_identifier.instance_number()));
        }
        h.ack().await;
        h.settle().await;
    }
    assert_eq!(reported, (6..=20).collect());
    h.no_notification().await;
}
