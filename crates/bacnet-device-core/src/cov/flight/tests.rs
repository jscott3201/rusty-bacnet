//! The plain flight state: deadlines, outcomes and fences (#896, #2303).
use super::*;
use crate::cov::{CovNotificationKind, CovObservation, CovSample};
use crate::cov::{CovSubscription, Fenced};
use alloc::vec;
use bacnet_types::MacAddr;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

const T0: Instant = Instant::from_secs(1_000);
/// One-second attempts and one retry: two seconds of retries, a deadline
/// three seconds after the report begins, and a two-second hold-off.
const TIMING: FlightTiming = FlightTiming::new(Duration::from_secs(1), 1);

fn proposal(kind: CovNotificationKind, property: PropertyIdentifier) -> CovSubscription {
    CovSubscription {
        subscriber_mac: MacAddr::from_slice(&[1]),
        subscriber_network: None,
        subscriber_process_identifier: 1,
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
        issue_confirmed_notifications: true,
        expires_at: Some(T0 + Duration::from_secs(600)),
        last_notified_observation: None,
        monitored_property: Some(property),
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: kind,
        timestamped: false,
    }
}

fn table() -> CovTable {
    CovTable::default().with_flight_timing(TIMING)
}

fn single(table: &mut CovTable) -> CovSubscriptionSnapshot {
    table
        .subscribe(
            proposal(
                CovNotificationKind::Single,
                PropertyIdentifier::PRESENT_VALUE,
            ),
            T0,
        )
        .unwrap()
}

/// A two-reference confirmed context.
fn context(table: &mut CovTable, route: u8) -> (CovSubscriptionSnapshot, CovSubscriptionSnapshot) {
    let mut refs = [
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::STATUS_FLAGS,
    ]
    .map(|property| proposal(CovNotificationKind::Multiple, property));
    for sub in &mut refs {
        sub.subscriber_mac = MacAddr::from_slice(&[route]);
        sub.subscriber_network = Some(bacnet_encoding::npdu::NpduAddress {
            network: 7,
            mac_address: MacAddr::from_slice(&[9]),
        });
    }
    let context = refs[0].key().unwrap().multiple_context().unwrap().clone();
    let route = refs[0].endpoint();
    let expires = refs[0].expires_at.unwrap();
    let mut accepted = table
        .subscribe_multiple(&context, &route, expires, 5, None, refs.to_vec(), T0)
        .unwrap();
    let b = accepted.pop().unwrap();
    (accepted.pop().unwrap(), b)
}

fn begin(
    table: &mut CovTable,
    carried: &[&CovSubscriptionSnapshot],
    now: Instant,
) -> (Token, PreparedCovCompletion) {
    let report = table.prepare_completion(carried[0]).unwrap();
    let token = table
        .begin_confirmed(report, carried.iter().map(|sub| (*sub, value(0.0))), now)
        .unwrap();
    (token, report)
}

fn value(v: f32) -> CovObservation {
    CovObservation::new(CovSample::new(&PropertyValue::Real(v)).unwrap(), None).unwrap()
}

#[test]
fn timing_sets_the_deadline_and_the_hold_off() {
    assert_eq!(TIMING.deadline(), Duration::from_secs(3));
    assert_eq!(TIMING.hold_off(), Duration::from_secs(2));
    let defaults = FlightTiming::default();
    assert_eq!(defaults.deadline(), Duration::from_secs(15));
    assert_eq!(defaults.hold_off(), Duration::from_secs(12));
}

/// A worker that never reports, aborted say: its report holds the
/// coordinate until the deadline, and ends itself there with no hold-off.
#[test]
fn an_unsettled_report_ends_itself_at_its_deadline() {
    let mut table = table();
    let sub = single(&mut table);
    let (token, report) = begin(&mut table, &[&sub], T0);
    let deadline = T0 + Duration::from_secs(3);
    let just_before = deadline - Duration::from_nanos(1);
    assert!(!table.confirmed_idle(&sub, just_before));
    assert!(
        table.confirmed_idle(&sub, deadline),
        "freed at the deadline"
    );
    // The abandoned report's late outcome counts for nothing.
    assert!(!table.complete_observation(&sub, report, value(1.0), deadline));
    table.delivered(token, Delivery::Failed, deadline);
    assert!(table.confirmed_idle(&sub, deadline), "no hold-off");
    // The next report begins at once.
    let (next, report) = begin(&mut table, &[&sub], deadline);
    assert!(table.complete_observation(&sub, report, value(2.0), deadline));
    table.delivered(next, Delivery::Acknowledged, deadline);
    assert!(table.confirmed_idle(
        &table.get_subscription(sub.key()).unwrap().clone(),
        deadline
    ));
}

/// A host reads a report's deadline off the table, to end what the report's
/// worker holds at the same instant.
#[test]
fn a_report_names_its_deadline_until_it_is_settled() {
    let mut table = table();
    let sub = single(&mut table);
    let (token, _) = begin(&mut table, &[&sub], T0);
    assert_eq!(table.report_deadline(token), Some(T0 + TIMING.deadline()));
    table.delivered(token, Delivery::Failed, T0);
    assert_eq!(table.report_deadline(token), None, "settled");
    // The next report on the coordinate has its own, from when it began.
    let later = T0 + TIMING.hold_off();
    let (next, _) = begin(&mut table, &[&sub], later);
    assert_eq!(table.report_deadline(next), Some(later + TIMING.deadline()));
    assert_eq!(table.report_deadline(token), None, "not the old report's");
}

/// The table says whether a report's failure counted, and until when its
/// coordinate then holds off (#2419).
#[cfg(feature = "cov-multiple")]
#[test]
fn a_failure_names_its_hold_off_only_when_the_table_counted_it() {
    let mut table = table();
    let (a, _) = context(&mut table, 1);
    let failed = T0 + Duration::from_secs(1);
    let until = failed + TIMING.hold_off();

    // Counted: the hold-off runs from the failure, and is still named once
    // it has passed, until the context's follow-up is taken.
    let (token, _) = begin(&mut table, &[&a], T0);
    assert_eq!(table.failed_hold_until(token), None, "outstanding");
    table.delivered(token, Delivery::Failed, failed);
    assert_eq!(table.failed_hold_until(token), Some(until));
    let context_key = a.key().multiple_context().unwrap().clone();
    assert!(table.take_owed_context(&context_key, until));
    assert_eq!(table.failed_hold_until(token), None, "taken");

    // Acknowledged, or never sent: no hold-off.
    for outcome in [Delivery::Acknowledged, Delivery::NotSent] {
        let a = table.get_subscription(a.key()).unwrap().clone();
        let (token, _) = begin(&mut table, &[&a], until);
        table.delivered(token, outcome, until);
        assert_eq!(table.failed_hold_until(token), None, "{outcome:?}");
    }

    // Past its deadline the report's failure changes nothing.
    let a = table.get_subscription(a.key()).unwrap().clone();
    let (late, _) = begin(&mut table, &[&a], until);
    let deadline = until + TIMING.deadline();
    table.delivered(late, Delivery::Failed, deadline);
    assert_eq!(table.failed_hold_until(late), None, "abandoned");

    // Fenced by a route change, a report's failure changes nothing either,
    // nor is it taken for the failure of the report that replaced it.
    let (fenced, _) = begin(&mut table, &[&a], deadline);
    let (moved, _) = context(&mut table, 2);
    table.delivered(fenced, Delivery::Failed, deadline);
    assert_eq!(table.failed_hold_until(fenced), None, "fenced");
    let (next, _) = begin(&mut table, &[&moved], deadline);
    table.delivered(next, Delivery::Failed, deadline);
    assert_eq!(table.failed_hold_until(fenced), None);
    let held = Some(deadline + TIMING.hold_off());
    assert_eq!(table.failed_hold_until(next), held);

    // Cancelled with its context, it has no coordinate left.
    table.unsubscribe_cov_multiple_context(&context_key);
    assert_eq!(table.failed_hold_until(next), None, "cancelled");

    // An ordinary subscription's hold-off is taken by nothing, so its end
    // stays on the flight. It is not named while a later report is marked,
    // out or past its deadline; that report's own late failure counts for
    // nothing.
    let sub = single(&mut table);
    let (first, _) = begin(&mut table, &[&sub], T0);
    table.delivered(first, Delivery::Failed, T0);
    let over = T0 + TIMING.hold_off();
    assert_eq!(table.failed_hold_until(first), Some(over));
    let (second, _) = begin(&mut table, &[&sub], over);
    assert_eq!(table.failed_hold_until(first), None, "reporting again");
    let abandoned = over + TIMING.deadline();
    table.delivered(second, Delivery::Failed, abandoned);
    assert_eq!(table.failed_hold_until(second), None, "past its deadline");
}

/// A hold-off in force goes with a replaced flight, and the table names
/// its context, so a host stops waiting for it (#2419). A flight replaced
/// with its report still out held nothing off.
#[test]
fn a_replaced_flight_names_the_context_whose_hold_off_went_with_it() {
    let mut table = table();
    let (a, _) = context(&mut table, 1);
    let key = a.key().multiple_context().unwrap().clone();
    let (out, _) = begin(&mut table, &[&a], T0);
    table.take_fenced();
    let (moved, _) = context(&mut table, 2);
    let fenced = table.take_fenced();
    assert_eq!(fenced.tokens, [out]);
    assert!(fenced.released.is_empty(), "no hold-off to lose");

    let (failed, _) = begin(&mut table, &[&moved], T0);
    table.delivered(failed, Delivery::Failed, T0);
    assert!(table.context_hold_until(&key, T0).is_some());
    context(&mut table, 1);
    let fenced = table.take_fenced();
    assert!(fenced.tokens.is_empty(), "no report was out");
    assert_eq!(fenced.released, core::slice::from_ref(&key));
    assert_eq!(table.context_hold_until(&key, T0), None);
}

/// The table's expiry pass reports an abandoned report: its token, so the
/// host drops the worker's wake-up, and its references, owed a fresh look.
#[test]
fn expire_reports_abandoned_reports_once() {
    let mut table = table();
    let sub = single(&mut table);
    let (a, b) = context(&mut table, 1);
    let (single_token, _) = begin(&mut table, &[&sub], T0);
    let (context_token, _) = begin(&mut table, &[&a], T0);
    table.take_fenced();
    table.expire(T0 + Duration::from_secs(2));
    assert_eq!(table.take_fenced(), Fenced::default(), "not yet due");
    table.expire(T0 + Duration::from_secs(3));
    let fenced = table.take_fenced();
    assert_eq!(fenced.tokens.len(), 2);
    assert!(fenced.tokens.contains(&single_token) && fenced.tokens.contains(&context_token));
    let mut revisit = fenced.revisit.clone();
    revisit.sort();
    let mut expected = vec![sub.key().clone(), a.key().clone(), b.key().clone()];
    expected.sort();
    assert_eq!(revisit, expected, "the whole context is owed a look");
    table.expire(T0 + Duration::from_secs(4));
    assert_eq!(table.take_fenced(), Fenced::default(), "reported once");
}

#[test]
fn outcomes_settle_the_report() {
    let mut table = table();
    let sub = single(&mut table);
    for outcome in [Delivery::Acknowledged, Delivery::NotSent] {
        let live = table.get_subscription(sub.key()).unwrap().clone();
        let (token, _) = begin(&mut table, &[&live], T0);
        assert!(!table.confirmed_idle(&live, T0));
        table.delivered(token, outcome, T0);
        // The Ack committed what the report carried, so a snapshot from
        // before it is stale.
        assert_eq!(
            table.confirmed_idle(&live, T0),
            outcome == Delivery::NotSent
        );
        let after = table.get_subscription(sub.key()).unwrap().clone();
        assert!(
            table.confirmed_idle(&after, T0),
            "{outcome:?} holds nothing off"
        );
    }
    let sub = table.get_subscription(sub.key()).unwrap().clone();
    let (token, _) = begin(&mut table, &[&sub], T0);
    table.delivered(token, Delivery::Failed, T0 + Duration::from_secs(1));
    let held_until = T0 + Duration::from_secs(3);
    assert!(!table.confirmed_idle(&sub, held_until - Duration::from_nanos(1)));
    assert!(
        table.confirmed_idle(&sub, held_until),
        "one retry cycle after the failure"
    );
    // A second outcome for the same report changes nothing.
    table.delivered(token, Delivery::Acknowledged, T0 + Duration::from_secs(1));
    assert!(!table.confirmed_idle(&sub, T0 + Duration::from_secs(2)));
}

/// A renewal or a route change replaces a flight and fences its outstanding
/// report; a removal ends the flight without a fence, as dropping its marker
/// did.
#[test]
fn fences_name_replaced_reports_only() {
    let mut table = table();
    let sub = single(&mut table);
    let (token, _) = begin(&mut table, &[&sub], T0);
    table.take_fenced();
    let renewed = single(&mut table);
    assert_eq!(table.take_fenced().tokens, vec![token]);
    assert!(table.confirmed_idle(&renewed, T0));

    let (a, b) = context(&mut table, 1);
    let (token, _) = begin(&mut table, &[&a, &b], T0);
    table.take_fenced();
    // A same-route empty renewal keeps the flight.
    let ctx = a.key().multiple_context().unwrap().clone();
    table
        .subscribe_multiple(
            &ctx,
            &a.endpoint(),
            T0 + Duration::from_secs(900),
            5,
            None,
            vec![],
            T0,
        )
        .unwrap();
    let fenced = table.take_fenced();
    assert!(fenced.tokens.is_empty() && fenced.revisit.is_empty());
    assert_eq!(fenced.contexts, vec![ctx.clone()]);
    // A route change fences the report and owes its references a look.
    let (moved, _) = context(&mut table, 2);
    let fenced = table.take_fenced();
    assert_eq!(fenced.tokens, vec![token]);
    assert_eq!(fenced.revisit.len(), 2);
    assert!(table.confirmed_idle(&moved, T0));

    let (token, _) = begin(&mut table, &[&renewed], T0);
    assert!(table.unsubscribe(renewed.key()));
    let fenced = table.take_fenced();
    assert!(fenced.tokens.is_empty(), "a removal fences nothing");
    assert_eq!(fenced.removed, vec![renewed.key().clone()]);
    table.delivered(token, Delivery::Acknowledged, T0);
    assert_eq!(
        table.remaining_lifetime(&renewed, T0),
        None,
        "the removed subscription stays gone"
    );
}

/// A report begun on a coordinate whose last report passed its deadline
/// unsettled fences that report: its worker, if alive, stops (#2303).
#[test]
fn a_report_past_its_deadline_is_fenced_by_the_next() {
    let mut table = table();
    let sub = single(&mut table);
    let (stale, _) = begin(&mut table, &[&sub], T0);
    table.take_fenced();
    let deadline = T0 + Duration::from_secs(3);
    let (next, report) = begin(&mut table, &[&sub], deadline);
    assert_eq!(table.take_fenced().tokens, vec![stale]);
    assert_ne!(stale, next);
    // The stale outcome changes nothing; the new report's counts.
    table.delivered(stale, Delivery::Acknowledged, deadline);
    assert!(!table.confirmed_idle(&sub, deadline));
    assert!(table.complete_observation(&sub, report, value(3.0), deadline));
    table.delivered(next, Delivery::Acknowledged, deadline);
    let live = table.get_subscription(sub.key()).unwrap().clone();
    assert!(table.confirmed_idle(&live, deadline));
}
