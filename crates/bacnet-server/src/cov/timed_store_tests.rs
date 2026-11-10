//! The server's store around the histories: its deadline wait, what its
//! table's admissions and removals leave in it, and the claims and turns
//! that settle themselves.
use std::sync::atomic::Ordering;

use bacnet_objects::clock::ClockFrame;
use bacnet_services::cov_multiple::COVNotificationValue;
use bacnet_types::MacAddr;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};

use super::*;
use crate::cov::CovSubscriptionTable;
use bacnet_device_core::cov::timed::TimedClaim;
use bacnet_device_core::cov::{
    CovNotificationKind, CovObservation, CovRecipient, CovSample, CovSubscription,
    SubscriberEndpoint,
};
use bacnet_time::{Duration, Instant};

fn context(process_id: u32) -> MultipleContextKey {
    MultipleContextKey {
        recipient: CovRecipient::Direct(MacAddr::from_slice(&[10, 0, 0, 1, 0xBA, 0xC0])),
        process_id,
        confirmed: false,
    }
}

fn key(process_id: u32, instance: u32) -> CovSubscriptionKey {
    CovSubscriptionKey::Multiple {
        context: context(process_id),
        object: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap(),
        property: PropertyIdentifier::PRESENT_VALUE,
        index: None,
    }
}

/// A change of one four-octet value, captured now at `second`.
fn change(second: u8) -> TimedChange {
    let frame = ClockFrame {
        local_date: Date {
            year: 126,
            month: 9,
            day: 29,
            day_of_week: 2,
        },
        local_time: Time {
            hour: 14,
            minute: 0,
            second,
            hundredths: 0,
        },
        utc_offset: 0,
        daylight_savings_status: false,
    };
    let sample = CovSample::new(&PropertyValue::Real(f32::from(second))).unwrap();
    TimedChange::new(
        frame,
        vec![COVNotificationValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0; 4],
            time_of_change: None,
        }],
        CovObservation::new(sample, None).unwrap(),
        runtime_clock::core_now(),
    )
}

fn seconds(changes: &[TimedChange]) -> Vec<u8> {
    changes
        .iter()
        .map(|change| change.frame().local_time.second)
        .collect()
}

fn store() -> (HistoryStore, Arc<AtomicCovCounters>) {
    let counters = Arc::new(AtomicCovCounters::default());
    (HistoryStore::new(1476, Arc::clone(&counters)), counters)
}

/// A claim of every pending change of `k`'s generation 1.
fn claim_all(store: &HistoryStore, k: &CovSubscriptionKey) -> ClaimGuard {
    let mut claim = TimedClaim::new();
    let (incarnation, drained) = store.lock().drain(k, 1);
    claim.add(k.clone(), incarnation, drained);
    ClaimGuard::of(store, claim)
}

/// [`claim_all`], lodged until `deadline` as a confirmed report's is: by
/// the histories, with the worker's handle over the lodged claim.
fn lodge_all(store: &HistoryStore, k: &CovSubscriptionKey, deadline: Instant) -> LodgedClaim {
    let claim = claim_all(store, k).into_plain();
    let id = store.lock().lodge(claim, deadline);
    LodgedClaim::of(store, id)
}

fn timed_reference(process_id: u32, expires_at: bacnet_time::Instant) -> CovSubscription {
    CovSubscription {
        subscriber_mac: MacAddr::from_slice(&[10, 0, 0, 1, 0xBA, 0xC0]),
        subscriber_network: None,
        subscriber_process_identifier: process_id,
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
        issue_confirmed_notifications: false,
        expires_at: Some(expires_at),
        last_notified_observation: None,
        monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: CovNotificationKind::Multiple,
        timestamped: true,
    }
}

#[test]
fn empty_admissions_of_unknown_contexts_leave_no_deadline_state() {
    let mut table = CovSubscriptionTable::new();
    let route = SubscriberEndpoint::new(&[10, 0, 0, 1, 0xBA, 0xC0], None);
    let expires = runtime_clock::core_now() + Duration::from_secs(300);
    // A peer varying process and form across empty-list requests.
    for process_id in 0..64 {
        for confirmed in [false, true] {
            let context = MultipleContextKey {
                confirmed,
                ..context(process_id)
            };
            let accepted = table
                .subscribe_multiple(&context, &route, expires, 10, None, Vec::new())
                .unwrap();
            assert!(accepted.is_empty());
        }
    }
    assert_eq!(table.timed().lock().held(), (0, 0));
}

#[tokio::test(start_paused = true)]
async fn cancel_and_expiry_take_a_contexts_deadline_state_along() {
    let far = runtime_clock::core_now() + Duration::from_secs(300);
    for expire in [false, true] {
        let mut table = CovSubscriptionTable::new();
        let sub = table.admit_for_test(timed_reference(1, far), 10).unwrap();
        table.timed().lock().hold_until(
            &context(1),
            runtime_clock::core_now() + Duration::from_secs(5),
        );
        assert_eq!(table.timed().lock().held(), (1, 1));
        if expire {
            table.expire_all_for_test();
            table.purge_expired();
        } else {
            assert!(table.unsubscribe(sub.key()));
        }
        assert_eq!(table.timed().lock().held(), (0, 0), "expire={expire}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_queued_change_wakes_the_deadline_wait() {
    let (store, _) = store();
    let k = key(1, 1);
    store.lock().reset(&k, 1, 0);
    store.lock().set_delay(&context(1), 2);
    let waiter = tokio::spawn({
        let store = store.clone();
        async move { store.next_due().await }
    });
    tokio::task::yield_now().await;
    let start = runtime_clock::core_now();
    store.lock().push(&k, 1, change(1));
    assert_eq!(waiter.await.unwrap(), [k]);
    assert_eq!(runtime_clock::core_now() - start, Duration::from_secs(2));
}

#[test]
fn a_send_turn_holds_back_other_reports_and_owes_one_follow_up() {
    let (store, _) = store();
    let revisits = Arc::new(crate::cov::CovRevisits::default());
    let k = key(1, 1);
    let begin = |context: MultipleContextKey, keys: Vec<CovSubscriptionKey>| {
        // As the fanout does: the core takes the turn, a guard holds it.
        let taken = store.lock().begin_send(&context);
        taken.then(|| SendTurn::held(&store, &context, &revisits, keys))
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
fn an_admission_without_a_known_maximum_apdu_keeps_the_one_advertised_before() {
    let mut table = CovSubscriptionTable::new();
    let route = SubscriberEndpoint::new(&[10, 0, 0, 1, 0xBA, 0xC0], None);
    let expires = runtime_clock::core_now() + Duration::from_secs(300);
    let sub = timed_reference(1, expires);
    let advertised = |table: &CovSubscriptionTable| {
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
fn admission_sizes_the_envelope_from_the_lifetime_left() {
    let route = SubscriberEndpoint::new(&[10, 0, 0, 1, 0xBA, 0xC0], None);
    for (lifetime, envelope) in [(120, 25), (300, 26), (70_000, 27)] {
        let mut table = CovSubscriptionTable::new();
        let expires = runtime_clock::core_now() + Duration::from_secs(lifetime);
        let reference = timed_reference(1, expires);
        table
            .subscribe_multiple(&context(1), &route, expires, 10, Some(50), vec![reference])
            .unwrap();
        assert_eq!(
            table.timed().lock().sizing_for_test(&context(1)),
            Some((envelope, 50 - envelope)),
            "lifetime {lifetime}"
        );
    }
}

#[test]
fn a_dropped_claim_gives_its_changes_back_and_a_committed_one_retires_them() {
    let (store, _) = store();
    let k = key(1, 1);
    store.lock().reset(&k, 1, 0);
    store.lock().push(&k, 1, change(1));
    store.lock().push(&k, 1, change(2));
    // Any early return, failed send or aborted task drops the claim.
    drop(claim_all(&store, &k));
    let claim = claim_all(&store, &k);
    assert_eq!(claim.in_order().len(), 2, "both came back");
    claim.commit();
    assert!(store.lock().drain(&k, 1).1.is_empty(), "delivered");
}

#[test]
fn the_guard_publishes_what_a_section_counted() {
    let (store, counters) = store();
    let (a, b) = (key(1, 1), key(1, 2));
    store.lock().reset_untimed(&a, 1, 10);
    store.lock().reset(&b, 1, 0);
    // An untimestamped reference whose values fit no notification.
    let mut claim = TimedClaim::new();
    claim.add_untimed(a.clone(), 1, None);
    store.lock().forgo_untimed(&mut claim);
    assert_eq!(
        counters
            .untimed_references_oversized
            .load(Ordering::Relaxed),
        1
    );
    // An older change returned after a newer one was delivered is dropped.
    store.lock().push(&b, 1, change(1));
    let older = claim_all(&store, &b);
    store.lock().push(&b, 1, change(2));
    claim_all(&store, &b).commit();
    let warnings = DropWarningCount::default();
    let guard = warnings.install();
    drop(older);
    drop(guard);
    assert_eq!(counters.timed_changes_dropped.load(Ordering::Relaxed), 1);
    assert_eq!(warnings.get(), 1, "its context's first for that cause");
}

#[tokio::test(start_paused = true)]
async fn a_lodged_claim_is_settled_by_its_worker_or_at_its_deadline() {
    let (store, _) = store();
    let k = key(1, 1);
    store.lock().reset(&k, 1, 0);
    store.lock().set_delay(&context(1), 60);
    let deadline = || runtime_clock::core_now() + Duration::from_secs(15);

    // Acknowledged: retired.
    store.lock().push(&k, 1, change(1));
    lodge_all(&store, &k, deadline()).commit();
    assert!(store.lock().drain(&k, 1).1.is_empty());

    // Its worker ended any other way, or was aborted: returned at once.
    store.lock().push(&k, 1, change(2));
    drop(lodge_all(&store, &k, deadline()));
    assert_eq!(seconds(&store.lock().drain(&k, 1).1), [2]);

    // A worker that never settles: the deadline wait gives the claim back,
    // and its late Ack retires nothing.
    store.lock().push(&k, 1, change(3));
    let lodged = lodge_all(&store, &k, deadline());
    let waiter = tokio::spawn({
        let store = store.clone();
        async move { store.next_due().await }
    });
    tokio::time::advance(std::time::Duration::from_secs(14)).await;
    tokio::task::yield_now().await;
    assert!(store.lock().drain(&k, 1).1.is_empty(), "still its worker's");
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    lodged.commit();
    assert_eq!(seconds(&store.lock().drain(&k, 1).1), [3], "given back");
    waiter.abort();
}

/// The histories report a context's first drop for a cause and 64 repeats in
/// all for one section; the guard's one line for the rest counts exactly the
/// changes no report names.
#[test]
fn a_sections_drops_past_the_kept_reports_are_counted_for_one_line() {
    // A device so small that its ceiling holds two of these changes.
    let small = HistoryStore::new(60, Arc::new(AtomicCovCounters::default()));
    let k = key(1, 1);
    let mut histories = small.lock();
    histories.reset(&k, 1, 0);
    for second in 0..200 {
        histories.push(&k, 1, change(second % 60));
    }
    let (counts, drops) = (histories.take_counts(), histories.take_drops());
    assert_eq!(counts.changes_dropped, 198, "all but the two it holds");
    assert_eq!(drops.len(), 64, "the first and the repeats kept");
    assert!(drops.iter().all(|dropped| dropped.count == 1));
    assert_eq!(unreported(&counts, &drops), 198 - 64);
    // A section whose reports name every drop has no such line.
    histories.push(&k, 1, change(1));
    let (counts, drops) = (histories.take_counts(), histories.take_drops());
    assert_eq!((counts.changes_dropped, drops.len()), (1, 1));
    assert_eq!(unreported(&counts, &drops), 0);
}
