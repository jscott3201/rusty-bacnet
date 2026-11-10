//! What a host does explicitly, on hand-set instants: settling a claim,
//! lodging one until a deadline, and asking for the next deadline.
use super::tests::{advance, change, dropped, histories, key, now, seconds};
use super::*;

/// A claim of every pending change of `k`'s generation 1.
fn claim_all(h: &mut TimedHistories, k: &CovSubscriptionKey) -> TimedClaim {
    let mut claim = TimedClaim::new();
    let (incarnation, drained) = h.drain(k, 1);
    claim.add(k.clone(), incarnation, drained);
    claim
}

const DELAY: Duration = Duration::from_secs(10);

/// Histories with one timestamped reference, Max_Notification_Delay 10 s.
fn one_reference() -> (TimedHistories, CovSubscriptionKey) {
    let mut h = histories(8, 4);
    let k = key(1, 1);
    h.reset(&k, 1, 10);
    (h, k)
}

#[test]
fn a_claim_is_committed_or_given_back_by_whoever_holds_it() {
    let (mut h, k) = one_reference();
    h.push(&k, 1, change(1, 4));
    h.push(&k, 1, change(2, 4));
    let claim = claim_all(&mut h, &k);
    assert!(!claim.is_empty());
    assert!(h.drain(&k, 1).1.is_empty(), "claimed");
    h.push(&k, 1, change(3, 4));
    h.give_back(claim, now());
    assert_eq!(
        seconds(&h.drain(&k, 1).1),
        [1, 2, 3],
        "back in capture order"
    );

    h.push(&k, 1, change(4, 4));
    let claim = claim_all(&mut h, &k);
    h.commit(claim);
    assert!(h.drain(&k, 1).1.is_empty(), "retired");
    assert!(TimedClaim::new().is_empty());
    assert_eq!(dropped(&h), 0);
}

#[test]
fn the_max_notification_delay_backstop_is_the_next_deadline() {
    let (mut h, k) = one_reference();
    assert_eq!(h.next_deadline(), None, "nothing pending");
    let start = now();
    h.push(&k, 1, change(1, 4));
    assert!(h.take_moved(), "a first pending change moves the deadline");
    assert_eq!(h.next_deadline(), Some(start + DELAY));
    // Nobody asks until well after it: the deadline stands, and the first
    // call at or after it hands the context out.
    advance(Duration::from_secs(9));
    assert_eq!(h.take_due(now()), (Vec::new(), Some(start + DELAY)));
    advance(Duration::from_secs(1));
    assert_eq!(h.next_deadline(), Some(now()), "due now");
    let (due, next) = h.take_due(now());
    assert_eq!(due, std::slice::from_ref(&k));
    assert_eq!(next, None, "nothing else is ahead");
    // Still pending, so the attempt's wait is the next deadline.
    assert_eq!(h.next_deadline(), Some(now() + DELAY));
    h.drain(&k, 1);
    assert_eq!(h.next_deadline(), None, "conveyed");
}

#[test]
fn a_lodged_claim_is_given_back_at_its_deadline_if_nobody_settles_it() {
    let (mut h, k) = one_reference();
    let start = now();
    h.push(&k, 1, change(1, 4));
    h.take_moved();
    let claim = claim_all(&mut h, &k);
    let deadline = start + Duration::from_secs(15);
    let id = h.lodge(claim, deadline);
    assert!(h.take_moved(), "a lodged claim brings a deadline");
    assert_eq!(h.next_deadline(), Some(deadline), "the claim's, alone");
    // Its holder is gone. Until the deadline the changes stay claimed.
    advance(Duration::from_secs(14));
    assert_eq!(h.take_due(now()), (Vec::new(), Some(deadline)));
    assert!(h.drain(&k, 1).1.is_empty());
    // At the deadline the histories give the claim back themselves, and the
    // change, older than its delay by now, is due at once.
    advance(Duration::from_secs(1));
    let (due, next) = h.take_due(now());
    assert_eq!(due, std::slice::from_ref(&k));
    assert_eq!(next, None);
    assert_eq!(seconds(&h.drain(&k, 1).1), [1]);
    // A holder that turns up after that settles nothing.
    assert!(!h.settle(id, Settlement::Delivered, now()));
    assert!(!h.settle(id, Settlement::Returned, now()));
    assert_eq!(dropped(&h), 0);
}

#[test]
fn a_lodged_claim_settled_in_time_never_comes_due() {
    let (mut h, k) = one_reference();
    h.push(&k, 1, change(1, 4));
    let deadline = now() + Duration::from_secs(15);
    let claim = claim_all(&mut h, &k);
    let delivered = h.lodge(claim, deadline);
    assert!(h.settle(delivered, Settlement::Delivered, now()));
    assert!(!h.settle(delivered, Settlement::Delivered, now()), "once");
    assert_eq!(h.next_deadline(), None, "its deadline went with it");

    h.push(&k, 1, change(2, 4));
    let pushed = now();
    let claim = claim_all(&mut h, &k);
    let returned = h.lodge(claim, deadline);
    advance(Duration::from_secs(3));
    assert!(h.settle(returned, Settlement::Returned, now()));
    assert_eq!(
        h.next_deadline(),
        Some(pushed + DELAY),
        "pending again, from when it was captured"
    );
    advance(Duration::from_secs(60));
    assert_eq!(h.take_due(now()).0, std::slice::from_ref(&k));
    assert_eq!(seconds(&h.drain(&k, 1).1), [2], "given back once");
}

#[test]
fn lodged_claims_come_due_each_at_its_own_deadline_oldest_first() {
    let mut h = histories(8, 4);
    let (a, b) = (key(1, 1), key(2, 1));
    for k in [&a, &b] {
        h.reset(k, 1, 3600);
    }
    let start = now();
    h.push(&a, 1, change(1, 4));
    h.push(&b, 1, change(2, 4));
    let (first, second) = (claim_all(&mut h, &a), claim_all(&mut h, &b));
    h.lodge(first, start + Duration::from_secs(20));
    h.lodge(second, start + Duration::from_secs(5));
    assert_eq!(h.next_deadline(), Some(start + Duration::from_secs(5)));
    advance(Duration::from_secs(5));
    // The later claim's deadline is still ahead, beside the context's own.
    assert_eq!(
        h.take_due(now()),
        (Vec::new(), Some(start + Duration::from_secs(20)))
    );
    assert_eq!(seconds(&h.drain(&b, 1).1), [2], "only the one past its own");
    assert!(h.drain(&a, 1).1.is_empty());
    advance(Duration::from_secs(15));
    h.take_due(now());
    assert_eq!(seconds(&h.drain(&a, 1).1), [1]);
}

#[test]
fn an_unsettled_claim_of_a_removed_reference_leaves_nothing_behind() {
    let (mut h, k) = one_reference();
    h.push(&k, 1, change(1, 4));
    let claim = claim_all(&mut h, &k);
    h.lodge(claim, now() + Duration::from_secs(15));
    h.remove(&k);
    advance(Duration::from_secs(15));
    assert_eq!(h.take_due(now()), (Vec::new(), None));
    assert_eq!(h.held(), (0, 0));
    assert_eq!(h.next_deadline(), None);
    assert_eq!(dropped(&h), 0, "a cancelled reference's changes just go");
}

#[test]
fn an_unsettled_owing_claim_owes_its_untimestamped_references_from_its_deadline() {
    let mut h = histories(8, 4);
    let k = key(1, 1);
    h.reset_untimed(&k, 1, 10);
    let mut claim = TimedClaim::new();
    claim.add_untimed(k.clone(), 1, None);
    let deadline = now() + Duration::from_secs(15);
    h.lodge(claim.owing(), deadline);
    advance(Duration::from_secs(15));
    assert_eq!(
        h.take_due(now()),
        (Vec::new(), Some(deadline + DELAY)),
        "owed from the deadline, due a delay later"
    );
    assert_eq!(h.take_owed(&k, 1), Some(deadline));
}
