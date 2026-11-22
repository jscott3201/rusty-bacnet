//! A subscription waiting at an in-flight limit (#2671) stops waiting with
//! it: when it is cancelled, expires, is removed with its object or its
//! peer, or is replaced by a renewal (whose own initial report waits in its
//! turn); when DeviceCommunicationControl restricts initiation; and when the
//! server stops. None leaves a place in the queue or a slot in flight.
use super::cov_owed_id_room_tests::{av, peer, tracker};
use super::cov_owed_id_tests::{owed, owed_counts, pv_of, subscribed_on};
use super::cov_owed_limit_tests::{
    in_flight, limited, peer_at_its_limit, places, subscribe_cov, waiting,
};
use super::cov_wire_test_support::*;
use super::*;
use crate::cov::CovInFlightGuard;
use bacnet_services::object_mgmt::DeleteObjectRequest;

/// [`limited`] to one report, process 2's confirmed SubscribeCOV to
/// `object` for `lifetime` seconds acknowledged, the peer's one slot held
/// by the test, and `object` changed: process 2 waits.
async fn held_back(object: ObjectIdentifier, lifetime: u32) -> (Harness, CovInFlightGuard) {
    let mut h = limited(1).await;
    subscribe_cov(&mut h, 2, object, Some(lifetime))
        .await
        .expect("subscribed");
    h.cov_notification().await;
    h.ack().await;
    h.settle().await;
    let slot = tracker(&h)
        .await
        .try_acquire(peer(), 1, &h.server.cov_in_flight)
        .expect("the peer's slot");
    h.write_local_to(object, 42.0).await;
    assert_eq!(waiting(&h).await, [2]);
    (h, slot)
}

/// Nothing waits, nothing holds a place, and once the test's slot goes
/// nothing is sent and nothing is in flight.
async fn gone(h: &Harness, slot: CovInFlightGuard) {
    assert!(owed(h).await.is_empty());
    assert_eq!(places(h).await, 0);
    drop(slot);
    tokio::time::sleep(Duration::from_secs(1)).await;
    h.no_notification().await;
    assert_eq!(in_flight(h).await, 0);
}

#[tokio::test(start_paused = true)]
async fn a_cancelled_subscription_stops_waiting() {
    let (mut h, slot) = held_back(av1(), 300).await;
    subscribe_cov(&mut h, 2, av1(), None)
        .await
        .expect("cancelled");
    assert_eq!(owed_counts(&h), (1, 0, 1));
    gone(&h, slot).await;
}

/// The purge, which runs every 30 s, removes the expired subscription and
/// its wait.
#[tokio::test(start_paused = true)]
async fn an_expired_subscription_stops_waiting() {
    let (h, slot) = held_back(av1(), 5).await;
    tokio::time::sleep(Duration::from_secs(35)).await;
    assert_eq!(owed_counts(&h), (1, 0, 1));
    gone(&h, slot).await;
}

#[tokio::test(start_paused = true)]
async fn a_subscription_to_a_deleted_object_stops_waiting() {
    let (mut h, slot) = held_back(av(2), 300).await;
    let mut body = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: av(2),
    }
    .encode(&mut body);
    h.request(ConfirmedServiceChoice::DELETE_OBJECT, body).await;
    response(&h).await.expect("deleted");
    assert_eq!(owed_counts(&h), (1, 0, 1));
    gone(&h, slot).await;
}

/// The application's removal of a peer's subscriptions, as for a peer it
/// finds unreachable.
#[tokio::test(start_paused = true)]
async fn a_removed_peers_subscription_stops_waiting() {
    let (h, slot) = held_back(av1(), 300).await;
    assert_eq!(h.server.remove_peer_subscriptions(&PEER, None).await, 1);
    assert_eq!(owed_counts(&h), (1, 0, 1));
    gone(&h, slot).await;
}

/// A renewal replaces the wait: its own initial report, held back too,
/// waits at the back, and one report of the current value goes.
#[tokio::test(start_paused = true)]
async fn a_renewal_waits_afresh_and_reports_once() {
    let (mut h, slot) = held_back(av1(), 300).await;
    subscribe_cov(&mut h, 2, av1(), Some(300))
        .await
        .expect("renewed");
    assert_eq!(waiting(&h).await, [2]);
    assert_eq!(owed_counts(&h), (2, 0, 1));

    drop(slot);
    assert_eq!(pv_of(&h.cov_notification().await), real(42.0));
    h.ack().await;
    h.settle().await;
    h.no_notification().await;
    assert_eq!(places(&h).await, 0);
    assert_eq!(in_flight(&h).await, 0);
}

/// Initiation restricted while a subscription waits behind the peer's
/// report: the report is withdrawn at its next attempt, and the look that
/// follows is dropped, as any fanout is under DCC. Nothing is left waiting
/// or in flight, and nothing goes when initiation is enabled again.
#[tokio::test(start_paused = true)]
async fn restricted_initiation_ends_the_wait() {
    let mut h = limited(1).await;
    subscribed_on(&mut h, 2, av1()).await;
    let _out = peer_at_its_limit(&mut h).await;
    h.write_local(42.0).await;
    assert_eq!(waiting(&h).await, [2]);

    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    // The report's first retry is due at 3 s.
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(owed(&h).await.is_empty());
    assert_eq!(places(&h).await, 0);
    assert_eq!(in_flight(&h).await, 0);
    h.no_notification().await;

    h.server.comm_state.set_for_test(DccState::Enable);
    tokio::time::sleep(Duration::from_secs(1)).await;
    h.no_notification().await;
    assert_eq!(owed_counts(&h), (1, 1, 0));
}

/// A stop ends every wait and every report in flight, and nothing goes
/// after it.
#[tokio::test(start_paused = true)]
async fn a_stop_ends_the_wait() {
    let mut h = limited(1).await;
    subscribed_on(&mut h, 2, av1()).await;
    let _out = peer_at_its_limit(&mut h).await;
    h.write_local(42.0).await;
    assert_eq!(waiting(&h).await, [2]);

    h.server.stop().await.unwrap();
    assert!(owed(&h).await.is_empty());
    assert_eq!(places(&h).await, 0);
    assert_eq!(in_flight(&h).await, 0);
    assert_eq!(owed_counts(&h), (1, 0, 1));
    tokio::time::sleep(Duration::from_secs(1)).await;
    h.no_notification().await;
}
