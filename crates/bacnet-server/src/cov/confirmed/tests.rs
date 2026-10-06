use super::*;
use crate::cov::{
    CovNotificationKind, CovObservation, CovSample, CovSubscription, SubscriberEndpoint,
};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bacnet_types::MacAddr;

fn proposal(kind: CovNotificationKind, property: PropertyIdentifier) -> CovSubscription {
    CovSubscription {
        subscriber_mac: MacAddr::from_slice(&[1]),
        subscriber_network: Some(NpduAddress {
            network: 7,
            mac_address: MacAddr::from_slice(&[9]),
        }),
        subscriber_process_identifier: 1,
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
        issue_confirmed_notifications: true,
        expires_at: Some(crate::runtime_clock::now() + Duration::from_secs(60)),
        last_notified_observation: None,
        monitored_property: Some(property),
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: kind,
        timestamped: false,
    }
}
fn single() -> CovSubscription {
    proposal(
        CovNotificationKind::Single,
        PropertyIdentifier::PRESENT_VALUE,
    )
}
fn reference(property: PropertyIdentifier) -> CovSubscription {
    proposal(CovNotificationKind::Multiple, property)
}
fn value(v: f32) -> CovObservation {
    CovObservation::new(CovSample::new(&PropertyValue::Real(v)).unwrap(), None).unwrap()
}
fn baseline(table: &CovSubscriptionTable, sub: &CovSubscriptionSnapshot) -> Option<CovObservation> {
    table
        .get_subscription(sub.key())
        .unwrap()
        .last_notified_observation
        .clone()
}
fn live(table: &CovSubscriptionTable, sub: &CovSubscriptionSnapshot) -> CovSubscriptionSnapshot {
    table.get_subscription(sub.key()).unwrap().clone()
}
fn ticket(sub: &CovSubscriptionSnapshot) -> PreparedCovCompletion {
    sub.prepare_completion().unwrap()
}
/// A two-reference confirmed context: Present_Value and Status_Flags.
fn context(table: &mut CovSubscriptionTable) -> (CovSubscriptionSnapshot, CovSubscriptionSnapshot) {
    let a = table
        .admit_for_test(reference(PropertyIdentifier::PRESENT_VALUE), 0)
        .unwrap();
    let b = table
        .admit_for_test(reference(PropertyIdentifier::STATUS_FLAGS), 0)
        .unwrap();
    (a, b)
}
fn refresh(
    table: &mut CovSubscriptionTable,
    of: &CovSubscriptionSnapshot,
    route: &SubscriberEndpoint,
    listed: Vec<CovSubscription>,
) {
    let context = of.key().multiple_context().unwrap().clone();
    let expires = listed
        .first()
        .and_then(|sub| sub.expires_at)
        .unwrap_or_else(|| crate::runtime_clock::now() + Duration::from_secs(120));
    table
        .subscribe_multiple(&context, route, expires, 5, None, listed)
        .unwrap();
}

#[test]
fn completion_needs_the_outstanding_report() {
    let mut table = CovSubscriptionTable::new();
    let sub = table.subscribe(single()).unwrap();
    let report = ticket(&sub);
    assert!(
        !table.complete_observation(&sub, report, value(10.0)),
        "no outstanding report, no completion"
    );
    let flight = table.begin_confirmed(report, [&sub]).unwrap();
    assert!(!table.confirmed_idle(&sub));
    let later = ticket(&sub);
    assert_eq!(
        table.begin_confirmed(later, [&sub]).unwrap_err(),
        BeginRefusal::Busy,
        "one outstanding report per subscription"
    );
    assert!(!table.complete_observation(&sub, later, value(20.0)));
    // Shutdown or cancellation: the baseline stays and there is no hold-off.
    drop(flight);
    assert_eq!(baseline(&table, &sub), None);
    assert!(table.confirmed_idle(&sub));
    assert!(!table.complete_observation(&sub, report, value(10.0)));

    let flight = table.begin_confirmed(later, [&sub]).unwrap();
    assert!(table.complete_observation(&sub, later, value(20.0)));
    assert!(!table.confirmed_idle(&sub), "the flight ends after the Ack");
    drop(flight);
    assert_eq!(baseline(&table, &sub), Some(value(20.0)));
    assert!(
        !table.confirmed_idle(&sub),
        "a snapshot older than the Ack no longer holds the live baseline"
    );
    assert_eq!(
        table.begin_confirmed(ticket(&sub), [&sub]).unwrap_err(),
        BeginRefusal::Busy
    );
    let current = live(&table, &sub);
    assert!(table.confirmed_idle(&current));
    drop(table.begin_confirmed(ticket(&current), [&current]).unwrap());
}

#[tokio::test(start_paused = true)]
async fn a_failed_report_holds_its_subscription_off() {
    let mut table = CovSubscriptionTable::new();
    let sub = table.subscribe(single()).unwrap();
    table
        .begin_confirmed(ticket(&sub), [&sub])
        .unwrap()
        .failed(Duration::from_millis(30));
    assert_eq!(baseline(&table, &sub), None);
    assert!(!table.confirmed_idle(&sub), "held off after the failure");
    assert_eq!(
        table.begin_confirmed(ticket(&sub), [&sub]).unwrap_err(),
        BeginRefusal::Busy
    );
    tokio::time::advance(Duration::from_millis(29)).await;
    assert!(!table.confirmed_idle(&sub));
    tokio::time::advance(Duration::from_millis(1)).await;
    assert!(table.confirmed_idle(&sub), "the hold-off has passed");
    let report = ticket(&sub);
    let flight = table.begin_confirmed(report, [&sub]).unwrap();
    assert!(table.complete_observation(&sub, report, value(1.0)));
    drop(flight);
    assert!(
        table.confirmed_idle(&live(&table, &sub)),
        "an Ack holds nothing off"
    );
}

#[test]
fn a_replaced_subscription_starts_unmarked() {
    let mut table = CovSubscriptionTable::new();
    let first = table.subscribe(single()).unwrap();
    let old = ticket(&first);
    let old_flight = table.begin_confirmed(old, [&first]).unwrap();
    let replacement = table.subscribe(single()).unwrap();
    assert!(table.confirmed_idle(&replacement));
    assert_eq!(
        table.begin_confirmed(ticket(&first), [&first]).unwrap_err(),
        BeginRefusal::NotCurrent
    );
    let new = ticket(&replacement);
    let new_flight = table.begin_confirmed(new, [&replacement]).unwrap();
    assert!(!table.complete_observation(&first, old, value(10.0)));
    old_flight.failed(Duration::from_secs(60));
    assert_eq!(baseline(&table, &replacement), None);
    assert!(
        !table.confirmed_idle(&replacement),
        "the stale report left the replacement's mark alone"
    );
    assert!(table.complete_observation(&replacement, new, value(20.0)));
    drop(new_flight);
    assert!(
        table.confirmed_idle(&live(&table, &replacement)),
        "nor did it hold the replacement off"
    );
}

#[test]
fn a_multiple_context_shares_one_mark() {
    let mut table = CovSubscriptionTable::new();
    let (a, b) = context(&mut table);
    let context = a.key().multiple_context().unwrap().clone();
    let solo = table.begin_confirmed(ticket(&a), [&a]).unwrap();
    assert!(!table.context_idle(&context, std::slice::from_ref(&b)));
    assert_eq!(
        table.begin_confirmed(ticket(&b), [&b]).unwrap_err(),
        BeginRefusal::Busy,
        "a sibling's report holds the whole context"
    );
    drop(solo);
    assert!(table.context_idle(&context, &[a.clone(), b.clone()]));
    let report = ticket(&b);
    let flight = table.begin_confirmed(report, [&a, &b]).unwrap();
    assert!(table.complete_observation(&a, report, value(1.0)));
    assert!(
        table.complete_observation(&b, report, value(2.0)),
        "every reference of one report completes under its ticket"
    );
    drop(flight);
    assert_eq!(baseline(&table, &a), Some(value(1.0)));
    assert_eq!(baseline(&table, &b), Some(value(2.0)));
    assert!(
        !table.context_idle(&context, std::slice::from_ref(&a)),
        "a snapshot older than the Ack waits for the follow-up"
    );
    assert!(table.context_idle(&context, &[live(&table, &a), live(&table, &b)]));
}

#[test]
fn a_route_change_fences_the_report_and_revisits_its_references() {
    let mut table = CovSubscriptionTable::new();
    let (a, b) = context(&mut table);
    let report = ticket(&a);
    let flight = table.begin_confirmed(report, [&a, &b]).unwrap();
    // Same-route expiry refresh keeps the report's authority.
    refresh(&mut table, &a, &a.endpoint(), vec![]);
    assert!(table.revisits().queued().is_empty());
    assert!(table.complete_observation(&a, report, value(1.0)));
    assert_eq!(baseline(&table, &a), Some(value(1.0)));
    let mut route = a.endpoint();
    route.mac = MacAddr::from_slice(&[2]);
    refresh(&mut table, &a, &route, vec![]);
    assert!(
        !table.complete_observation(&b, report, value(2.0)),
        "an old-route report cannot complete the moved reference"
    );
    assert_eq!(
        table.revisits().queued(),
        HashSet::from([a.key().clone(), b.key().clone()]),
        "the held changes get a fresh look on the new route"
    );
    let moved = live(&table, &b);
    assert!(
        table.confirmed_idle(&moved),
        "the new route starts unmarked"
    );
    drop(flight);
    // The report was still outstanding, so the moved untimestamped references
    // report afresh on the new route (#923).
    assert_eq!(baseline(&table, &a), None);
    assert_eq!(baseline(&table, &b), None);
}

#[test]
fn a_busy_context_resubscription_fences_the_report() {
    let mut table = CovSubscriptionTable::new();
    let (a, b) = context(&mut table);
    // Listing references while idle keeps the context's marker.
    refresh(
        &mut table,
        &a,
        &a.endpoint(),
        vec![reference(PropertyIdentifier::PRESENT_VALUE)],
    );
    assert!(live(&table, &b).flight.same(&b.flight));
    let report = ticket(&b);
    let flight = table.begin_confirmed(report, [&b]).unwrap();
    refresh(
        &mut table,
        &a,
        &a.endpoint(),
        vec![reference(PropertyIdentifier::PRESENT_VALUE)],
    );
    let relisted = live(&table, &a);
    let context = relisted.key().multiple_context().unwrap().clone();
    assert!(
        table.context_idle(&context, std::slice::from_ref(&relisted)),
        "the initial report need not wait for the old one"
    );
    assert!(!table.complete_observation(&b, report, value(2.0)));
    assert_eq!(
        table.revisits().queued(),
        HashSet::from([a.key().clone(), b.key().clone()]),
        "the kept reference, and the relisted one as a first report in case this \
         follow-up beats the initial report"
    );
    drop(flight);
}

#[test]
fn a_fence_while_owed_forgets_only_carried_kept_untimestamped_baselines() {
    let mut table = CovSubscriptionTable::new();
    let kept = table
        .admit_for_test(reference(PropertyIdentifier::PRESENT_VALUE), 0)
        .unwrap();
    let uncarried = table
        .admit_for_test(reference(PropertyIdentifier::RELIABILITY), 0)
        .unwrap();
    let mut proposal = reference(PropertyIdentifier::STATUS_FLAGS);
    proposal.timestamped = true;
    let stamped = table.admit_for_test(proposal, 0).unwrap();
    let relisted = table
        .admit_for_test(reference(PropertyIdentifier::OUT_OF_SERVICE), 0)
        .unwrap();
    let report = ticket(&kept);
    let flight = table
        .begin_confirmed(report, [&kept, &uncarried, &stamped, &relisted])
        .unwrap();
    for (sub, v) in [
        (&kept, 1.0),
        (&uncarried, 5.0),
        (&stamped, 2.0),
        (&relisted, 3.0),
    ] {
        assert!(table.complete_observation(sub, report, value(v)));
    }
    drop(flight);
    let baselines = |table: &CovSubscriptionTable| {
        [&kept, &uncarried, &stamped, &relisted].map(|sub| baseline(table, sub))
    };
    let acknowledged = [
        Some(value(1.0)),
        Some(value(5.0)),
        Some(value(2.0)),
        Some(value(3.0)),
    ];

    // A route change while the context is idle: the Ack settled every baseline.
    let home = kept.endpoint();
    let mut away = home.clone();
    away.mac = MacAddr::from_slice(&[2]);
    refresh(&mut table, &kept, &away, vec![]);
    assert_eq!(
        baselines(&table),
        acknowledged,
        "an idle fence clears nothing"
    );
    assert!(table.revisits().queued().is_empty());

    // Back home, listing one reference, while a report is outstanding. The
    // subscriber may hold what that report carried, ahead of the baselines.
    let moved = live(&table, &kept);
    let flight = table.begin_confirmed(ticket(&moved), [&moved]).unwrap();
    let mut relisting = reference(PropertyIdentifier::OUT_OF_SERVICE);
    relisting.last_notified_observation = Some(value(4.0));
    refresh(&mut table, &kept, &home, vec![relisting]);
    assert_eq!(
        baselines(&table),
        [None, Some(value(5.0)), Some(value(2.0)), Some(value(4.0))],
        "the carried kept untimestamped reference reports afresh; one the report \
         did not carry keeps its acknowledged baseline, the timestamped one \
         queues its changes, and the relisted one is what its request admitted"
    );
    assert_eq!(table.revisits().queued().len(), 4);
    drop(flight);
}

#[tokio::test(start_paused = true)]
async fn a_resubscription_during_a_hold_off_starts_unmarked() {
    let mut table = CovSubscriptionTable::new();
    let (a, b) = context(&mut table);
    assert!(table.complete_for_test(&b, value(2.0)));
    let acknowledged = live(&table, &b);
    table
        .begin_confirmed(ticket(&a), [&a, &acknowledged])
        .unwrap()
        .failed(Duration::from_secs(60));
    let context = a.key().multiple_context().unwrap().clone();
    // An empty renewal keeps the hold-off.
    refresh(&mut table, &a, &a.endpoint(), vec![]);
    assert!(!table.context_idle(&context, &[]));
    assert_eq!(baseline(&table, &b), Some(value(2.0)));
    refresh(
        &mut table,
        &a,
        &a.endpoint(),
        vec![reference(PropertyIdentifier::PRESENT_VALUE)],
    );
    assert!(table.context_idle(&context, &[live(&table, &a)]));
    assert_eq!(
        table.revisits().queued().len(),
        2,
        "the failed report's owed follow-up moves to the fresh marker"
    );
    assert_eq!(
        baseline(&table, &b),
        None,
        "the failed report may have been delivered, so the kept reference reports afresh"
    );
}

#[tokio::test]
async fn revisits_drain_once_and_forget_removed_references() {
    let mut table = CovSubscriptionTable::new();
    let a = table.subscribe(single()).unwrap();
    let mut other = single();
    other.subscriber_process_identifier = 2;
    let b = table.subscribe(other).unwrap();
    let revisits = Arc::clone(table.revisits());
    revisits.request([a.key().clone(), b.key().clone(), a.key().clone()]);
    assert!(table.unsubscribe(b.key()));
    assert_eq!(revisits.next().await, vec![a.key().clone()]);
    revisits.request([a.key().clone()]);
    assert_eq!(revisits.next().await, vec![a.key().clone()]);
}

#[tokio::test(start_paused = true)]
async fn a_passed_hold_off_is_owed_once_per_context() {
    let mut table = CovSubscriptionTable::new();
    let (a, _) = context(&mut table);
    let context = a.key().multiple_context().unwrap().clone();
    assert!(!table.take_owed_context(&context), "nothing failed");
    table
        .begin_confirmed(ticket(&a), [&a])
        .unwrap()
        .failed(Duration::from_millis(30));
    assert!(!table.take_owed_context(&context), "still holding off");
    tokio::time::advance(Duration::from_millis(30)).await;
    assert!(table.take_owed_context(&context));
    assert!(!table.take_owed_context(&context), "owed once");
    assert!(table.context_idle(&context, &[]));
}

#[tokio::test(start_paused = true)]
async fn a_replaced_marker_stops_its_flight() {
    let mut table = CovSubscriptionTable::new();
    let sub = table.subscribe(single()).unwrap();
    let flight = table.begin_confirmed(ticket(&sub), [&sub]).unwrap();
    assert!(futures_util::poll!(std::pin::pin!(flight.fenced())).is_pending());
    table.subscribe(single()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), flight.fenced())
        .await
        .expect("a renewal fences the old report");

    let (a, b) = context(&mut table);
    let flight = table.begin_confirmed(ticket(&a), [&a, &b]).unwrap();
    // An empty same-route renewal keeps the report going.
    refresh(&mut table, &a, &a.endpoint(), vec![]);
    assert!(futures_util::poll!(std::pin::pin!(flight.fenced())).is_pending());
    refresh(
        &mut table,
        &a,
        &a.endpoint(),
        vec![reference(PropertyIdentifier::PRESENT_VALUE)],
    );
    tokio::time::timeout(Duration::from_secs(1), flight.fenced())
        .await
        .expect("a busy re-subscription fences the old report");
}
