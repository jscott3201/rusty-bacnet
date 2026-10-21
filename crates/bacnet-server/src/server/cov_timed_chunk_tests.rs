//! Timestamped COV-multiple history that one notification cannot carry goes
//! out in several (135-2020 §13.1, §13.18.1.1; #986).
//!
//! Each notification fits the smaller of the local maximum APDU and the one
//! the subscriber advertised in its SubscribeCOVPropertyMultiple request.
//! Older changes go first, and the last notification carries the newest with
//! the untimestamped values; `cov_timed_capture_order_tests` covers latest
//! changes that do not fit it (#1008), `cov_timed_value_split_tests` changes
//! too large for a notification of their own (#1090), and
//! `cov_untimed_split_tests` untimestamped values that do not fit (#1038).
//! Every envelope names the last change its notification carries. Time is
//! paused.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::primitives::Time;
use tokio::time::Instant as TokioInstant;

const SMALL_APDU: u16 = 206;

/// Changes held before the one that releases them. With it, ten PV and
/// Status_Flags changes: fewer than one 206-octet context holds, about
/// twenty by its room for items, and more than one 206-octet notification
/// carries.
const HELD: u8 = 9;

/// One PV row: encoded value and Time_Of_Change.
type Row = (Vec<u8>, Option<Time>);

/// Encoded APDU length of `notification`: its service request plus the
/// unconfirmed (2 octets) or unsegmented confirmed (4 octets) header.
fn apdu_len(notification: &COVNotificationMultipleRequest, confirmed: bool) -> usize {
    let mut encoded = BytesMut::new();
    notification.encode(&mut encoded).unwrap();
    encoded.len() + if confirmed { 4 } else { 2 }
}

/// `notification` fits `limit`, and its envelope names the last PV change it
/// carries (PV changes here are one second apart, in capture order).
fn check(notification: &COVNotificationMultipleRequest, limit: u16, confirmed: bool) {
    let len = apdu_len(notification, confirmed);
    assert!(
        len <= usize::from(limit),
        "{len}-octet APDU exceeds {limit}"
    );
    let rows: Vec<_> = notification
        .list_of_cov_notifications
        .iter()
        .filter(|item| item.monitored_object_identifier == av1())
        .flat_map(|item| &item.list_of_values)
        .filter(|value| value.property_identifier == PV)
        .collect();
    let last = rows
        .last()
        .and_then(|value| value.time_of_change)
        .expect("a timestamped PV row");
    assert_eq!(
        envelope(notification),
        Some((at(0).local_date, last)),
        "the envelope names the last change carried"
    );
}

/// AV-1's PV rows of a notification that may also carry other objects.
fn av1_pv_rows(notification: &COVNotificationMultipleRequest) -> Vec<Row> {
    notification
        .list_of_cov_notifications
        .iter()
        .filter(|item| item.monitored_object_identifier == av1())
        .flat_map(|item| &item.list_of_values)
        .filter(|value| value.property_identifier == PV)
        .map(|value| (value.value.clone(), value.time_of_change))
        .collect()
}

/// Queue PV changes at seconds `first..first + count` behind
/// DISABLE_INITIATION, then re-enable and change once more, which reports
/// them all. Returns every change as its expected row, in capture order.
async fn hold_then_release(h: &Harness, first: u8, count: u8) -> Vec<Row> {
    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    let mut expected = Vec::new();
    for second in first..=first + count {
        if second == first + count {
            h.server.comm_state.set_for_test(DccState::Enable);
        }
        h.set_clock(second);
        let value = f32::from(second);
        h.write_local(value).await;
        expected.push((real(value), Some(time(second))));
    }
    expected
}

/// Unconfirmed notifications up to the one carrying `expected`'s last change,
/// each checked against `limit`.
async fn take_unconfirmed(
    h: &Harness,
    expected: &[Row],
    limit: u16,
) -> Vec<COVNotificationMultipleRequest> {
    let mut taken = Vec::new();
    let mut conveyed = Vec::new();
    while conveyed.last() != expected.last() {
        let notification = h.notification().await;
        check(&notification, limit, false);
        conveyed.extend(av1_pv_rows(&notification));
        taken.push(notification);
    }
    assert_eq!(conveyed, expected, "every change once, oldest first");
    taken
}

#[tokio::test(start_paused = true)]
async fn unconfirmed_history_beyond_one_apdu_goes_out_in_several_notifications() {
    let mut h = Harness::start(ServerConfig {
        device: bacnet_objects::DeviceIdentity::default()
            .with_max_apdu(u16::try_from(u32::from(SMALL_APDU)).expect("Device APDU fits u16"))
            .expect("valid Device APDU"),
        ..ServerConfig::default()
    })
    .await;
    h.subscribe(false).await;
    h.notification().await;
    let expected = hold_then_release(&h, 1, HELD).await;
    let taken = take_unconfirmed(&h, &expected, SMALL_APDU).await;
    assert!(taken.len() >= 2, "{} notifications", taken.len());
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
    // Each was retired when it was sent.
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn notifications_fit_the_maximum_apdu_the_subscriber_advertised() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.request_max_apdu = SMALL_APDU;
    h.subscribe(false).await;
    check(&h.notification().await, SMALL_APDU, false);
    let expected = hold_then_release(&h, 1, HELD).await;
    let taken = take_unconfirmed(&h, &expected, SMALL_APDU).await;
    assert!(taken.len() >= 2, "{} notifications", taken.len());
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn history_beyond_the_bound_drops_the_oldest_changes_and_counts_them() {
    let mut h = Harness::start(ServerConfig {
        device: bacnet_objects::DeviceIdentity::default()
            .with_max_apdu(u16::try_from(u32::from(SMALL_APDU)).expect("Device APDU fits u16"))
            .expect("valid Device APDU"),
        ..ServerConfig::default()
    })
    .await;
    h.subscribe(false).await;
    h.notification().await;
    let all = hold_then_release(&h, 1, 40).await;
    let dropped = h.server.cov_counters().timed_changes_dropped;
    assert!(dropped > 0, "41 changes exceed the bound");
    // The newest changes survive, several notifications' worth.
    let kept = &all[usize::try_from(dropped).unwrap()..];
    let taken = take_unconfirmed(&h, kept, SMALL_APDU).await;
    assert!(taken.len() >= 2, "{} notifications", taken.len());
    assert_eq!(
        h.server.cov_counters().timed_changes_dropped,
        dropped,
        "sending drops nothing more"
    );
    h.server.stop().await.unwrap();
}

fn av2() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 2).unwrap()
}

#[tokio::test(start_paused = true)]
async fn untimestamped_values_go_in_the_last_notification() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(
            AnalogValueObject::new(2, "AV-2", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
        ))
        .unwrap();
    })
    .await;
    h.request_max_apdu = SMALL_APDU;
    h.subscribe_specs(
        false,
        vec![(av1(), vec![(PV, true)]), (av2(), vec![(PV, false)])],
    )
    .await;
    h.notification().await;
    // AV-1's changes queue; AV-2's untimestamped change then reports them.
    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    let mut expected = Vec::new();
    for second in 1..=HELD {
        h.set_clock(second);
        h.write_local(f32::from(second)).await;
        expected.push((real(f32::from(second)), Some(time(second))));
    }
    h.server.comm_state.set_for_test(DccState::Enable);
    h.set_clock(16);
    h.write_local_to(av2(), 7.0).await;
    let mut taken = Vec::new();
    let mut conveyed = Vec::new();
    while conveyed.last() != expected.last() {
        let notification = h.notification().await;
        check(&notification, SMALL_APDU, false);
        conveyed.extend(av1_pv_rows(&notification));
        taken.push(notification);
    }
    assert_eq!(conveyed, expected);
    assert!(taken.len() >= 2, "{} notifications", taken.len());
    let carries_av2 = |notification: &COVNotificationMultipleRequest| {
        notification
            .list_of_cov_notifications
            .iter()
            .any(|item| item.monitored_object_identifier == av2())
    };
    let (last, earlier) = taken.split_last().unwrap();
    assert!(
        earlier.iter().all(|n| !carries_av2(n)),
        "history goes first"
    );
    let av2_pv: Vec<_> = last
        .list_of_cov_notifications
        .iter()
        .filter(|item| item.monitored_object_identifier == av2())
        .flat_map(|item| &item.list_of_values)
        .filter(|value| value.property_identifier == PV)
        .map(|value| (value.value.clone(), value.time_of_change))
        .collect();
    assert_eq!(av2_pv, vec![(real(7.0), None)]);
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn confirmed_history_chunks_go_out_one_per_acknowledgment() {
    let mut h = Harness::start(ServerConfig {
        device: bacnet_objects::DeviceIdentity::default()
            .with_max_apdu(u16::try_from(u32::from(SMALL_APDU)).expect("Device APDU fits u16"))
            .expect("valid Device APDU"),
        ..ServerConfig::default()
    })
    .await;
    h.subscribe(true).await;
    h.notification().await;
    h.ack().await;
    h.settle().await;
    let expected = hold_then_release(&h, 1, HELD).await;
    let mut conveyed = Vec::new();
    let mut count = 0;
    loop {
        let notification = h.notification().await;
        check(&notification, SMALL_APDU, true);
        conveyed.extend(av1_pv_rows(&notification));
        count += 1;
        if conveyed.last() == expected.last() {
            break;
        }
        // The rest stays queued, uncounted, until this chunk's Ack.
        h.no_notification().await;
        assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
        h.ack().await;
    }
    h.ack().await;
    h.settle().await;
    assert!(count >= 2, "{count} notifications");
    assert_eq!(conveyed, expected, "every change once, oldest first");
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

/// Hold-off after a failed confirmed report with a 10 ms retry timeout.
const HOLD_OFF: Duration = Duration::from_millis(10 * (DEFAULT_APDU_RETRIES as u64 + 1));

#[tokio::test(start_paused = true)]
async fn a_failed_confirmed_chunk_goes_out_again_before_the_rest() {
    let mut h = Harness::start(ServerConfig {
        device: bacnet_objects::DeviceIdentity::default()
            .with_max_apdu(u16::try_from(u32::from(SMALL_APDU)).expect("Device APDU fits u16"))
            .expect("valid Device APDU"),
        cov_retry_timeout_ms: 10,
        ..ServerConfig::default()
    })
    .await;
    h.subscribe_with_delay(true, vec![(av1(), vec![(PV, true)])], 1)
        .await;
    h.notification().await;
    h.ack().await;
    h.settle().await;
    let expected = hold_then_release(&h, 1, HELD).await;
    let first = h.notification().await;
    check(&first, SMALL_APDU, true);
    assert_ne!(av1_pv_rows(&first).last(), expected.last(), "a first chunk");
    // Never acknowledged: its retries run out and the context holds off; the
    // Max_Notification_Delay backstop then reports the context again.
    h.workers_idle().await;
    tokio::time::sleep(HOLD_OFF).await;
    let again = h.notification().await;
    assert_eq!(av1_pv_rows(&again), av1_pv_rows(&first));
    let mut conveyed = av1_pv_rows(&again);
    while conveyed.last() != expected.last() {
        h.ack().await;
        let notification = h.notification().await;
        check(&notification, SMALL_APDU, true);
        conveyed.extend(av1_pv_rows(&notification));
    }
    h.ack().await;
    assert_eq!(conveyed, expected);
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn chunks_an_exhausted_budget_held_back_go_out_at_the_deadline() {
    let mut h = Harness::start(ServerConfig {
        cov_policy: crate::cov::CovPolicy {
            max_notifications_per_event: 1,
            ..crate::cov::CovPolicy::default()
        },
        ..ServerConfig::default()
    })
    .await;
    h.request_max_apdu = SMALL_APDU;
    h.subscribe_with_delay(false, vec![(av1(), vec![(PV, true)])], 2)
        .await;
    h.notification().await;
    let expected = hold_then_release(&h, 1, HELD).await;
    let released = TokioInstant::now();
    let first = h.notification().await;
    check(&first, SMALL_APDU, false);
    // One notification per event: the rest waits for the backstop.
    h.no_notification().await;
    let mut conveyed = av1_pv_rows(&first);
    let mut count = 1;
    while conveyed.last() != expected.last() {
        let notification = h.notification().await;
        check(&notification, SMALL_APDU, false);
        conveyed.extend(av1_pv_rows(&notification));
        count += 1;
    }
    assert!(count >= 2, "{count} notifications");
    assert!(
        released.elapsed() >= Duration::from_secs(2),
        "later chunks waited for the deadline"
    );
    assert_eq!(conveyed, expected);
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
    h.server.stop().await.unwrap();
}

/// AV-1 PV rows of every notification up to the one carrying `last`.
async fn take_through(h: &Harness, last: &Row, limit: u16) -> Vec<Row> {
    let mut conveyed = Vec::new();
    while conveyed.last() != Some(last) {
        let notification = h.notification().await;
        check(&notification, limit, false);
        conveyed.extend(av1_pv_rows(&notification));
    }
    conveyed
}

#[tokio::test(start_paused = true)]
async fn a_report_going_out_holds_back_a_later_one_until_its_last_part() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.request_max_apdu = SMALL_APDU;
    h.subscribe(false).await;
    h.notification().await;
    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    let mut expected = Vec::new();
    for second in 1..HELD {
        h.set_clock(second);
        h.write_local(f32::from(second)).await;
        expected.push((real(f32::from(second)), Some(time(second))));
    }
    h.server.comm_state.set_for_test(DccState::Enable);
    // The release goes through the network, so its report is sent from the
    // server's task; the transport holds that report's second part.
    let release = h.hold_notification(1);
    h.set_clock(HELD);
    h.write_pv(f32::from(HELD), HELD).await;
    expected.push((real(f32::from(HELD)), Some(time(HELD))));
    let first = h.notification().await;
    check(&first, SMALL_APDU, false);
    // A newer change while that report is still going out stands back: it
    // must not reach the subscriber ahead of the older parts (#986).
    h.set_clock(20);
    h.write_local(20.0).await;
    h.no_notification().await;
    release.add_permits(1);
    expected.push((real(20.0), Some(time(20))));
    let mut conveyed = av1_pv_rows(&first);
    conveyed.extend(take_through(&h, expected.last().unwrap(), SMALL_APDU).await);
    assert_eq!(conveyed, expected, "capture order across both reports");
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn disabling_communication_between_parts_leaves_the_rest_until_reenabled() {
    use bacnet_types::enums::EnableDisable;
    let mut h = Harness::start(ServerConfig {
        dcc_policy: crate::server::DccPolicy::LegacyPermissive,
        ..ServerConfig::default()
    })
    .await;
    h.request_max_apdu = SMALL_APDU;
    h.subscribe_with_delay(false, vec![(av1(), vec![(PV, true)])], 1)
        .await;
    h.notification().await;
    // Initiation is disabled right after the first part goes out.
    h.disable_after_notification(0);
    let expected = hold_then_release(&h, 1, HELD).await;
    let first = h.notification().await;
    check(&first, SMALL_APDU, false);
    assert_ne!(av1_pv_rows(&first).last(), expected.last(), "a first part");
    // Clause 16.1: nothing more while initiation is disabled, not even at the
    // Max_Notification_Delay deadline.
    tokio::time::sleep(Duration::from_secs(3)).await;
    h.no_notification().await;
    h.dcc(EnableDisable::ENABLE, None).await;
    let mut conveyed = av1_pv_rows(&first);
    conveyed.extend(take_through(&h, expected.last().unwrap(), SMALL_APDU).await);
    assert_eq!(conveyed, expected);
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_part_whose_send_fails_goes_out_again_with_the_rest_at_the_deadline() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.request_max_apdu = SMALL_APDU;
    h.subscribe_with_delay(false, vec![(av1(), vec![(PV, true)])], 1)
        .await;
    h.notification().await;
    h.fail_notification(1);
    let expected = hold_then_release(&h, 1, HELD).await;
    // The first part is sent and retired; the second fails, and it and the
    // rest return to the queue, uncounted, for the backstop.
    let first = h.notification().await;
    let mut conveyed = av1_pv_rows(&first);
    h.no_notification().await;
    // The first part carried only history: the reference completes with the
    // part carrying its latest change, not before.
    assert_eq!(av1_completed(&h).await, Some(PropertyValue::Real(0.0)));
    conveyed.extend(take_through(&h, expected.last().unwrap(), SMALL_APDU).await);
    assert_eq!(conveyed, expected, "the failed part once, in order");
    assert_eq!(
        av1_completed(&h).await,
        Some(PropertyValue::Real(f32::from(1 + HELD)))
    );
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
    h.server.stop().await.unwrap();
}

/// The Present_Value AV-1's timestamped reference last completed.
async fn av1_completed(h: &Harness) -> Option<PropertyValue> {
    let mut table = h.server.cov_table.write().await;
    table
        .subscriptions_for(&av1())
        .into_iter()
        .find(|sub| sub.timestamped)
        .and_then(|sub| sub.last_notified_observation.as_ref())
        .map(|observation| observation.sample().value().clone())
}

/// PV rows of `object` in a notification that may carry several objects.
fn pv_rows_of(notification: &COVNotificationMultipleRequest, object: ObjectIdentifier) -> Vec<Row> {
    notification
        .list_of_cov_notifications
        .iter()
        .filter(|item| item.monitored_object_identifier == object)
        .flat_map(|item| &item.list_of_values)
        .filter(|value| value.property_identifier == PV)
        .map(|value| (value.value.clone(), value.time_of_change))
        .collect()
}

/// Hold alternating changes of AV-1 and AV-2, then release them with one
/// more AV-1 change. Returns each object's expected rows in capture order.
async fn hold_interleaved(h: &Harness) -> (Vec<Row>, Vec<Row>) {
    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    let (mut first, mut second) = (Vec::new(), Vec::new());
    for at_second in 1..=HELD {
        if at_second == HELD {
            h.server.comm_state.set_for_test(DccState::Enable);
        }
        h.set_clock(at_second);
        let value = f32::from(at_second);
        let (object, rows) = if at_second % 2 == 1 {
            (av1(), &mut first)
        } else {
            (av2(), &mut second)
        };
        h.write_local_to(object, value).await;
        rows.push((real(value), Some(time(at_second))));
    }
    (first, second)
}

async fn two_object_harness(confirmed: bool) -> Harness {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(
            AnalogValueObject::new(2, "AV-2", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
        ))
        .unwrap();
    })
    .await;
    h.request_max_apdu = SMALL_APDU;
    h.subscribe_specs(
        confirmed,
        vec![(av1(), vec![(PV, true)]), (av2(), vec![(PV, true)])],
    )
    .await;
    h.notification().await;
    if confirmed {
        h.ack().await;
        h.settle().await;
    }
    h
}

/// The newest PV time a notification carries, across its objects.
fn newest_pv_time(notification: &COVNotificationMultipleRequest) -> Option<Time> {
    [av1(), av2()]
        .into_iter()
        .flat_map(|object| pv_rows_of(notification, object))
        .filter_map(|(_, time)| time)
        .max_by_key(|time| time.second)
}

#[tokio::test(start_paused = true)]
async fn several_references_keep_their_own_order_across_unconfirmed_parts() {
    let mut h = two_object_harness(false).await;
    let (first, second) = hold_interleaved(&h).await;
    let (mut got_first, mut got_second) = (Vec::new(), Vec::new());
    let mut count = 0;
    while got_first.last() != first.last() {
        let notification = h.notification().await;
        let len = apdu_len(&notification, false);
        assert!(len <= usize::from(SMALL_APDU), "{len} octets");
        assert_eq!(
            notification.timestamp.map(|(_, time)| time),
            newest_pv_time(&notification),
            "the header names the last change carried"
        );
        got_first.extend(pv_rows_of(&notification, av1()));
        got_second.extend(pv_rows_of(&notification, av2()));
        count += 1;
    }
    assert!(count >= 2, "{count} notifications");
    assert_eq!(got_first, first);
    assert_eq!(got_second, second);
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_confirmed_report_carries_several_references_part_by_part() {
    let mut h = two_object_harness(true).await;
    let (first, second) = hold_interleaved(&h).await;
    let (mut got_first, mut got_second) = (Vec::new(), Vec::new());
    let mut count = 0;
    loop {
        let notification = h.notification().await;
        assert!(apdu_len(&notification, true) <= usize::from(SMALL_APDU));
        got_first.extend(pv_rows_of(&notification, av1()));
        got_second.extend(pv_rows_of(&notification, av2()));
        count += 1;
        if got_first.last() == first.last() {
            break;
        }
        h.no_notification().await;
        h.ack().await;
    }
    h.ack().await;
    h.settle().await;
    assert!(count >= 2, "{count} notifications");
    assert_eq!(got_first, first);
    assert_eq!(got_second, second);
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn cancelling_while_parts_are_deferred_holds_and_sends_nothing() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.request_max_apdu = SMALL_APDU;
    h.subscribe(true).await;
    h.notification().await;
    h.ack().await;
    h.settle().await;
    hold_then_release(&h, 1, HELD).await;
    // The first part is outstanding; the rest is deferred in the queue.
    h.notification().await;
    h.cancel_specs(true, vec![(av1(), vec![(PV, true)])]).await;
    h.settle().await;
    assert_eq!(
        h.server.cov_table.read().await.timed().lock().held(),
        (0, 0),
        "the cancelled reference took its deferred parts along"
    );
    h.ack().await;
    h.settle().await;
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_value_too_large_for_any_notification_is_dropped_and_counted() {
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
    h.subscribe_specs(false, vec![(csv, vec![(PV, true)])])
        .await;
    h.notification().await;
    let write = |text: String| {
        let server = &h.server;
        async move {
            server
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
    };
    // Two 200-character values: each change alone is larger than a 206-octet
    // notification, though the context's bound holds both.
    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    h.set_clock(1);
    write("a".repeat(200)).await;
    h.server.comm_state.set_for_test(DccState::Enable);
    h.set_clock(2);
    write("b".repeat(200)).await;
    // Neither value could ever be sent, the latest no more than the earlier
    // one, so both are dropped and counted, and nothing goes over the limit
    // (#1008). The rest of each change, its Status_Flags, goes out alone, in
    // capture order (#1090).
    for second in 1..=2 {
        let flags = h.notification().await;
        let carried: Vec<_> = flags.list_of_cov_notifications[0]
            .list_of_values
            .iter()
            .map(|value| (value.property_identifier, value.time_of_change))
            .collect();
        assert_eq!(carried, vec![(SF, Some(time(second)))]);
    }
    h.no_notification().await;
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 2);
    // Nothing is left to hold the context back.
    h.set_clock(3);
    write("c".into()).await;
    let report = h.notification().await;
    let rows: Vec<_> = report.list_of_cov_notifications[0]
        .list_of_values
        .iter()
        .filter(|value| value.property_identifier == PV)
        .map(|value| value.time_of_change)
        .collect();
    assert_eq!(rows, vec![Some(time(3))]);
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 2);
    h.no_notification().await;
    h.server.stop().await.unwrap();
}
