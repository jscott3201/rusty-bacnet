//! `subscribe_multiple` admits proposals in request order and stops at the
//! first one past a subscription cap (Clause 13.16.2; #1058, #1059).
use super::*;
use bacnet_time::Instant;

/// A Priority_Array element proposal of the usual context.
fn element(index: u32, increment: f32, expiry: Instant) -> CovSubscription {
    let mut sub = proposal(Some(index), false);
    sub.cov_increment = Some(increment);
    sub.expires_at = Some(expiry);
    sub
}

#[test]
fn cov_multiple_admission_stops_at_the_first_proposal_past_the_quota() {
    let mut table = CovSubscriptionTable::with_policy(
        CovPolicy {
            max_subscriptions_per_peer: 3,
            ..CovPolicy::default()
        },
        Arc::new(AtomicCovCounters::default()),
    );
    let soon = crate::runtime_clock::core_now() + Duration::from_secs(60);
    let first = table.admit_for_test(element(0, 0.5, soon), 1).unwrap();
    let context = first.key().multiple_context().unwrap().clone();
    let route = first.endpoint();
    let expiry = crate::runtime_clock::core_now() + Duration::from_secs(600);

    // Renewing element 0 and repeating element 1 add no subscription, so
    // element 3 is the fourth of a three-per-peer quota.
    let refusal = table
        .subscribe_multiple(
            &context,
            &route,
            expiry,
            2,
            None,
            vec![
                element(0, 1.0, expiry),
                element(1, 1.0, expiry),
                element(1, 2.0, expiry),
                element(2, 1.0, expiry),
                element(3, 1.0, expiry),
                element(1, 9.0, expiry),
            ],
        )
        .unwrap_err();
    resource_error(refusal.error);
    assert_eq!(refusal.refused, Some(4));
    // The final duplicate before the refused proposal wins; the repeat after
    // it is never processed.
    let committed: Vec<_> = refusal
        .committed
        .iter()
        .map(|sub| (sub.monitored_property_array_index, sub.cov_increment))
        .collect();
    assert_eq!(
        committed,
        [
            (Some(0), Some(1.0)),
            (Some(1), Some(2.0)),
            (Some(2), Some(1.0))
        ]
    );
    assert_eq!(table.len(), 3);
    assert!(refusal.committed.iter().all(|sub| table.is_current(sub)));
    assert!(!table.is_current(&first));
    // The kept proposals renewed the context as a whole.
    let terms = |table: &CovSubscriptionTable| {
        table
            .multiple_context_references(&context)
            .map(|sub| (sub.expires_at, sub.max_notification_delay()))
            .collect::<Vec<_>>()
    };
    assert_eq!(terms(&table), [(Some(expiry), Some(2)); 3]);
    assert_eq!(table.counters.snapshot().subscriptions_rejected_quota, 1);

    // Refused at its first proposal, a request changes nothing: the renewal
    // after it is not processed and the context keeps its terms.
    let later = crate::runtime_clock::core_now() + Duration::from_secs(900);
    let refusal = table
        .subscribe_multiple(
            &context,
            &route,
            later,
            3,
            None,
            vec![element(3, 1.0, later), element(0, 4.0, later)],
        )
        .unwrap_err();
    resource_error(refusal.error);
    assert_eq!(refusal.refused, Some(0));
    assert!(refusal.committed.is_empty());
    assert_eq!(terms(&table), [(Some(expiry), Some(2)); 3]);
    assert_eq!(table.counters.snapshot().subscriptions_rejected_quota, 2);
}
