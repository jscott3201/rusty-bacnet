//! The table's earliest-due instant (#2303): the expiry pass skips work
//! only while nothing can be due, never past a lifetime or a deadline.
use super::*;
use crate::cov::{CovNotificationKind, Delivery, FlightTiming};
use alloc::vec;
use bacnet_time::Duration;
use bacnet_types::MacAddr;
use bacnet_types::enums::ObjectType;

const T0: Instant = Instant::from_secs(1_000);
/// One-second attempts, no retry: deadlines two seconds after a begin.
const TIMING: FlightTiming = FlightTiming::new(Duration::from_secs(1), 0);

fn secs(n: u64) -> Instant {
    T0 + Duration::from_secs(n)
}

fn proposal(process: u32, expires: Option<u64>) -> CovSubscription {
    CovSubscription {
        subscriber_mac: MacAddr::from_slice(&[1]),
        subscriber_network: None,
        subscriber_process_identifier: process,
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
        issue_confirmed_notifications: true,
        expires_at: expires.map(secs),
        last_notified_observation: None,
        monitored_property: None,
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: CovNotificationKind::Single,
        timestamped: false,
    }
}

/// The cached instant is never later than any lifetime or outstanding
/// deadline in the table.
fn assert_not_late(table: &CovTable) {
    let due = table.next_expiry();
    let lifetimes = table.entries().filter_map(|entry| entry.expires_at);
    let deadlines = table
        .flights
        .values()
        .filter_map(FlightState::outstanding_deadline);
    for at in lifetimes.chain(deadlines) {
        assert!(due.is_some_and(|due| due <= at), "{due:?} after {at:?}");
    }
}

fn begin(table: &mut CovTable, sub: &CovSubscriptionSnapshot, now: Instant) -> Token {
    let live = table.get_subscription(sub.key()).unwrap().clone();
    let report = table.prepare_completion(&live).unwrap();
    let observation = crate::cov::CovObservation::new(
        crate::cov::CovSample::new(&bacnet_types::primitives::PropertyValue::Real(0.0)).unwrap(),
        None,
    )
    .unwrap();
    table
        .begin_confirmed(report, [(&live, observation)], now)
        .unwrap()
}

#[test]
fn an_empty_or_indefinite_table_is_never_due() {
    let mut table = CovTable::default().with_flight_timing(TIMING);
    assert_eq!(table.next_expiry(), None);
    table.subscribe(proposal(1, None), T0).unwrap();
    assert_eq!(table.next_expiry(), None);
    assert_eq!(table.expire(secs(1_000_000)), 0);
    assert_eq!(table.len(), 1);
}

/// A lifetime shortened by a renewal, a removal of the earliest one, and a
/// report begun on an indefinite subscription: each due instant is met
/// exactly, and the pass before it finds nothing.
#[test]
fn renewals_removals_and_reports_are_never_skipped() {
    let mut table = CovTable::default().with_flight_timing(TIMING);
    let long = table.subscribe(proposal(1, Some(60)), T0).unwrap();
    let short = table.subscribe(proposal(2, Some(10)), T0).unwrap();
    let forever = table.subscribe(proposal(3, None), T0).unwrap();
    assert_not_late(&table);
    // Removing the earliest leaves the cache early: a pass then, and none
    // is removed, but it recomputes.
    assert!(table.unsubscribe(short.key()));
    assert_eq!(table.expire(secs(10)), 0);
    assert_eq!(table.next_expiry(), Some(secs(60)));
    // A renewal that shortens a lifetime lowers it.
    table.subscribe(proposal(1, Some(15)), secs(10)).unwrap();
    assert_not_late(&table);
    assert_eq!(table.expire(secs(15) - Duration::from_nanos(1)), 0);
    assert_eq!(table.expire(secs(15)), 1, "expired exactly on time");
    assert!(!table.contains(long.key()));
    // A report begun on the indefinite subscription is due at its deadline.
    let token = begin(&mut table, &forever, secs(20));
    assert_not_late(&table);
    table.take_fenced();
    table.expire(secs(22) - Duration::from_nanos(1));
    assert!(table.take_fenced().tokens.is_empty());
    table.expire(secs(22));
    assert_eq!(
        table.take_fenced().tokens,
        vec![token],
        "ended at its deadline"
    );
    assert_eq!(table.next_expiry(), None);
    // A settled report leaves the cache early, which only costs a pass.
    let token = begin(&mut table, &forever, secs(30));
    table.delivered(token, Delivery::Acknowledged, secs(30));
    table.expire(secs(32));
    assert!(table.take_fenced().is_empty());
    assert_eq!(table.next_expiry(), None);
}

#[test]
fn a_context_renewal_and_expire_all_lower_it() {
    let mut table = CovTable::default();
    let mut reference = proposal(1, Some(100));
    reference.notification_kind = CovNotificationKind::Multiple;
    reference.monitored_property = Some(bacnet_types::enums::PropertyIdentifier::PRESENT_VALUE);
    let context = reference.key().unwrap().multiple_context().unwrap().clone();
    let route = reference.endpoint();
    table
        .subscribe_multiple(&context, &route, secs(100), 0, None, vec![reference], T0)
        .unwrap();
    // An empty renewal that shortens the context's lifetime.
    table
        .subscribe_multiple(&context, &route, secs(3), 0, None, vec![], T0)
        .unwrap();
    assert_not_late(&table);
    assert_eq!(table.expire(secs(3)), 1);
    table.subscribe(proposal(2, Some(50)), T0).unwrap();
    table.expire_all(secs(4));
    assert_not_late(&table);
    assert_eq!(table.expire(secs(4)), 1);
    assert_eq!(table.next_expiry(), None);
}

/// A full pass that removes an expired subscription keeps the deadline of a
/// report still outstanding on another, so the next pass ends that report.
#[test]
fn a_full_pass_keeps_an_outstanding_deadline() {
    let mut table = CovTable::default().with_flight_timing(TIMING);
    table.subscribe(proposal(1, Some(1)), T0).unwrap();
    let forever = table.subscribe(proposal(2, None), T0).unwrap();
    let token = begin(&mut table, &forever, T0);
    table.take_fenced();
    assert_eq!(table.expire(secs(1)), 1);
    assert_not_late(&table);
    table.take_fenced();
    table.expire(secs(2));
    assert_eq!(
        table.take_fenced().tokens,
        vec![token],
        "ended at its deadline"
    );
}
