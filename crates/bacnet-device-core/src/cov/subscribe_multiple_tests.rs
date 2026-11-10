//! SubscribeCOVPropertyMultiple's checks, its cancellation, and how the
//! histories follow the table through it, on hand-set instants.
use bacnet_services::cov_multiple::COVReference;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use super::*;
use crate::cov::CovTimeRemaining;
use crate::cov::multiple_fixture::{
    HandClock, PV, T0, at, av, context, database, deliver, frame, key, reference, request,
    seconds_and_values, set, subscribe,
};

/// A property an Analog Value does not have.
const MISSING: PropertyIdentifier = PropertyIdentifier::FEEDBACK_VALUE;

fn fresh() -> (CovTable, TimedHistories) {
    (CovTable::default(), TimedHistories::new(1476))
}

fn class_and_code(error: &Error) -> (ErrorClass, ErrorCode) {
    match error {
        Error::Remote(RemoteError::Error { class, code })
        | Error::Structured { class, code, .. } => (*class, *code),
        other => panic!("not an Error PDU: {other:?}"),
    }
}

/// The reference a first-failed-subscription error names.
fn named(error: &Error) -> Option<(ObjectIdentifier, u32)> {
    match error {
        Error::Structured { detail, .. } => match &**detail {
            ErrorDetail::FirstFailedSubscription(reference) => {
                Some((reference.object_identifier, reference.property_identifier))
            }
            _ => None,
        },
        _ => None,
    }
}

/// Seconds of lifetime the table reports for `snapshot` at `now`.
fn remaining(table: &CovTable, snapshot: &CovSubscriptionSnapshot, now: Instant) -> Option<u32> {
    table
        .remaining_lifetime(snapshot, now)
        .and_then(CovTimeRemaining::wire_seconds)
}

#[test]
fn a_lifetime_and_a_delay_come_together_or_not_at_all() {
    let clock = HandClock::showing(1);
    let db = database(Some(&clock), 1);
    let (mut table, mut histories) = fresh();
    let one = [reference(PV, true)];
    for (lifetime, delay) in [(Some(300), None), (None, Some(30))] {
        let refusal = subscribe(
            &mut table,
            &mut histories,
            &db,
            request(1, lifetime, delay, &[(1, &one)]),
            T0,
        )
        .unwrap_err();
        assert_eq!(
            refusal.error.reject_reason(),
            Some(RejectReason::INCONSISTENT_PARAMETERS)
        );
        assert_eq!((refusal.refused, refusal.committed.len()), (None, 0));
    }
    assert!(table.is_empty());
    assert_eq!(histories.held(), (0, 0));
}

#[test]
fn timing_out_of_range_refuses_the_request_before_any_reference() {
    let clock = HandClock::showing(1);
    let db = database(Some(&clock), 1);
    let (mut table, mut histories) = fresh();
    let one = [reference(PV, true)];
    // No lifetime at all, a delay over an hour, a delay as long as the
    // lifetime, and one longer.
    for (lifetime, delay) in [(0, 0), (7200, 3601), (60, 60), (60, 61)] {
        let refusal = subscribe(
            &mut table,
            &mut histories,
            &db,
            request(1, Some(lifetime), Some(delay), &[(1, &one)]),
            T0,
        )
        .unwrap_err();
        assert_eq!(
            class_and_code(&refusal.error),
            (ErrorClass::SERVICES, ErrorCode::VALUE_OUT_OF_RANGE),
            "lifetime {lifetime}, delay {delay}"
        );
        // The general form: it names no reference and keeps nothing.
        assert_eq!(named(&refusal.error), None);
        assert_eq!((refusal.refused, refusal.committed.len()), (None, 0));
    }
    assert!(table.is_empty());
    // The longest delay and the shortest lifetime over it are accepted.
    subscribe(
        &mut table,
        &mut histories,
        &db,
        request(1, Some(3601), Some(3600), &[(1, &one)]),
        T0,
    )
    .unwrap();
    assert_eq!(table.len(), 1);
}

#[test]
fn an_object_that_is_missing_or_reports_nothing_refuses_its_first_reference() {
    let clock = HandClock::showing(1);
    let mut db = database(Some(&clock), 1);
    let (mut table, mut histories) = fresh();
    let refs = [reference(PV, false), reference(MISSING, false)];

    // Analog Value 2 is not there.
    let refusal = subscribe(
        &mut table,
        &mut histories,
        &db,
        request(1, Some(300), Some(30), &[(2, &refs)]),
        T0,
    )
    .unwrap_err();
    assert_eq!(
        class_and_code(&refusal.error),
        (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT)
    );
    assert_eq!(named(&refusal.error), Some((av(2), PV.to_raw())));
    assert_eq!((refusal.refused, refusal.committed.len()), (Some(0), 0));

    // A Device object is there, and has no COV reporting.
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap();
    db.add(Box::new(
        bacnet_objects::device::DeviceObject::new(bacnet_objects::device::DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(9, 0).unwrap(),
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    assert!(db.get(&device).is_some_and(|object| object.cov().is_none()));
    let mut request = request(1, Some(300), Some(30), &[(1, &refs[..1]), (1, &refs[..1])]);
    request.list_of_cov_subscription_specifications[1].monitored_object_identifier = device;
    let refusal = subscribe(&mut table, &mut histories, &db, request, T0).unwrap_err();
    assert_eq!(
        class_and_code(&refusal.error),
        (
            ErrorClass::OBJECT,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED
        )
    );
    assert_eq!(named(&refusal.error), Some((device, PV.to_raw())));
    // The reference before it was processed, and stays subscribed.
    assert_eq!((refusal.refused, refusal.committed.len()), (Some(1), 1));
    assert!(table.contains(&key(1, 1, PV)));
}

#[test]
fn a_reference_that_fails_is_named_and_the_ones_before_it_are_kept() {
    let clock = HandClock::showing(3);
    let mut db = database(Some(&clock), 3);
    set(&mut db, 1, 4.0);
    let (mut table, mut histories) = fresh();
    let good = [reference(PV, true)];
    let bad = [reference(PV, true), reference(MISSING, true)];
    let refusal = subscribe(
        &mut table,
        &mut histories,
        &db,
        request(1, Some(300), Some(30), &[(1, &good), (2, &bad), (3, &good)]),
        T0,
    )
    .unwrap_err();
    assert_eq!(
        class_and_code(&refusal.error),
        (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)
    );
    assert_eq!(named(&refusal.error), Some((av(2), MISSING.to_raw())));
    assert_eq!(refusal.refused, Some(2), "the third reference in order");
    let kept: Vec<_> = refusal
        .committed
        .iter()
        .map(|kept| kept.key().clone())
        .collect();
    // Nothing after the failed reference is processed.
    assert_eq!(kept, [key(1, 1, PV), key(1, 2, PV)]);
    assert_eq!(table.len(), 2);
    // The kept references are owed their initial reports like an accepted
    // request's: each has one, at the Device time of admission.
    assert_eq!(
        seconds_and_values(&deliver(&mut histories, &refusal.committed[0])),
        [(3, 4.0)]
    );
    assert_eq!(
        seconds_and_values(&deliver(&mut histories, &refusal.committed[1])),
        [(3, 0.0)]
    );

    // A request whose first reference fails changes nothing at all.
    let refusal = subscribe(
        &mut table,
        &mut histories,
        &db,
        request(2, Some(300), Some(30), &[(1, &bad[1..]), (1, &good)]),
        T0,
    )
    .unwrap_err();
    assert_eq!(
        class_and_code(&refusal.error),
        (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)
    );
    assert_eq!((refusal.refused, refusal.committed.len()), (Some(0), 0));
    assert!(
        table
            .multiple_context_references(&context(2))
            .next()
            .is_none()
    );
}

#[test]
fn a_timestamped_reference_needs_a_valid_device_clock() {
    let clock = HandClock::showing(1);
    let db = database(Some(&clock), 1);
    let refs: [COVReference; 2] = [
        reference(PropertyIdentifier::STATUS_FLAGS, false),
        reference(PV, true),
    ];
    let mut invalid = frame(1);
    invalid.local_date.day_of_week = 5;
    for showing in [None, Some(invalid)] {
        clock.set(showing);
        let (mut table, mut histories) = fresh();
        let refusal = subscribe(
            &mut table,
            &mut histories,
            &db,
            request(1, Some(300), Some(30), &[(1, &refs)]),
            T0,
        )
        .unwrap_err();
        assert_eq!(
            class_and_code(&refusal.error),
            (
                ErrorClass::SERVICES,
                ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED
            )
        );
        // The reference that asked for timestamps, after the one that did
        // not, which is kept.
        assert_eq!(named(&refusal.error), Some((av(1), PV.to_raw())));
        assert_eq!((refusal.refused, refusal.committed.len()), (Some(1), 1));
        assert_eq!(histories.next_deadline(), None, "no initial capture");
    }
    // Without timestamps the same device takes the request.
    clock.set(None);
    let (mut table, mut histories) = fresh();
    let untimed = [reference(PV, false)];
    let accepted = subscribe(
        &mut table,
        &mut histories,
        &db,
        request(1, Some(300), Some(30), &[(1, &untimed)]),
        T0,
    )
    .unwrap();
    assert_eq!(accepted.len(), 1);
}

#[test]
fn an_accepted_request_runs_its_lifetime_and_its_initial_reports_from_the_one_instant() {
    let clock = HandClock::showing(7);
    let mut db = database(Some(&clock), 2);
    set(&mut db, 1, 4.0);
    let (mut table, mut histories) = fresh();
    let accepted = subscribe(
        &mut table,
        &mut histories,
        &db,
        request(
            1,
            Some(300),
            Some(30),
            &[(1, &[reference(PV, true)]), (2, &[reference(PV, false)])],
        ),
        at(5),
    )
    .unwrap();
    assert_eq!(accepted.len(), 2);
    assert!(accepted[0].timestamped && !accepted[1].timestamped);

    // The lifetime runs from the request's instant.
    assert_eq!(remaining(&table, &accepted[0], at(5)), Some(300));
    assert_eq!(remaining(&table, &accepted[0], at(304)), Some(1));
    assert_eq!(remaining(&table, &accepted[0], at(305)), None);
    // So does the delay of the timestamped reference's initial report,
    // which carries the value and the Device time of admission.
    assert_eq!(
        histories.next_deadline(),
        Some(at(5) + Duration::from_secs(30))
    );
    assert_eq!(
        seconds_and_values(&deliver(&mut histories, &accepted[0])),
        [(7, 4.0)]
    );
    // An untimestamped reference has no history; it is only known, so that
    // a report can come to owe its values.
    assert!(deliver(&mut histories, &accepted[1]).is_empty());
    assert_eq!(histories.held(), (2, 0));
    // What the admission fenced is left for the host.
    assert_eq!(table.fenced().contexts, [context(1)]);
}

#[test]
fn a_renewal_keeps_what_is_pending_and_takes_the_new_delay() {
    let clock = HandClock::showing(1);
    let mut db = database(Some(&clock), 1);
    let (mut table, mut histories) = fresh();
    let timed = [reference(PV, true)];
    subscribe(
        &mut table,
        &mut histories,
        &db,
        request(1, Some(300), Some(30), &[(1, &timed)]),
        T0,
    )
    .unwrap();
    table.take_fenced();
    assert_eq!(histories.next_deadline(), Some(at(30)));

    // Renewed five seconds on with a shorter delay and a new value: the
    // initial report still pending counts the new delay from when it was
    // captured, and the renewal's own initial report follows it.
    set(&mut db, 1, 6.0);
    clock.set(Some(frame(6)));
    let renewed = subscribe(
        &mut table,
        &mut histories,
        &db,
        request(1, Some(600), Some(10), &[(1, &timed)]),
        at(5),
    )
    .unwrap();
    assert_eq!(table.len(), 1);
    assert_eq!(remaining(&table, &renewed[0], at(5)), Some(600));
    assert_eq!(histories.next_deadline(), Some(at(10)));
    assert_eq!(
        seconds_and_values(&deliver(&mut histories, &renewed[0])),
        [(1, 0.0), (6, 6.0)]
    );
}

#[test]
fn a_renewal_of_one_reference_gives_its_whole_context_the_new_delay() {
    let clock = HandClock::showing(1);
    let db = database(Some(&clock), 2);
    let (mut table, mut histories) = fresh();
    let (timed, untimed) = ([reference(PV, true)], [reference(PV, false)]);
    subscribe(
        &mut table,
        &mut histories,
        &db,
        request(1, Some(300), Some(30), &[(1, &timed), (2, &untimed)]),
        T0,
    )
    .unwrap();
    table.take_fenced();
    // Analog Value 1's initial report is pending, from the admission.
    assert_eq!(histories.next_deadline(), Some(at(30)));

    // The request names Analog Value 2 alone, which has no history. The
    // delay is the context's, so the pending report of Analog Value 1 is
    // now due ten seconds after it was captured.
    subscribe(
        &mut table,
        &mut histories,
        &db,
        request(1, Some(300), Some(10), &[(2, &untimed)]),
        at(5),
    )
    .unwrap();
    assert_eq!(table.len(), 2);
    assert_eq!(histories.next_deadline(), Some(at(10)));
}

#[test]
fn a_cancellation_removes_the_references_and_their_histories() {
    let clock = HandClock::showing(1);
    let db = database(Some(&clock), 2);
    let (mut table, mut histories) = fresh();
    let both = [
        reference(PV, true),
        reference(PropertyIdentifier::STATUS_FLAGS, false),
    ];
    let accepted = subscribe(
        &mut table,
        &mut histories,
        &db,
        request(1, Some(300), Some(30), &[(1, &both), (2, &both)]),
        T0,
    )
    .unwrap();
    table.take_fenced();
    assert_eq!((table.len(), histories.held()), (4, (4, 0)));
    assert_eq!(histories.next_deadline(), Some(at(30)));

    // One reference cancelled: its history goes, with its pending change.
    let cancelled = subscribe(
        &mut table,
        &mut histories,
        &db,
        request(1, None, None, &[(1, &both[..1])]),
        at(1),
    )
    .unwrap();
    assert!(cancelled.is_empty());
    assert_eq!(table.len(), 3);
    assert!(!table.contains(&key(1, 1, PV)));
    assert_eq!(histories.held().0, 3);
    assert!(deliver(&mut histories, &accepted[0]).is_empty());
    assert_eq!(table.fenced().removed, [key(1, 1, PV)], "left for the host");
    table.take_fenced();

    // Cancelling what is not subscribed succeeds and changes nothing.
    subscribe(
        &mut table,
        &mut histories,
        &db,
        request(1, None, None, &[(1, &both[..1])]),
        at(1),
    )
    .unwrap();
    assert_eq!((table.len(), histories.held().0), (3, 3));

    // No list at all cancels the whole context, pending changes and waits
    // included.
    subscribe(
        &mut table,
        &mut histories,
        &db,
        request(1, None, None, &[]),
        at(2),
    )
    .unwrap();
    assert!(table.is_empty());
    assert_eq!(histories.held(), (0, 0));
    assert_eq!(histories.next_deadline(), None);
    assert_eq!(table.fenced().removed.len(), 3);
}
