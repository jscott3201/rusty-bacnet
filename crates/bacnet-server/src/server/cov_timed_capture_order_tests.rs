//! A timestamped COV-multiple report that one notification cannot carry goes
//! out strictly in capture order, each reference's latest change included
//! (135-2020 §13.1, §13.18.1.1; #1008).
//!
//! Each notification fits the subscriber's maximum APDU even when the latest
//! changes alone would not, and carries a contiguous run of the report's
//! changes: every change in one notification is older than every change in
//! the next. The untimestamped values go in the last notification, or, when
//! they alone do not fit one, after every change in as many as they need
//! (#1038). A reference whose latest change went out in an earlier part
//! completes its
//! observation when that part is sent, or, when confirmed, acknowledged. A
//! change too large for any notification goes out one value per
//! notification (`cov_timed_value_split_tests`, #1090), and a value too large
//! even alone is dropped and counted, latest or not, so it never stalls the
//! context. Time is paused.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::primitives::Time;

const SMALL_APDU: u16 = 206;

/// Timestamped references in the many-object context: one PV and
/// Status_Flags change of each takes 33 octets, so the latest changes of all
/// of them exceed one 206-octet notification, which carries five.
const OBJECTS: u32 = 8;

/// One PV row: object, encoded value and Time_Of_Change.
type Row = (ObjectIdentifier, Vec<u8>, Option<Time>);

fn av(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap()
}

fn timed_objects() -> impl Iterator<Item = ObjectIdentifier> {
    (1..=OBJECTS).map(av)
}

/// Encoded APDU length of `notification`: its service request plus the
/// unconfirmed (2 octets) or unsegmented confirmed (4 octets) header.
fn apdu_len(notification: &COVNotificationMultipleRequest, confirmed: bool) -> usize {
    let mut encoded = BytesMut::new();
    notification.encode(&mut encoded).unwrap();
    encoded.len() + if confirmed { 4 } else { 2 }
}

/// Every PV row of `notification`, ordered by time; within one notification
/// items group by object, not by time.
fn pv_rows_by_time(notification: &COVNotificationMultipleRequest) -> Vec<Row> {
    let mut rows: Vec<Row> = notification
        .list_of_cov_notifications
        .iter()
        .flat_map(|item| {
            item.list_of_values
                .iter()
                .filter(|value| value.property_identifier == PV)
                .map(|value| {
                    (
                        item.monitored_object_identifier,
                        value.value.clone(),
                        value.time_of_change,
                    )
                })
        })
        .collect();
    rows.sort_by_key(|(_, _, time)| time.map(|time| time.second));
    rows
}

/// `notification` fits the small APDU, and its header names the newest
/// timestamped PV change it carries.
fn check(notification: &COVNotificationMultipleRequest, confirmed: bool) {
    let len = apdu_len(notification, confirmed);
    assert!(
        len <= usize::from(SMALL_APDU),
        "{len}-octet APDU exceeds {SMALL_APDU}"
    );
    let newest = pv_rows_by_time(notification)
        .into_iter()
        .filter_map(|(_, _, time)| time)
        .next_back();
    assert_eq!(
        notification.timestamp.map(|(_, time)| time),
        newest,
        "the header names the last change carried"
    );
}

/// A server with AV-1 to AV-8 and, when `untimed` is given, that many more
/// AVs after them, subscribed by one context of the subscriber's 206-octet
/// APDU: the first eight with timestamps, the rest without. The initial
/// report is left untaken.
async fn many_object_harness(confirmed: bool, untimed: u32, delay: u32) -> Harness {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        for instance in 2..=OBJECTS + untimed {
            let name = format!("AV-{instance}");
            db.add(Box::new(
                AnalogValueObject::new(instance, &name, EngineeringUnits::DEGREES_CELSIUS).unwrap(),
            ))
            .unwrap();
        }
    })
    .await;
    h.request_max_apdu = SMALL_APDU;
    let specs = (1..=OBJECTS + untimed)
        .map(|instance| (av(instance), vec![(PV, instance <= OBJECTS)]))
        .collect();
    h.subscribe_with_delay(confirmed, specs, delay).await;
    h
}

/// Notifications until `expected` PV rows have arrived, each checked and, when
/// confirmed, acknowledged before the next. Returns the notifications.
async fn take(
    h: &Harness,
    expected: usize,
    confirmed: bool,
) -> Vec<COVNotificationMultipleRequest> {
    take_as(h, expected, confirmed, true).await
}

/// The initial report, unchecked: a test examines its own report.
async fn take_initial(h: &Harness, expected: usize, confirmed: bool) {
    take_as(h, expected, confirmed, false).await;
}

async fn take_as(
    h: &Harness,
    expected: usize,
    confirmed: bool,
    checked: bool,
) -> Vec<COVNotificationMultipleRequest> {
    let mut taken = Vec::new();
    let mut rows = 0;
    while rows < expected {
        if confirmed && !taken.is_empty() {
            // One part outstanding at a time.
            h.no_notification().await;
            h.ack().await;
        }
        let notification = h.notification().await;
        if checked {
            check(&notification, confirmed);
        }
        rows += pv_rows_by_time(&notification).len();
        taken.push(notification);
    }
    if confirmed {
        h.ack().await;
        h.settle().await;
    }
    h.no_notification().await;
    taken
}

/// Queue one PV change of each object, in turn at seconds `first..`, behind
/// DISABLE_INITIATION; the last re-enables and reports them all. Returns
/// every change as its expected row, in capture order.
async fn hold_one_change_each(h: &Harness, objects: &[ObjectIdentifier], first: u8) -> Vec<Row> {
    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    let mut expected = Vec::new();
    for (index, (at_second, &object)) in (first..).zip(objects).enumerate() {
        if index + 1 == objects.len() {
            h.server.comm_state.set_for_test(DccState::Enable);
        }
        h.set_clock(at_second);
        let value = f32::from(at_second);
        h.write_local_to(object, value).await;
        expected.push((object, real(value), Some(time(at_second))));
    }
    expected
}

/// Each notification carries a contiguous run of the changes, in capture
/// order, and every change arrives once.
fn assert_capture_order(taken: &[COVNotificationMultipleRequest], expected: &[Row]) {
    let conveyed: Vec<Row> = taken.iter().flat_map(pv_rows_by_time).collect();
    assert_eq!(
        conveyed, expected,
        "every change once, strictly oldest first"
    );
}

/// The PV value each timestamped reference of `object` last completed.
async fn completed(h: &Harness, object: ObjectIdentifier) -> Option<PropertyValue> {
    let mut table = h.server.cov_table.write().await;
    table
        .subscriptions_for(&object)
        .into_iter()
        .find(|sub| sub.timestamped)
        .and_then(|sub| sub.last_notified_observation.as_ref())
        .map(|observation| observation.sample().value().clone())
}

#[tokio::test(start_paused = true)]
async fn an_initial_report_whose_latest_changes_exceed_the_apdu_goes_out_in_several() {
    let mut h = many_object_harness(false, 0, 10).await;
    let taken = take(&h, OBJECTS as usize, false).await;
    assert!(taken.len() >= 2, "{} notifications", taken.len());
    let mut objects: Vec<_> = taken
        .iter()
        .flat_map(pv_rows_by_time)
        .map(|(object, value, time_of_change)| {
            assert_eq!((value, time_of_change), (real(0.0), Some(time(0))));
            object
        })
        .collect();
    objects.sort_by_key(|object| object.instance_number());
    assert_eq!(objects, timed_objects().collect::<Vec<_>>(), "each once");
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn unconfirmed_latest_changes_beyond_one_apdu_go_out_in_capture_order() {
    let mut h = many_object_harness(false, 0, 10).await;
    take_initial(&h, OBJECTS as usize, false).await;
    let objects: Vec<_> = timed_objects().collect();
    let expected = hold_one_change_each(&h, &objects, 1).await;
    let taken = take(&h, expected.len(), false).await;
    assert_eq!(taken.len(), 2);
    assert_capture_order(&taken, &expected);
    // The last notification takes as many of the newest changes as fit.
    assert_eq!(pv_rows_by_time(&taken[1]).len(), 5);
    for (object, value, _) in &expected {
        assert_eq!(
            completed(&h, *object).await.map(|pv| real_of(&pv)),
            Some(value.clone())
        );
    }
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn confirmed_latest_changes_beyond_one_apdu_go_out_part_by_part_in_capture_order() {
    let mut h = many_object_harness(true, 0, 10).await;
    take_initial(&h, OBJECTS as usize, true).await;
    let objects: Vec<_> = timed_objects().collect();
    let expected = hold_one_change_each(&h, &objects, 1).await;
    let mut taken = Vec::new();
    let mut conveyed = 0;
    while conveyed < expected.len() {
        let notification = h.notification().await;
        check(&notification, true);
        let rows = pv_rows_by_time(&notification);
        // Nothing more until this part's Ack, which completes the references
        // it carried and only those (#1008).
        h.no_notification().await;
        for (object, _, _) in &rows {
            assert_eq!(completed(&h, *object).await, Some(PropertyValue::Real(0.0)));
        }
        h.ack().await;
        h.settle().await;
        for (object, value, _) in &rows {
            assert_eq!(
                completed(&h, *object).await.map(|pv| real_of(&pv)),
                Some(value.clone())
            );
        }
        conveyed += rows.len();
        taken.push(notification);
    }
    assert!(taken.len() >= 2, "{} notifications", taken.len());
    assert_capture_order(&taken, &expected);
    h.no_notification().await;
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
    h.server.stop().await.unwrap();
}

fn real_of(value: &PropertyValue) -> Vec<u8> {
    let PropertyValue::Real(value) = value else {
        panic!("a REAL Present_Value, got {value:?}");
    };
    real(*value)
}

#[tokio::test(start_paused = true)]
async fn a_latest_change_older_than_another_references_history_goes_out_first() {
    let mut h = many_object_harness(false, 0, 10).await;
    take_initial(&h, OBJECTS as usize, false).await;
    // AV-2 changes once, then AV-1 nine times: AV-2's only change, its latest,
    // is the oldest of the report and leads it.
    let mut objects = vec![av(2)];
    objects.extend(std::iter::repeat_n(av(1), 9));
    let expected = hold_one_change_each(&h, &objects, 1).await;
    let taken = take(&h, expected.len(), false).await;
    assert!(taken.len() >= 2, "{} notifications", taken.len());
    assert_capture_order(&taken, &expected);
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn untimestamped_values_go_last_with_the_newest_changes_that_still_fit() {
    let mut h = many_object_harness(false, 1, 10).await;
    let untimed = av(OBJECTS + 1);
    take_initial(&h, OBJECTS as usize + 1, false).await;
    // Every timestamped object changes behind DISABLE_INITIATION, then the
    // untimestamped one, which reports them all at once (§13.1).
    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    let mut expected = Vec::new();
    for (at_second, object) in (1..).zip(timed_objects()) {
        h.set_clock(at_second);
        h.write_local_to(object, f32::from(at_second)).await;
        expected.push((object, real(f32::from(at_second)), Some(time(at_second))));
    }
    h.server.comm_state.set_for_test(DccState::Enable);
    h.set_clock(20);
    h.write_local_to(untimed, 7.0).await;
    expected.push((untimed, real(7.0), None));
    let taken = take(&h, expected.len(), false).await;
    let carries_untimed = |notification: &COVNotificationMultipleRequest| {
        notification
            .list_of_cov_notifications
            .iter()
            .any(|item| item.monitored_object_identifier == untimed)
    };
    let (last, earlier) = taken.split_last().unwrap();
    assert!(
        !earlier.is_empty(),
        "the timestamped changes need a part of their own"
    );
    assert!(earlier.iter().all(|n| !carries_untimed(n)));
    assert!(carries_untimed(last));
    assert!(
        pv_rows_by_time(last).len() > 1,
        "the newest changes that fit share the last notification"
    );
    // Untimed rows sort first in a notification; compare the timed ones.
    let conveyed: Vec<Row> = taken
        .iter()
        .flat_map(pv_rows_by_time)
        .filter(|(_, _, time)| time.is_some())
        .collect();
    assert_eq!(conveyed, expected[..expected.len() - 1]);
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn untimestamped_values_too_large_for_one_apdu_follow_every_change_in_parts_that_fit() {
    // Nine untimestamped references take more than one notification on their
    // own. The initial report carries them all with the timestamped ones:
    // the timestamped changes go first, in parts that fit, and the
    // untimestamped values follow, split by object item into parts that fit
    // too (#1038).
    let untimed = 9;
    let mut h = many_object_harness(false, untimed, 10).await;
    let mut taken = Vec::new();
    let mut untimed_rows = 0;
    while untimed_rows < untimed as usize {
        let notification = h.notification().await;
        check(&notification, false);
        untimed_rows += pv_rows_by_time(&notification)
            .iter()
            .filter(|(object, _, _)| object.instance_number() > OBJECTS)
            .count();
        taken.push(notification);
    }
    h.no_notification().await;
    let carries_untimed = |notification: &COVNotificationMultipleRequest| {
        notification
            .list_of_cov_notifications
            .iter()
            .any(|item| item.monitored_object_identifier.instance_number() > OBJECTS)
    };
    let first_untimed = taken.iter().position(carries_untimed).unwrap();
    let (timed, untimed_parts) = taken.split_at(first_untimed);
    let timed_rows: usize = timed.iter().map(|n| pv_rows_by_time(n).len()).sum();
    assert_eq!(
        timed_rows, OBJECTS as usize,
        "every timestamped change first"
    );
    assert!(
        untimed_parts.len() >= 2,
        "{} untimestamped parts",
        untimed_parts.len()
    );
    let mut objects = Vec::new();
    for notification in untimed_parts {
        let rows = pv_rows_by_time(notification);
        assert!(
            rows.iter()
                .all(|(object, _, time)| { object.instance_number() > OBJECTS && time.is_none() })
        );
        assert_eq!(notification.timestamp, None, "nothing in it is timestamped");
        objects.extend(rows.into_iter().map(|(object, _, _)| object));
    }
    objects.sort_by_key(|object| object.instance_number());
    let expected: Vec<_> = (OBJECTS + 1..=OBJECTS + untimed).map(av).collect();
    assert_eq!(objects, expected, "each untimestamped value once");
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_reference_completes_when_the_part_carrying_its_latest_change_is_sent() {
    let mut h = many_object_harness(false, 0, 1).await;
    take_initial(&h, OBJECTS as usize, false).await;
    // The second part fails; the first was sent, and its references are done.
    h.fail_notification(1);
    let objects: Vec<_> = timed_objects().collect();
    let expected = hold_one_change_each(&h, &objects, 1).await;
    let first = h.notification().await;
    check(&first, false);
    h.no_notification().await;
    let sent = pv_rows_by_time(&first);
    assert!(sent.len() < expected.len(), "a first part");
    for (object, value, _) in &expected {
        let done = sent.iter().any(|(sent, _, _)| sent == object);
        let want = if done { value.clone() } else { real(0.0) };
        assert_eq!(
            completed(&h, *object).await.map(|pv| real_of(&pv)),
            Some(want)
        );
    }
    // The failed part goes out again at the Max_Notification_Delay deadline.
    let mut taken = vec![first];
    taken.extend(take(&h, expected.len() - sent.len(), false).await);
    assert_capture_order(&taken, &expected);
    for (object, value, _) in &expected {
        assert_eq!(
            completed(&h, *object).await.map(|pv| real_of(&pv)),
            Some(value.clone())
        );
    }
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
    h.server.stop().await.unwrap();
}

/// A harness whose one context subscribes CSV-1's Present_Value with
/// timestamps from a 206-octet subscriber, with the initial report taken
/// (and acknowledged, when confirmed), and a writer of CSV-1's value.
async fn string_harness(confirmed: bool) -> (Harness, ObjectIdentifier) {
    use bacnet_objects::value_types::CharacterStringValueObject;
    let csv = ObjectIdentifier::new(ObjectType::CHARACTERSTRING_VALUE, 1).unwrap();
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(
            CharacterStringValueObject::new(1, "CSV-1").unwrap(),
        ))
        .unwrap();
    })
    .await;
    h.request_max_apdu = SMALL_APDU;
    h.subscribe_specs(confirmed, vec![(csv, vec![(PV, true)])])
        .await;
    h.notification().await;
    if confirmed {
        h.ack().await;
        h.settle().await;
    }
    (h, csv)
}

async fn write_string(h: &Harness, csv: ObjectIdentifier, text: String) {
    h.server
        .write_local(
            &csv,
            PV,
            None,
            PropertyValue::CharacterString(text),
            Some(8),
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_confirmed_latest_change_too_large_for_any_notification_never_stalls_the_context() {
    let (mut h, csv) = string_harness(true).await;
    // A 200-character value alone exceeds a 206-octet notification: no
    // report could ever carry it, so it is dropped and counted rather than
    // sent over the subscriber's limit and retried without end. The rest of
    // its change, the Status_Flags, goes out on its own (#1090).
    h.set_clock(1);
    write_string(&h, csv, "a".repeat(200)).await;
    let flags = h.notification().await;
    let carried: Vec<_> = flags.list_of_cov_notifications[0]
        .list_of_values
        .iter()
        .map(|value| (value.property_identifier, value.time_of_change))
        .collect();
    assert_eq!(carried, vec![(SF, Some(time(1)))]);
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 1);
    h.ack().await;
    h.settle().await;
    h.no_notification().await;
    // The context is idle, so the next change reports at once.
    h.set_clock(2);
    write_string(&h, csv, "short".into()).await;
    let report = h.notification().await;
    assert!(apdu_len(&report, true) <= usize::from(SMALL_APDU));
    let rows: Vec<_> = report.list_of_cov_notifications[0]
        .list_of_values
        .iter()
        .filter(|value| value.property_identifier == PV)
        .map(|value| value.time_of_change)
        .collect();
    assert_eq!(rows, vec![Some(time(2))]);
    h.ack().await;
    h.settle().await;
    h.no_notification().await;
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 1);
    h.server.stop().await.unwrap();
}
