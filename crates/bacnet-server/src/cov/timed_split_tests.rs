//! Splitting claims, the per-context bound, send turns and the subscriber's
//! maximum APDU (#986), owed untimestamped references (#1038), and changes
//! sent one value per notification (#1090).
use super::tests::{
    change, context, dropped, frame, histories, item_len, key, seconds, store, subscriber_for,
    timed_reference, LIFETIME,
};
use super::*;

/// A claim of `a`'s changes at seconds 1, 2, 3 and `b`'s at 9 and 10, each
/// sequenced by its second.
fn two_reference_claim(store: &TimedStore) -> (TimedClaim, CovSubscriptionKey, CovSubscriptionKey) {
    let (a, b) = (key(1, 1), key(1, 2));
    let sequenced = |seconds: &[u8]| {
        seconds
            .iter()
            .map(|&second| {
                let mut change = change(second, 4);
                change.seq = u64::from(second);
                change
            })
            .collect::<Vec<_>>()
    };
    let mut claim = TimedClaim::new(store.clone());
    claim.add(a.clone(), 1, sequenced(&[1, 2, 3]));
    claim.add(b.clone(), 1, sequenced(&[9, 10]));
    (claim, a, b)
}

fn claimed_seconds(claim: &TimedClaim) -> Vec<u8> {
    claim
        .in_order()
        .iter()
        .map(|(_, c)| c.frame().local_time.second)
        .collect()
}

#[test]
fn splitting_moves_the_oldest_changes_in_capture_order_latest_ones_included() {
    let (store, counters) = store(8, 4);
    let (mut claim, _, b) = two_reference_claim(&store);
    assert_eq!(claimed_seconds(&claim), [1, 2, 3, 9, 10]);
    assert!(claimed_seconds(&claim.split_oldest(0)).is_empty());

    let part = claim.split_oldest(2);
    assert_eq!(claimed_seconds(&part), [1, 2]);
    assert_eq!(claimed_seconds(&claim), [3, 9, 10]);

    // `a`'s latest change goes with `b`'s older one: capture order across
    // the claim decides, not whether a change is a reference's latest (#1008).
    let next = claim.split_oldest(2);
    assert_eq!(claimed_seconds(&next), [3, 9]);
    assert_eq!(
        next.last_changes()
            .map(|(_, c)| c.frame().local_time.second)
            .collect::<Vec<_>>(),
        [3, 9]
    );
    assert_eq!(
        claim
            .last_changes()
            .map(|(key, c)| (key.clone(), c.frame()))
            .collect::<Vec<_>>(),
        [(b, frame(10))],
        "`a` has nothing left here"
    );

    // Asking for more than there is moves everything.
    let rest = claim.split_oldest(5);
    assert_eq!(claimed_seconds(&rest), [10]);
    assert!(claimed_seconds(&claim).is_empty());
    drop((part, next, rest, claim));
    assert_eq!(dropped(&counters), 0, "splitting drops nothing");
}

#[test]
fn split_parts_retire_and_return_on_their_own_without_drops() {
    let (store, counters) = store(8, 4);
    let k = key(1, 1);
    store.lock().reset(&k, 1, 0);
    for second in 1..=3 {
        store.lock().push(&k, 1, change(second, 4));
    }
    let mut claim = TimedClaim::new(store.clone());
    let (incarnation, drained) = store.lock().drain(&k, 1);
    claim.add(k.clone(), incarnation, drained);
    let first = claim.split_oldest(1);
    // A confirmed report sends the first part and returns the rest.
    drop(claim);
    first.commit();
    assert_eq!(seconds(&store.lock().drain(&k, 1).1), [2, 3]);

    // An unconfirmed report retires each part it sends; a later part that
    // fails returns only itself.
    store.lock().push(&k, 1, change(4, 4));
    store.lock().push(&k, 1, change(5, 4));
    let mut claim = TimedClaim::new(store.clone());
    let (incarnation, drained) = store.lock().drain(&k, 1);
    claim.add(k.clone(), incarnation, drained);
    let first = claim.split_oldest(1);
    first.commit();
    drop(claim);
    assert_eq!(seconds(&store.lock().drain(&k, 1).1), [5]);
    assert_eq!(dropped(&counters), 0);
}

#[test]
fn the_bound_spans_several_notifications_of_the_smaller_apdu_less_a_reserve() {
    let (mut h, counters) = histories(8, 4);
    let k = key(1, 1);
    h.reset(&k, 1, 0);
    for second in 1..=8 {
        h.push(&k, 1, change(second, 4));
    }
    assert_eq!(dropped(&counters), 0, "eight changes fit the local bound");
    h.drain(&k, 1);

    // A subscriber with a smaller APDU shrinks the room to two changes.
    h.set_sizing(&context(1), Some(subscriber_for(2, 4)), LIFETIME);
    for second in 11..=13 {
        h.push(&k, 1, change(second, 4));
    }
    assert_eq!(seconds(&h.drain(&k, 1).1), [12, 13]);
    assert_eq!(dropped(&counters), 1);

    // A larger one cannot exceed the local maximum, nor can an unknown one.
    for subscriber in [Some(u16::MAX), None] {
        h.set_sizing(&context(1), subscriber, LIFETIME);
        for second in 21..=29 {
            h.push(&k, 1, change(second, 4));
        }
        assert_eq!(h.drain(&k, 1).1.len(), 8, "{subscriber:?}");
    }
    assert_eq!(dropped(&counters), 3);

    // Room the untimestamped values took is kept for them.
    h.set_sizing(&context(1), Some(subscriber_for(8, 4)), LIFETIME);
    h.note_reserve(&context(1), item_len(4));
    for second in 31..=38 {
        h.push(&k, 1, change(second, 4));
    }
    assert_eq!(seconds(&h.drain(&k, 1).1), [32, 33, 34, 35, 36, 37, 38]);
    assert_eq!(dropped(&counters), 4);
    // At most one notification's worth: two changes here.
    h.note_reserve(&context(1), 10 * item_len(4));
    for second in 41..=47 {
        h.push(&k, 1, change(second, 4));
    }
    assert_eq!(h.drain(&k, 1).1.len(), 6);
    assert_eq!(dropped(&counters), 5);
    // An admission starts the reserve over.
    h.set_sizing(&context(1), Some(subscriber_for(8, 4)), LIFETIME);
    for second in 51..=58 {
        h.push(&k, 1, change(second, 4));
    }
    assert_eq!(h.drain(&k, 1).1.len(), 8);
    assert_eq!(dropped(&counters), 5);
    // The reserve keeps room in notifications, not memory: at the local
    // maximum, where the memory ceiling binds, it changes nothing (#1287).
    h.set_sizing(&context(1), None, LIFETIME);
    h.note_reserve(&context(1), item_len(4));
    for second in 61..=68 {
        h.push(&k, 1, change(second, 4));
    }
    assert_eq!(h.drain(&k, 1).1.len(), 8);
    assert_eq!(dropped(&counters), 5);
}

#[test]
fn the_reserve_belongs_to_the_context_and_starts_over_when_it_loses_a_reference() {
    let (mut h, counters) = histories(8, 4);
    let (a, b) = (key(1, 1), key(1, 2));
    h.reset(&a, 1, 0);
    h.set_sizing(&context(1), Some(subscriber_for(8, 4)), LIFETIME);
    h.note_reserve(&context(1), item_len(4));
    h.reset(&b, 1, 0); // a later reference shares the context's reserve
    for second in 1..=8 {
        h.push(&b, 1, change(second, 4));
    }
    assert_eq!(
        dropped(&counters),
        1,
        "seven changes fit beside the reserve"
    );
    h.drain(&b, 1);
    h.remove(&a); // the context lost a reference: its reserve starts over
    for second in 11..=18 {
        h.push(&b, 1, change(second, 4));
    }
    assert_eq!(dropped(&counters), 1);
    h.remove(&b);
    assert_eq!(h.held(), (0, 0), "the last reference took the terms along");
}

#[test]
fn a_send_turn_holds_back_other_reports_and_owes_one_follow_up() {
    let (store, _) = store(8, 4);
    let revisits = Arc::new(crate::cov::CovRevisits::default());
    let k = key(1, 1);
    let begin = |context: MultipleContextKey, keys: Vec<CovSubscriptionKey>| {
        SendTurn::begin(&store, &context, &revisits, keys)
    };
    let turn = begin(context(1), vec![k.clone()]).expect("free");
    assert!(
        begin(context(1), vec![k.clone()]).is_none(),
        "one at a time"
    );
    drop(begin(context(2), vec![key(2, 1)]).expect("per context"));
    assert!(
        revisits.queued().is_empty(),
        "nobody stood back from context 2"
    );
    drop(turn);
    assert_eq!(
        revisits.queued(),
        std::collections::HashSet::from([k.clone()]),
        "the report that stood back gets one follow-up"
    );
    drop(begin(context(1), vec![k.clone()]).expect("free again"));
}

#[test]
fn deferred_parts_return_without_eviction_and_discarded_ones_are_counted() {
    let (store, counters) = store(2, 4);
    let k = key(1, 1);
    store.lock().reset(&k, 1, 0);
    store.lock().push(&k, 1, change(1, 4));
    store.lock().push(&k, 1, change(2, 4));
    let mut claim = TimedClaim::new(store.clone());
    let (incarnation, drained) = store.lock().drain(&k, 1);
    claim.add(k.clone(), incarnation, drained);
    store.lock().push(&k, 1, change(3, 4));
    // Back over the bound, but a deferred part is not evicted.
    drop(claim.without_eviction());
    assert_eq!(dropped(&counters), 0);
    let (incarnation, drained) = store.lock().drain(&k, 1);
    assert_eq!(seconds(&drained), [1, 2, 3]);
    let mut claim = TimedClaim::new(store.clone());
    claim.add(k.clone(), incarnation, drained);
    // A change whose one value fits no notification even alone is given up.
    let part = claim.split_oldest(1);
    assert!(part.split_values(|_, _| ValueFit::TooLarge).is_empty());
    assert_eq!(dropped(&counters), 1);
    drop(claim);
    assert_eq!(seconds(&store.lock().drain(&k, 1).1), [2, 3]);
}

#[test]
fn an_admission_without_a_known_maximum_apdu_keeps_the_one_advertised_before() {
    let mut table = crate::cov::CovSubscriptionTable::new();
    let route = crate::cov::SubscriberEndpoint::new(&[10, 0, 0, 1, 0xBA, 0xC0], None);
    let expires = crate::runtime_clock::now() + std::time::Duration::from_secs(300);
    let sub = timed_reference(1, expires);
    let advertised = |table: &crate::cov::CovSubscriptionTable| {
        table
            .get_subscription(&sub.key().unwrap())
            .and_then(|entry| entry.subscriber_max_apdu())
    };
    table
        .subscribe_multiple(
            &context(1),
            &route,
            expires,
            10,
            Some(206),
            vec![sub.clone()],
        )
        .unwrap();
    assert_eq!(advertised(&table), Some(206));
    table
        .subscribe_multiple(&context(1), &route, expires, 10, None, Vec::new())
        .unwrap();
    assert_eq!(advertised(&table), Some(206), "an unknown one keeps it");
    table
        .subscribe_multiple(&context(1), &route, expires, 10, Some(480), Vec::new())
        .unwrap();
    assert_eq!(advertised(&table), Some(480), "a known one replaces it");
}

#[test]
fn an_undelivered_untimestamped_reference_is_owed_once_its_report_began() {
    let (store, counters) = store(8, 4);
    let oversized = || {
        counters
            .untimed_references_oversized
            .load(std::sync::atomic::Ordering::Relaxed)
    };
    let (a, b) = (key(1, 1), key(1, 2));
    store.lock().reset_untimed(&a, 1, 10);
    store.lock().reset_untimed(&b, 1, 10);
    let claim_of = |entries: &[(&CovSubscriptionKey, u64, Option<Instant>)]| {
        let mut claim = TimedClaim::new(store.clone());
        for &(key, generation, owed) in entries {
            claim.add_untimed(key.clone(), generation, owed);
        }
        claim
    };
    // A report none of which went out owes nothing: like any lost
    // notification, the reference's next fanout reports it.
    drop(claim_of(&[(&a, 1, None)]));
    assert_eq!(store.lock().take_owed(&a, 1), None);
    // A part deferred behind a delivered one owes its references.
    let start = Instant::now();
    let mut claim = claim_of(&[(&a, 1, None), (&b, 1, None)]);
    let deferred = claim.split_untimed(&std::collections::HashSet::from([b.clone()]));
    claim.commit();
    drop(deferred.owing());
    assert_eq!(store.lock().take_owed(&a, 1), None, "delivered");
    let since = store.lock().take_owed(&b, 1).expect("owed");
    assert!(since >= start);
    assert_eq!(store.lock().take_owed(&b, 1), None, "taking settles it");
    // Once owed, a reference stays owed from when it first was until a part
    // carrying it is delivered, whichever report takes it.
    drop(claim_of(&[(&b, 1, Some(since))]));
    assert_eq!(store.lock().take_owed(&b, 1), Some(since));
    // Values that fit no notification are given up and counted, not owed
    // (#1066); nothing else above counted.
    assert_eq!(oversized(), 0);
    let mut claim = claim_of(&[(&b, 1, Some(since))]);
    claim.forgo_untimed();
    assert_eq!(oversized(), 1, "one reference left out");
    claim.forgo_untimed();
    assert_eq!(oversized(), 1, "counted once");
    drop(claim.owing());
    assert_eq!(store.lock().take_owed(&b, 1), None);
    assert_eq!(dropped(&counters), 0, "no timestamped change dropped");
    // A renewed or cancelled reference owes nothing.
    let claim = claim_of(&[(&a, 1, None), (&b, 1, Some(since))]);
    store.lock().reset_untimed(&b, 2, 10);
    store.lock().remove(&a);
    drop(claim.owing());
    assert_eq!(store.lock().take_owed(&b, 2), None);
    assert_eq!(store.lock().held(), (1, 0), "only the renewed reference");
}

#[tokio::test(start_paused = true)]
async fn an_owed_reference_makes_its_context_due_like_a_pending_change() {
    let (mut h, _) = histories(8, 4);
    let k = key(1, 1);
    h.reset_untimed(&k, 1, 10);
    assert_eq!(h.take_due(Instant::now()), (Vec::new(), None));
    let start = Instant::now();
    h.owe(&k, 1, start);
    let delay = Duration::from_secs(10);
    assert_eq!(
        h.take_due(Instant::now()),
        (Vec::new(), Some(start + delay))
    );
    tokio::time::advance(delay).await;
    assert_eq!(h.take_due(Instant::now()).0, std::slice::from_ref(&k));
    // Blocked again: a hold-off moves the next attempt, and re-enabled
    // communication ends the wait.
    let until = Instant::now() + Duration::from_secs(3);
    h.hold_until(&context(1), until);
    assert_eq!(h.take_due(Instant::now()), (Vec::new(), Some(until)));
    h.rearm();
    assert_eq!(h.take_due(Instant::now()).0, std::slice::from_ref(&k));
    // A later mark keeps the earlier one, and taking the mark settles it.
    h.owe(&k, 1, Instant::now());
    assert_eq!(h.take_owed(&k, 1), Some(start));
    assert_eq!(h.take_due(Instant::now()), (Vec::new(), None));
    assert_eq!(h.held(), (1, 0), "a settled reference holds no wait");
    h.owe(&k, 1, start);
    h.remove(&k);
    assert_eq!(
        h.held(),
        (0, 0),
        "removal takes the mark and the wait along"
    );
}

/// A change at `second` with one value per entry of `payloads`, of that many
/// octets each, under properties 1, 2, ... in order.
fn change_of(second: u8, payloads: &[usize]) -> TimedChange {
    let values = payloads
        .iter()
        .zip(1..)
        .map(|(&payload, property)| COVNotificationValue {
            property_identifier: bacnet_types::enums::PropertyIdentifier::from_raw(property),
            property_array_index: None,
            value: vec![0; payload],
            time_of_change: None,
        })
        .collect();
    TimedChange::new(
        frame(second),
        values,
        change(second, 0).observation().clone(),
    )
}

/// Value sizes of each change, in order.
fn payloads(changes: &[TimedChange]) -> Vec<Vec<usize>> {
    changes
        .iter()
        .map(|change| change.values().iter().map(|v| v.value.len()).collect())
        .collect()
}

/// A claim of everything `k` has queued.
fn claim_all(store: &TimedStore, k: &CovSubscriptionKey) -> TimedClaim {
    let mut claim = TimedClaim::new(store.clone());
    let (incarnation, drained) = store.lock().drain(k, 1);
    claim.add(k.clone(), incarnation, drained);
    claim
}

#[test]
fn value_parts_keep_capture_order_and_rejoin_whatever_order_they_return_in() {
    let (store, counters) = store(8, 4);
    let (a, b) = (key(1, 1), key(1, 2));
    for k in [&a, &b] {
        store.lock().reset(k, 1, 0);
    }
    store.lock().push(&a, 1, change_of(1, &[4, 5, 6]));
    store.lock().push(&b, 1, change_of(2, &[7]));
    let queued = store.lock().context_held.get(&context(1)).copied();
    let mut claim = claim_all(&store, &a);
    let (incarnation, drained) = store.lock().drain(&b, 1);
    claim.add(b.clone(), incarnation, drained);
    let parts = claim.split_values(|_, _| ValueFit::Fits);
    // One value each: capture order across references, then each change's
    // own order, every value keeping its change's time (#1090).
    let carried: Vec<_> = parts
        .iter()
        .map(|part| {
            let changes = part.in_order();
            assert_eq!(changes.len(), 1);
            let (key, change) = changes[0];
            assert_eq!(change.values().len(), 1);
            let second = change.frame().local_time.second;
            assert_eq!(
                change.values()[0].time_of_change,
                Some(frame(second).local_time)
            );
            (key.clone(), second, change.values()[0].value.len())
        })
        .collect();
    assert_eq!(
        carried,
        [
            (a.clone(), 1, 4),
            (a.clone(), 1, 5),
            (a.clone(), 1, 6),
            (b.clone(), 2, 7),
        ]
    );
    // Returned newest first, `a`'s parts rejoin as its one change, in the
    // order captured, counting against the bound what it counted before.
    for part in parts.into_iter().rev() {
        drop(part);
    }
    assert_eq!(store.lock().context_held.get(&context(1)).copied(), queued);
    // Returned oldest first, they rejoin the same way.
    for part in claim_all(&store, &a).split_values(|_, _| ValueFit::Fits) {
        drop(part);
    }
    assert_eq!(store.lock().context_held.get(&context(1)).copied(), queued);
    assert_eq!(payloads(&store.lock().drain(&a, 1).1), [vec![4, 5, 6]]);
    assert_eq!(payloads(&store.lock().drain(&b, 1).1), [vec![7]]);
    assert_eq!(dropped(&counters), 0, "nothing was lost");
}

#[test]
fn value_parts_rejoin_in_captured_order_from_any_return_order() {
    let (store, counters) = store(8, 4);
    let k = key(1, 1);
    store.lock().reset(&k, 1, 0);
    let whole = change_of(1, &[4, 5, 6]);
    let held = |store: &TimedStore| store.lock().context_held.get(&context(1)).copied();
    let one = |change: &TimedChange| Held::of([change]);
    // Every order three parts can come back in, the middle part last
    // included: each time they rejoin as the change, in captured order, and
    // count against the bound exactly what the change counted.
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        store.lock().push(&k, 1, whole.clone());
        let mut parts: Vec<_> = claim_all(&store, &k)
            .split_values(|_, _| ValueFit::Fits)
            .into_iter()
            .map(Some)
            .collect();
        for at in order {
            drop(parts[at].take());
        }
        assert_eq!(held(&store), Some(one(&whole)), "{order:?}");
        let rejoined = store.lock().drain(&k, 1).1;
        assert_eq!(payloads(&rejoined), [vec![4, 5, 6]], "{order:?}");
        assert_eq!(rejoined[0].octets, whole.octets, "{order:?}");
        assert_eq!(held(&store), None, "{order:?}");
    }
    // After the first value went out, the other two coming back last first
    // still rejoin in order, counting only what is left.
    store.lock().push(&k, 1, whole.clone());
    let mut parts = claim_all(&store, &k)
        .split_values(|_, _| ValueFit::Fits)
        .into_iter();
    let (first, second, third) = (
        parts.next().unwrap(),
        parts.next().unwrap(),
        parts.next().unwrap(),
    );
    first.commit();
    drop(third);
    drop(second);
    let rest_held = held(&store).expect("the rest is queued");
    let rest = store.lock().drain(&k, 1).1;
    assert_eq!(payloads(&rest), [vec![5, 6]]);
    assert_eq!(one(&rest[0]), rest_held);
    assert_eq!(rest_held.octets, item_octets(rest[0].values()));
    assert_eq!(dropped(&counters), 0, "nothing was lost");
}

#[test]
fn only_the_part_with_a_changes_last_value_delivers_the_change() {
    let (store, counters) = store(8, 4);
    let k = key(1, 1);
    store.lock().reset(&k, 1, 0);
    store.lock().push(&k, 1, change_of(1, &[4, 5]));
    let committed = |store: &TimedStore| store.lock().histories[&k].committed;
    let mut parts = claim_all(&store, &k)
        .split_values(|_, _| ValueFit::Fits)
        .into_iter();
    let (first, second) = (parts.next().unwrap(), parts.next().unwrap());
    // The first value went out and the second did not: the change is not
    // delivered, and the second value returns as all that is left of it.
    first.commit();
    assert_eq!(committed(&store), 0);
    drop(second);
    let rest = store.lock().drain(&k, 1).1;
    assert_eq!(payloads(&rest), [vec![5]]);
    // Sent again, the rest fits on its own and delivers the change.
    let mut claim = TimedClaim::new(store.clone());
    claim.add(k.clone(), store.lock().histories[&k].incarnation, rest);
    let seq = claim.in_order()[0].1.seq();
    claim.commit();
    assert_eq!(committed(&store), seq);
    assert_eq!(dropped(&counters), 0);
}

#[test]
fn a_value_too_large_alone_is_dropped_once_per_change_and_the_rest_still_goes() {
    let (store, counters) = store(8, 4);
    let k = key(1, 1);
    store.lock().reset(&k, 1, 0);
    store.lock().push(&k, 1, change_of(1, &[100, 4, 100, 3]));
    let fit = |_: &CovSubscriptionKey, part: &TimedChange| match part.values()[0].value.len() {
        100 => ValueFit::TooLarge,
        3 => ValueFit::Empty,
        _ => ValueFit::Fits,
    };
    let parts = claim_all(&store, &k).split_values(fit);
    // Both oversized values are given up, counted once for their change; the
    // value that would carry nothing is left out, uncounted.
    assert_eq!(dropped(&counters), 1);
    assert_eq!(parts.len(), 1);
    let carried: Vec<_> = parts[0]
        .in_order()
        .iter()
        .map(|(_, c)| (*c).clone())
        .collect();
    assert_eq!(payloads(&carried), [vec![4]]);
    // The one value kept is the last sent, so it delivers the change.
    let seq = carried[0].seq();
    parts.into_iter().for_each(TimedClaim::commit);
    assert_eq!(store.lock().histories[&k].committed, seq);
    assert!(store.lock().drain(&k, 1).1.is_empty());
}

#[test]
fn the_bound_keeps_a_change_in_delivery_until_its_last_value_is_delivered() {
    // A bound of one single-value change: a two-value change exceeds it
    // alone, and so does any second change.
    let (store, counters) = store(1, 4);
    let (a, b) = (key(1, 1), key(1, 2));
    for k in [&a, &b] {
        store.lock().reset(k, 1, 0);
    }
    store.lock().push(&a, 1, change_of(1, &[4, 4]));
    let mut parts = claim_all(&store, &a)
        .split_values(|_, _| ValueFit::Fits)
        .into_iter();
    let (first, second) = (parts.next().unwrap(), parts.next().unwrap());
    // The first value is delivered and the second's send fails: what is left
    // of the change is in delivery, and newer changes do not evict it.
    first.commit();
    drop(second);
    store.lock().push(&a, 1, change(2, 4));
    store.lock().push(&b, 1, change(3, 4));
    assert_eq!(dropped(&counters), 0, "only changes eviction may not take");
    // A real overflow takes the change between the one in delivery and the
    // newest, and only that change is counted (#1163).
    store.lock().push(&a, 1, change(4, 4));
    assert_eq!(dropped(&counters), 1);
    let claim = claim_all(&store, &a);
    let held: Vec<_> = claim
        .in_order()
        .iter()
        .map(|(_, change)| (change.frame().local_time.second, change.values().len()))
        .collect();
    assert_eq!(held, [(1, 1), (4, 1)]);
    assert_eq!(seconds(&store.lock().drain(&b, 1).1), [3]);
    // Once its last value is delivered, the change keeps nothing back.
    claim.commit();
    for second in 5..=6 {
        store.lock().push(&a, 1, change(second, 4));
    }
    assert_eq!(seconds(&store.lock().drain(&a, 1).1), [6]);
    assert_eq!(dropped(&counters), 2);
}

#[test]
fn a_change_is_in_delivery_only_once_a_part_of_it_went_out() {
    let (store, counters) = store(1, 4);
    let k = key(1, 1);
    store.lock().reset(&k, 1, 0);
    // No part went out: the parts rejoin, and a newer change evicts the
    // change as before.
    store.lock().push(&k, 1, change_of(1, &[4, 4]));
    drop(claim_all(&store, &k).split_values(|_, _| ValueFit::Fits));
    store.lock().push(&k, 1, change(2, 4));
    assert_eq!(dropped(&counters), 1);
    assert_eq!(seconds(&store.lock().drain(&k, 1).1), [2]);
    // A confirmed report's first part going out puts its change in delivery
    // before the Ack: the deferred rest outlasts newer changes.
    store.lock().push(&k, 1, change_of(3, &[4, 4]));
    let mut parts = claim_all(&store, &k)
        .split_values(|_, _| ValueFit::Fits)
        .into_iter();
    let (first, second) = (parts.next().unwrap(), parts.next().unwrap());
    first.going_out();
    drop(second.without_eviction());
    store.lock().push(&k, 1, change(4, 4));
    store.lock().push(&k, 1, change_of(5, &[4, 4]));
    assert_eq!(dropped(&counters), 2, "only the change at 4");
    first.commit();
    // The next report delivers the rest, then sends the change at 5 value by
    // value; its first value delivered moves the mark to it.
    let mut claim = claim_all(&store, &k);
    assert_eq!(claimed_seconds(&claim), [3, 5]);
    claim.split_oldest(1).commit();
    let mut parts = claim.split_values(|_, _| ValueFit::Fits).into_iter();
    let (first, second) = (parts.next().unwrap(), parts.next().unwrap());
    first.commit();
    drop(second);
    store.lock().push(&k, 1, change(6, 4));
    assert_eq!(seconds(&store.lock().drain(&k, 1).1), [5, 6]);
    assert_eq!(dropped(&counters), 2);
}
