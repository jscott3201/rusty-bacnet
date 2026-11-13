//! [`TimedHistories::replace_latest`] (#2535): the capture after a write's
//! transition takes the place of the row the write queued, while that row
//! is still the reference's newest pending change.

use super::tests::{TimedClaim, change, histories, key, seconds, store};
use bacnet_time::Duration;

#[test]
fn the_row_a_write_queued_is_replaced_by_its_transitions_capture() {
    let mut h = histories(8, 4);
    let k = key(1, 1);
    h.reset(&k, 1, 0);
    assert!(h.push(&k, 1, change(1, 4)), "the write's row");
    assert!(h.replace_latest(&k, 1, change(2, 4)), "the transition's");
    assert_eq!(h.baseline(&k, 1), Some(change(2, 4).observation()));
    assert_eq!(seconds(&h.drain(&k, 1).1), [2], "one row");
}

#[test]
fn only_the_newest_pending_change_is_replaced() {
    let mut h = histories(8, 4);
    let k = key(1, 1);
    h.reset(&k, 1, 0);
    h.push(&k, 1, change(1, 4));
    h.push(&k, 1, change(2, 4));
    h.replace_latest(&k, 1, change(3, 4));
    assert_eq!(seconds(&h.drain(&k, 1).1), [1, 3]);
}

/// A row a report has taken (being sent, or sent) is the subscriber's: the
/// capture after the transition is queued after it.
#[test]
fn a_row_a_report_took_is_not_replaced() {
    let mut h = histories(8, 4);
    let k = key(1, 1);
    h.reset(&k, 1, 0);
    h.push(&k, 1, change(1, 4));
    assert_eq!(seconds(&h.drain(&k, 1).1), [1], "a report takes it");
    h.replace_latest(&k, 1, change(2, 4));
    assert_eq!(seconds(&h.drain(&k, 1).1), [2], "queued after it");
}

/// With nothing pending the capture is queued as a change of its own.
#[test]
fn with_nothing_pending_the_capture_is_queued() {
    let mut h = histories(8, 4);
    let k = key(1, 1);
    h.reset(&k, 1, 0);
    h.replace_latest(&k, 1, change(2, 4));
    assert_eq!(seconds(&h.drain(&k, 1).1), [2]);
}

/// The replacement's Max_Notification_Delay runs from the write's capture,
/// not from the later one that replaced it.
#[test]
fn the_replacement_keeps_the_writes_capture_instant() {
    let mut h = histories(8, 4);
    let k = key(1, 1);
    h.reset(&k, 1, 5);
    h.push(&k, 1, change(1, 4));
    let due = h.next_deadline().expect("a pending change is due");
    super::host::advance(Duration::from_secs(2));
    h.replace_latest(&k, 1, change(2, 4));
    assert_eq!(h.next_deadline(), Some(due));
}

#[test]
fn a_generation_no_longer_live_queues_nothing() {
    let mut h = histories(8, 4);
    let k = key(1, 1);
    h.reset(&k, 2, 0);
    assert!(!h.replace_latest(&k, 1, change(2, 4)));
    assert!(h.drain(&k, 2).1.is_empty());
}

/// A change a part has carried some values of stays in delivery: its rest
/// is the subscriber's to complete, so the capture is queued after it.
#[test]
fn a_change_in_delivery_is_not_replaced() {
    let mut h = histories(8, 4);
    let k = key(1, 1);
    h.reset(&k, 1, 0);
    h.push(&k, 1, change(1, 4));
    let history = h.history_mut(&k, 1).expect("live");
    history.in_delivery = history.latest;
    h.replace_latest(&k, 1, change(2, 4));
    assert_eq!(seconds(&h.drain(&k, 1).1), [1, 2]);
}

/// An older change a failed notification gave back while a newer one is in
/// flight is not the write's row: the capture is queued after it, and both
/// go out in capture order if the newer notification fails as well.
#[test]
fn an_older_change_given_back_is_not_replaced() {
    let (store, _) = store(8, 4);
    let k = key(1, 1);
    store.lock().reset(&k, 1, 0);
    let mut claims = Vec::new();
    for second in 1..=2 {
        store.lock().push(&k, 1, change(second, 4));
        let mut claim = TimedClaim::new(store.clone());
        let (incarnation, drained) = store.lock().drain(&k, 1);
        claim.add(k.clone(), incarnation, drained);
        claims.push(claim);
    }
    let newer = claims.pop().expect("two claims");
    drop(claims); // the older failed first, while the newer is in flight
    store.lock().replace_latest(&k, 1, change(3, 4));
    drop(newer);
    assert_eq!(seconds(&store.lock().drain(&k, 1).1), [1, 2, 3]);
}

/// The replaced row's octets and memory leave its context's bound: the
/// context holds one row's worth, and nothing once it is drained.
#[test]
fn a_replaced_row_leaves_its_contexts_bound() {
    let mut h = histories(8, 4);
    let k = key(1, 1);
    h.reset(&k, 1, 0);
    h.push(&k, 1, change(1, 4));
    let one = h.context_held.get(&super::tests::context(1)).copied();
    assert!(one.is_some(), "the write's row is held");
    h.replace_latest(&k, 1, change(2, 4));
    assert_eq!(h.context_held.get(&super::tests::context(1)).copied(), one);
    h.drain(&k, 1);
    assert!(h.context_held.is_empty(), "{:?}", h.context_held);
}
