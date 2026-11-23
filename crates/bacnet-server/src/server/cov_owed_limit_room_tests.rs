//! What a wait for room keeps and what wakes it (#2671): a waiter whose
//! report goes out by its own fanout first leaves its place, so it spends no
//! room at the next look; and a pass runs only when what held the waiters
//! back may be free again, an ID when none was free, a report ending
//! otherwise, without losing a wake when the reason changes between passes.
use super::cov_fanout::owed_gap;
use super::cov_owed_id_room_tests::{av, peer, tracker};
use super::cov_owed_id_tests::{lease_all, owed, owed_counts, pv_of, subscribed_on};
use super::cov_owed_limit_tests::{fill_pool, limited, places, waiting};
use super::cov_wire_test_support::*;
use super::*;
use std::cell::Cell;
use std::rc::Rc;

/// A waiter whose own next fanout finds room first, before any pass (here
/// the pool grows by a slot and no report ends, so none runs), is sent by
/// that fanout and keeps no place; a later waiter of the same peer then
/// goes at the next end.
#[tokio::test(start_paused = true)]
async fn a_waiter_sent_by_its_own_fanout_keeps_no_place() {
    let mut h = limited(16).await;
    subscribed_on(&mut h, 2, av1()).await;
    subscribed_on(&mut h, 3, av(2)).await;
    let tracker = tracker(&h).await;
    let _pool = fill_pool(&h, &tracker, 255);
    h.write_local(42.0).await;
    assert_eq!(waiting(&h).await, [2]);

    h.server.cov_in_flight.add_permits(1);
    h.write_local(43.0).await;
    assert!(owed(&h).await.is_empty());
    assert_eq!(places(&h).await, 0);
    assert_eq!(owed_counts(&h), (1, 1, 0));
    assert_eq!(pv_of(&h.cov_notification().await), real(43.0));

    // The pool full again: AV-2's subscriber waits alone, and goes once
    // the report to process 2 ends.
    h.write_local_to(av(2), 5.0).await;
    assert_eq!(waiting(&h).await, [3]);
    h.ack().await;
    let report = h.cov_notification().await;
    assert_eq!(report.subscriber_process_identifier, 3);
    assert_eq!(pv_of(&report), real(5.0));
}

/// The same for a confirmed COV-multiple context, whose every reference
/// leaves its place once the context's report begins.
#[tokio::test(start_paused = true)]
async fn a_context_sent_by_its_own_fanout_keeps_no_place() {
    let mut h = limited(16).await;
    h.subscribe_specs(true, vec![(av1(), vec![(PV, false)])])
        .await;
    h.notification().await;
    h.ack().await;
    h.settle().await;
    let tracker = tracker(&h).await;
    let _pool = fill_pool(&h, &tracker, 255);
    h.write_local(42.0).await;
    assert_eq!(waiting(&h).await, [856]);

    h.server.cov_in_flight.add_permits(1);
    h.write_local(43.0).await;
    assert!(owed(&h).await.is_empty());
    assert_eq!(places(&h).await, 0);
    assert_eq!(pv_rows(&h.notification().await), vec![(real(43.0), None)]);
}

/// A reference held back only by its peer's limit is not looked at when
/// an invoke ID comes back. Once a report of the peer ends with every ID
/// leased, the look finds no ID and waits for one instead, and the
/// reference goes when one comes back.
#[tokio::test(start_paused = true)]
async fn a_wait_wakes_only_for_what_it_lacks() {
    let mut h = limited(1).await;
    subscribed_on(&mut h, 2, av1()).await;
    let slot = tracker(&h)
        .await
        .try_acquire(peer(), 1, &h.server.cov_in_flight)
        .expect("the peer's slot");
    h.write_local(42.0).await;
    h.settle().await;
    assert_eq!(waiting(&h).await, [2]);

    let looked = Rc::new(Cell::new(false));
    let hook = Rc::clone(&looked);
    owed_gap::set(move || hook.set(true));
    let lease = h
        .server
        .notification_transactions
        .reserve(
            canonical_direct_peer(&[1]),
            ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
        )
        .expect("an invoke ID");
    drop(lease);
    h.settle().await;
    assert!(!looked.get(), "an ID back runs no pass for a peer's waiter");

    let mut held = lease_all(&h);
    drop(slot);
    h.settle().await;
    assert!(looked.get(), "the report's end ran a pass");
    h.no_notification().await;
    assert_eq!(waiting(&h).await, [2], "now it lacks an ID");

    drop(held.pop());
    assert_eq!(pv_of(&h.cov_notification().await), real(42.0));
}
