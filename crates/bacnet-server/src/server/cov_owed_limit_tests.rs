//! A confirmed COV report held back by an in-flight limit waits (#2671): one
//! refused by its peer's limit goes once one of that peer's reports ends, one
//! refused by the pool of 255 once any report ends, each with the value then
//! current and once however often the value moved meanwhile, measured from
//! the value last reported. A peer at its limit holds back none but its own
//! references, and at the subscription cap each subscription waits once. The
//! wait is the one a report refused an invoke ID has (#2661), for every form.
//!
//! Time is paused. A test holds a slot itself where it says so; otherwise the
//! peer is at its limit with a report of the server's own, unanswered, which
//! the test acknowledges well inside its retries.
use super::cov_owed_id_room_tests::{av, peer, throttled, tracker};
use super::cov_owed_id_tests::{owed, owed_counts, pv_of, subscribed_on};
use super::cov_wire_test_support::*;
use super::*;
use bacnet_device_core::cov::{
    CovNotificationKind, CovPolicy, CovRecipient, CovSubscription, CovSubscriptionKey,
};
use bacnet_objects::analog::AnalogValueObject;
use bacnet_services::cov::{SubscribeCOVPropertyRequest, SubscribeCOVRequest};
use bacnet_types::enums::EngineeringUnits;

/// A server with AV-1 and AV-2 whose subscriber may have `per_peer`
/// confirmed reports in flight, under `policy` otherwise.
pub(super) async fn limited_under(per_peer: usize, policy: CovPolicy) -> Harness {
    Harness::start_with(
        ServerConfig {
            cov_policy: CovPolicy {
                max_confirmed_in_flight_per_peer: per_peer,
                ..policy
            },
            ..ServerConfig::default()
        },
        |db| {
            let av2 = AnalogValueObject::new(2, "AV-2", EngineeringUnits::DEGREES_CELSIUS);
            db.add(Box::new(av2.unwrap())).unwrap();
        },
    )
    .await
}

/// [`limited_under`] the default policy.
pub(super) async fn limited(per_peer: usize) -> Harness {
    limited_under(per_peer, CovPolicy::default()).await
}

/// [`limited`] to one report, with process 1's confirmed SubscribeCOV to
/// AV-2 acknowledged. AV-2 then changes, and the report to process 1 stays
/// out unanswered, so the peer is at its limit. Returns that report's
/// invoke ID and service, for its Ack.
pub(super) async fn peer_at_its_limit(h: &mut Harness) -> (u8, ConfirmedServiceChoice) {
    subscribed_on(h, 1, av(2)).await;
    h.write_local_to(av(2), 5.0).await;
    assert_eq!(h.cov_notification().await.subscriber_process_identifier, 1);
    h.take_confirmed()
}

/// The process of each subscription waiting for room, oldest first.
pub(super) async fn waiting(h: &Harness) -> Vec<u32> {
    owed(h)
        .await
        .iter()
        .map(|key| match key {
            CovSubscriptionKey::Object { process_id, .. }
            | CovSubscriptionKey::Property { process_id, .. } => *process_id,
            CovSubscriptionKey::Multiple { context, .. } => context.process_id,
        })
        .collect()
}

/// References holding a place, waiting or being looked at.
pub(super) async fn places(h: &Harness) -> usize {
    h.server.cov_table.read().await.revisits().owed_places()
}

/// Confirmed reports in flight to the harness subscriber.
pub(super) async fn in_flight(h: &Harness) -> usize {
    tracker(h)
        .await
        .in_flight()
        .get(&peer())
        .copied()
        .unwrap_or(0)
}

/// SubscribeCOV for `process` on `object`, confirmed for `lifetime`
/// seconds, or a cancellation when it is `None`; the answer.
pub(super) async fn subscribe_cov(
    h: &mut Harness,
    process: u32,
    object: ObjectIdentifier,
    lifetime: Option<u32>,
) -> Result<(), ErrorCode> {
    let mut body = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: process,
        monitored_object_identifier: object,
        issue_confirmed_notifications: lifetime.map(|_| true),
        lifetime,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV, body).await;
    response(h).await
}

/// A confirmed SubscribeCOVProperty of `process` to AV-1's Present_Value
/// with `increment`, its initial report acknowledged.
async fn subscribed_to_pv(h: &mut Harness, process: u32, increment: Option<f32>) {
    let mut body = BytesMut::new();
    SubscribeCOVPropertyRequest {
        subscriber_process_identifier: process,
        monitored_object_identifier: av1(),
        issue_confirmed_notifications: Some(true),
        lifetime: Some(300),
        monitored_property_identifier: PV,
        monitored_property_array_index: None,
        cov_increment: increment,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY, body)
        .await;
    response(h).await.expect("subscribed");
    h.cov_notification().await;
    h.ack().await;
    h.settle().await;
}

/// A second peer's confirmed SubscribeCOV to AV-1, as process 3, with no
/// baseline yet: AV-1's next change reports to it.
async fn another_peer_subscribed(h: &Harness) {
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
}

/// Hold `count` slots of the pool, for a peer the harness never answers.
pub(super) fn fill_pool(
    h: &Harness,
    tracker: &Arc<crate::cov::CovInFlightTracker>,
    count: usize,
) -> Vec<crate::cov::CovInFlightGuard> {
    let other = CovRecipient::from_endpoint(&[7], None);
    (0..count)
        .map(|_| {
            tracker
                .try_acquire(other.clone(), usize::MAX, &h.server.cov_in_flight)
                .expect("a pool slot")
        })
        .collect()
}

/// Reports the pool held back so far.
fn pool_refusals(h: &Harness) -> u64 {
    h.server
        .cov_counters
        .notifications_throttled_pool
        .load(std::sync::atomic::Ordering::Relaxed)
}

/// A value that changes once while its subscriber's peer is at its limit is
/// reported, with that value, once the peer acknowledges its report in
/// flight, with no further change to prompt it. The wait holds no slot.
#[tokio::test(start_paused = true)]
async fn a_change_held_back_by_its_peers_limit_goes_once_a_report_of_the_peer_ends() {
    let mut h = limited(1).await;
    subscribed_on(&mut h, 2, av1()).await;
    let out = peer_at_its_limit(&mut h).await;
    h.write_local(42.0).await;
    h.settle().await;
    h.no_notification().await;
    assert_eq!(waiting(&h).await, [2]);
    assert_eq!(throttled(&h), 1);
    assert_eq!(in_flight(&h).await, 1, "the wait holds no slot");

    h.ack_request(out).await;
    let report = h.cov_notification().await;
    assert_eq!(report.subscriber_process_identifier, 2);
    assert_eq!(pv_of(&report), real(42.0));
    h.ack().await;
    h.settle().await;
    h.no_notification().await;
    assert!(owed(&h).await.is_empty());
    assert_eq!(places(&h).await, 0);
    assert_eq!(owed_counts(&h), (1, 1, 0));
    assert_eq!(in_flight(&h).await, 0);
}

/// The same for the pool: with 255 confirmed reports in flight to another
/// peer, a change waits, untried, and is reported once one of them ends.
#[tokio::test(start_paused = true)]
async fn a_change_held_back_by_the_pool_goes_once_a_report_ends() {
    let mut h = limited(16).await;
    subscribed_on(&mut h, 1, av1()).await;
    let tracker = tracker(&h).await;
    let mut pool = fill_pool(&h, &tracker, 255);
    h.write_local(42.0).await;
    h.settle().await;
    h.no_notification().await;
    assert_eq!(waiting(&h).await, [1]);
    assert_eq!(pool_refusals(&h), 1);

    drop(pool.pop());
    assert_eq!(pv_of(&h.cov_notification().await), real(42.0));
    h.ack().await;
    h.settle().await;
    h.no_notification().await;
    assert_eq!(owed_counts(&h), (1, 1, 0));
    assert_eq!(
        pool_refusals(&h),
        1,
        "nothing tried while the pool was full"
    );
}

/// Several changes while held back make one wait and one report, of the
/// latest value; each change counts as held back.
#[tokio::test(start_paused = true)]
async fn changes_held_back_by_a_limit_make_one_report_of_the_latest() {
    let mut h = limited(1).await;
    subscribed_on(&mut h, 2, av1()).await;
    let out = peer_at_its_limit(&mut h).await;
    for value in [1.0, 2.0, 3.0, 4.0] {
        h.write_local(value).await;
    }
    assert_eq!(waiting(&h).await, [2]);
    assert_eq!(throttled(&h), 4);

    h.ack_request(out).await;
    assert_eq!(pv_of(&h.cov_notification().await), real(4.0));
    h.ack().await;
    h.settle().await;
    h.no_notification().await;
    assert_eq!(owed_counts(&h), (1, 1, 0));
}

/// The look measures from the value last reported to the subscriber: a
/// value back within the COV increment of it sends nothing, however far it
/// moved while it waited; one past it later goes at once. SubscribeCOVProperty
/// with an increment of 1.
#[tokio::test(start_paused = true)]
async fn a_value_back_within_the_increment_of_the_last_report_sends_nothing() {
    let mut h = limited(1).await;
    subscribed_to_pv(&mut h, 2, Some(1.0)).await;
    let out = peer_at_its_limit(&mut h).await;
    h.write_local(5.0).await;
    h.write_local(0.6).await;
    assert_eq!(waiting(&h).await, [2]);

    h.ack_request(out).await;
    h.settle().await;
    h.no_notification().await;
    assert!(owed(&h).await.is_empty());
    assert_eq!(owed_counts(&h), (1, 1, 0));
    h.write_local(1.5).await;
    assert_eq!(pv_of(&h.cov_notification().await), real(1.5));
}

/// A peer at its limit delays no other peer: a change of their common
/// object goes to the other at once while the first peer's waits.
#[tokio::test(start_paused = true)]
async fn a_peer_at_its_limit_does_not_delay_another_peers_report() {
    let mut h = limited(1).await;
    subscribed_on(&mut h, 2, av1()).await;
    let out = peer_at_its_limit(&mut h).await;
    another_peer_subscribed(&h).await;
    h.write_local(42.0).await;
    let report = h.cov_notification().await;
    assert_eq!(report.subscriber_process_identifier, 3);
    assert_eq!(pv_of(&report), real(42.0));
    h.no_notification().await;
    assert_eq!(waiting(&h).await, [2]);

    h.ack_request(out).await;
    let report = h.cov_notification().await;
    assert_eq!(report.subscriber_process_identifier, 2);
    assert_eq!(pv_of(&report), real(42.0));
}

/// Both peers wait for the pool, the one at its limit first: a slot that
/// frees goes to the other peer's reference behind it, and the first
/// peer's goes once its own report ends.
#[tokio::test(start_paused = true)]
async fn a_peer_at_its_limit_does_not_hold_the_pool_back_from_another() {
    let mut h = limited(1).await;
    subscribed_on(&mut h, 2, av1()).await;
    let out = peer_at_its_limit(&mut h).await;
    another_peer_subscribed(&h).await;
    // The pool full: the harness peer's report and 254 of another peer's.
    let tracker = tracker(&h).await;
    let mut pool = fill_pool(&h, &tracker, 254);
    h.write_local(42.0).await;
    h.settle().await;
    h.no_notification().await;
    assert_eq!(waiting(&h).await, [2, 3], "the full peer's first");
    assert_eq!((throttled(&h), pool_refusals(&h)), (1, 1));

    drop(pool.pop());
    let report = h.cov_notification().await;
    assert_eq!(report.subscriber_process_identifier, 3);
    assert_eq!(pv_of(&report), real(42.0));
    h.no_notification().await;
    assert_eq!(waiting(&h).await, [2]);

    h.ack_request(out).await;
    let report = h.cov_notification().await;
    assert_eq!(report.subscriber_process_identifier, 2);
    assert_eq!(pv_of(&report), real(42.0));
}

/// At the subscription cap, every subscription held back by its peer's
/// limit waits once however often its value changes, so the waits are at
/// most as many as the subscriptions; each then reports the latest value,
/// in the order held back.
#[tokio::test(start_paused = true)]
async fn at_the_subscription_cap_each_subscription_waits_once() {
    const CAP: u32 = 8;
    let mut h = limited_under(
        1,
        CovPolicy {
            max_subscriptions_global: CAP as usize,
            reserved_capacity: 0,
            ..CovPolicy::default()
        },
    )
    .await;
    for process in 1..=CAP {
        subscribed_on(&mut h, process, av1()).await;
    }
    assert!(
        subscribe_cov(&mut h, 99, av1(), Some(300)).await.is_err(),
        "the table is at its cap"
    );
    let slot = tracker(&h)
        .await
        .try_acquire(peer(), 1, &h.server.cov_in_flight)
        .expect("the peer's slot");
    for step in 1..=20_u8 {
        h.write_local(f32::from(step)).await;
    }
    assert_eq!(waiting(&h).await, (1..=CAP).collect::<Vec<_>>());
    assert_eq!(places(&h).await, CAP as usize);
    assert_eq!(owed_counts(&h), (u64::from(CAP), 0, 0));
    assert_eq!(throttled(&h), 20 * u64::from(CAP));

    drop(slot);
    for process in 1..=CAP {
        let report = h.cov_notification().await;
        assert_eq!(report.subscriber_process_identifier, process);
        assert_eq!(pv_of(&report), real(20.0));
        h.ack().await;
        h.settle().await;
    }
    h.no_notification().await;
    assert_eq!(places(&h).await, 0);
    assert_eq!(owed_counts(&h), (u64::from(CAP), u64::from(CAP), 0));
}

/// A confirmed SubscribeCOVProperty held back by its peer's limit goes the
/// same way.
#[tokio::test(start_paused = true)]
async fn a_property_subscription_held_back_by_its_peers_limit_goes_once_a_report_ends() {
    let mut h = limited(1).await;
    subscribed_to_pv(&mut h, 2, None).await;
    let out = peer_at_its_limit(&mut h).await;
    h.write_local(42.0).await;
    h.settle().await;
    h.no_notification().await;
    assert_eq!(waiting(&h).await, [2]);

    h.ack_request(out).await;
    let report = h.cov_notification().await;
    assert_eq!(report.subscriber_process_identifier, 2);
    assert_eq!(pv_of(&report), real(42.0));
}

/// An untimestamped confirmed COV-multiple context held back by its peer's
/// limit goes with the latest value once the peer's report ends.
#[tokio::test(start_paused = true)]
async fn a_context_held_back_by_its_peers_limit_goes_once_a_report_ends() {
    let mut h = limited(1).await;
    h.subscribe_specs(true, vec![(av1(), vec![(PV, false)])])
        .await;
    h.notification().await;
    h.ack().await;
    h.settle().await;
    let out = peer_at_its_limit(&mut h).await;
    h.write_local(42.0).await;
    h.write_local(43.0).await;
    h.settle().await;
    h.no_notification().await;
    assert_eq!(waiting(&h).await, [856]);
    assert_eq!(throttled(&h), 2, "held back at each change, waiting once");

    h.ack_request(out).await;
    assert_eq!(pv_rows(&h.notification().await), vec![(real(43.0), None)]);
    h.ack().await;
    h.settle().await;
    h.no_notification().await;
    assert_eq!(owed_counts(&h), (1, 1, 0));
}

/// A timestamped context held back keeps its rows: each refusal, the
/// backstop's included, gives the history back, and the report that goes
/// once the peer's report ends carries every row once, in capture order.
#[tokio::test(start_paused = true)]
async fn a_timestamped_context_held_back_by_its_peers_limit_keeps_its_rows() {
    let mut h = limited(1).await;
    h.subscribe_with_delay(true, vec![(av1(), vec![(PV, true)])], 2)
        .await;
    h.notification().await;
    h.ack().await;
    h.settle().await;
    let out = peer_at_its_limit(&mut h).await;
    h.set_clock(10);
    h.write_local(1.0).await;
    h.set_clock(11);
    h.write_local(2.0).await;
    // Past the context's delay, inside the report's first retry timeout.
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    h.no_notification().await;
    assert_eq!(waiting(&h).await, [856]);

    h.ack_request(out).await;
    assert_eq!(
        pv_rows(&h.notification().await),
        vec![(real(1.0), Some(time(10))), (real(2.0), Some(time(11)))]
    );
    h.ack().await;
    h.settle().await;
    tokio::time::sleep(Duration::from_secs(30)).await;
    h.no_notification().await;
}
