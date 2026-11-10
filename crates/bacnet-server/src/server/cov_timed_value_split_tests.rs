//! A timestamped COV-multiple change too large for one notification even on
//! its own goes out one value per notification, in the order its values were
//! captured (135-2020 §13.1, §13.18.1.1; #1090).
//!
//! Each of those notifications carries the change's Time_Of_Change on its
//! value, and its envelope names the change. Only the notification with the
//! change's last value completes the reference; values whose notification
//! fails come back as one change and go out again value by value. Changes
//! that fit a notification still go out whole, as before. Time is paused.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;
use bacnet_types::primitives::Time;

/// The smallest maximum APDU a request can advertise. One timestamped
/// Present_Value and Status_Flags change of AV-1 takes 60 octets or more,
/// each value alone 50 at most with this harness's envelope.
const TINY_APDU: u16 = 50;
const SMALL_APDU: u16 = 206;

/// Characters of a CharacterString value that fits a 206-octet notification
/// alone but not together with its change's Status_Flags.
const LONG: usize = 155;

/// One conveyed value: property, encoded value and Time_Of_Change.
type Value = (PropertyIdentifier, Vec<u8>, Option<Time>);

/// Encoded APDU length of `notification`: its service request plus the
/// unconfirmed (2 octets) or unsegmented confirmed (4 octets) header.
fn apdu_len(notification: &COVNotificationMultipleRequest, confirmed: bool) -> usize {
    let mut encoded = BytesMut::new();
    notification.encode(&mut encoded).unwrap();
    encoded.len() + if confirmed { 4 } else { 2 }
}

/// The values of `notification`, once checked that it fits `limit`, carries
/// one object and names the time of its last value in its envelope.
fn values(
    notification: &COVNotificationMultipleRequest,
    limit: u16,
    confirmed: bool,
) -> Vec<Value> {
    let len = apdu_len(notification, confirmed);
    assert!(
        len <= usize::from(limit),
        "{len}-octet APDU exceeds {limit}"
    );
    assert_eq!(notification.list_of_cov_notifications.len(), 1);
    let values: Vec<Value> = notification.list_of_cov_notifications[0]
        .list_of_values
        .iter()
        .map(|value| {
            (
                value.property_identifier,
                value.value.clone(),
                value.time_of_change,
            )
        })
        .collect();
    let last = values.last().and_then(|(_, _, time)| *time);
    assert_eq!(
        envelope(notification).map(|(_, time)| time),
        last,
        "the envelope names the change of its last value"
    );
    values
}

/// The values of each of the next `count` notifications, acknowledging each
/// when confirmed.
async fn take(h: &Harness, count: usize, limit: u16, confirmed: bool) -> Vec<Vec<Value>> {
    let mut taken = Vec::new();
    for _ in 0..count {
        let notification = h.notification().await;
        taken.push(values(&notification, limit, confirmed));
        if confirmed {
            h.ack().await;
        }
    }
    if confirmed {
        h.settle().await;
    }
    taken
}

/// AV-1's encoded Status_Flags with no flag set.
fn normal_flags() -> Vec<u8> {
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(
        &mut encoded,
        &PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0],
        },
    )
    .unwrap();
    encoded.to_vec()
}

/// A change of AV-1 to `value` at `second`, one value per notification:
/// Present_Value, then Status_Flags.
fn av1_apart(value: f32, second: u8) -> Vec<Vec<Value>> {
    vec![
        vec![(PV, real(value), Some(time(second)))],
        vec![(SF, normal_flags(), Some(time(second)))],
    ]
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

/// A harness whose one context subscribes AV-1's Present_Value with
/// timestamps from a 50-octet subscriber, with the given delay, the initial
/// report's two notifications taken and checked.
async fn tiny_harness(config: ServerConfig, confirmed: bool, delay: u32) -> Harness {
    let mut h = Harness::start(config).await;
    h.request_max_apdu = TINY_APDU;
    h.subscribe_with_delay(confirmed, vec![(av1(), vec![(PV, true)])], delay)
        .await;
    assert_eq!(
        take(&h, 2, TINY_APDU, confirmed).await,
        av1_apart(0.0, 0),
        "confirmed: {confirmed}"
    );
    h
}

#[tokio::test(start_paused = true)]
async fn a_50_octet_subscriber_gets_each_timestamped_change_one_value_per_notification() {
    for confirmed in [false, true] {
        let warnings = crate::cov::timed_store::DropWarningCount::default();
        let _guard = warnings.install();
        // Even the initial report's change does not fit one notification:
        // its Present_Value goes first, then its Status_Flags, both with the
        // change's time.
        let mut h = tiny_harness(ServerConfig::default(), confirmed, 10).await;
        for second in 1..=2 {
            h.set_clock(second);
            h.write_local(f32::from(second)).await;
            assert_eq!(
                take(&h, 2, TINY_APDU, confirmed).await,
                av1_apart(f32::from(second), second),
                "confirmed: {confirmed}"
            );
        }
        h.no_notification().await;
        assert_eq!(av1_completed(&h).await, Some(PropertyValue::Real(2.0)));
        // Without timestamps the same values fit one notification.
        h.subscribe_process(857, confirmed, vec![(av1(), vec![(PV, false)])], Some(10))
            .await;
        let report = h.notification().await;
        assert!(apdu_len(&report, confirmed) <= usize::from(TINY_APDU));
        assert_eq!(pv_rows(&report), vec![(real(2.0), None)]);
        if confirmed {
            h.ack().await;
        }
        h.no_notification().await;
        // Nothing was lost, so nothing is counted or logged (#1039).
        assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
        assert_eq!(warnings.get(), 0);
        h.server.stop().await.unwrap();
    }
}

/// A harness whose one context subscribes CSV-1's Present_Value with
/// timestamps from a 206-octet subscriber, with the initial report taken
/// (and acknowledged, when confirmed).
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

async fn write_string(h: &Harness, csv: ObjectIdentifier, text: &str) {
    h.server
        .write_local(
            &csv,
            PV,
            None,
            PropertyValue::CharacterString(text.into()),
            Some(8),
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
}

fn encoded_text(text: &str) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(
        &mut encoded,
        &PropertyValue::CharacterString(text.into()),
    )
    .unwrap();
    encoded.to_vec()
}

#[tokio::test(start_paused = true)]
async fn changes_that_fit_go_out_whole_between_ones_sent_value_by_value() {
    for confirmed in [false, true] {
        let (mut h, csv) = string_harness(confirmed).await;
        let (a, b, c) = ("a".repeat(LONG), "b".to_string(), "c".repeat(LONG));
        // Three changes held back, then released together: the long ones do
        // not fit a notification with their Status_Flags, the short one does.
        h.server
            .comm_state
            .set_for_test(DccState::DisableInitiation);
        h.set_clock(1);
        write_string(&h, csv, &a).await;
        h.set_clock(2);
        write_string(&h, csv, &b).await;
        h.server.comm_state.set_for_test(DccState::Enable);
        h.set_clock(3);
        write_string(&h, csv, &c).await;
        let flags = |second| (SF, normal_flags(), Some(time(second)));
        let text = |text: &str, second| (PV, encoded_text(text), Some(time(second)));
        let expected = if confirmed {
            // One part per Ack: once the first long value is through, the
            // rest of its change fits and goes out with the next one.
            vec![
                vec![text(&a, 1)],
                vec![flags(1), text(&b, 2), flags(2)],
                vec![text(&c, 3)],
                vec![flags(3)],
            ]
        } else {
            vec![
                vec![text(&a, 1)],
                vec![flags(1)],
                vec![text(&b, 2), flags(2)],
                vec![text(&c, 3)],
                vec![flags(3)],
            ]
        };
        let taken = take(&h, expected.len(), SMALL_APDU, confirmed).await;
        assert_eq!(taken, expected, "confirmed: {confirmed}");
        h.no_notification().await;
        assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
        // A change that fits on its own goes out whole, as before.
        h.set_clock(4);
        write_string(&h, csv, "d").await;
        assert_eq!(
            take(&h, 1, SMALL_APDU, confirmed).await,
            vec![vec![text("d", 4), flags(4)]],
            "confirmed: {confirmed}"
        );
        h.no_notification().await;
        h.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn a_value_whose_send_fails_goes_out_again_alone_at_the_deadline() {
    let mut h = tiny_harness(ServerConfig::default(), false, 1).await;
    h.fail_notification(1);
    h.set_clock(1);
    h.write_local(1.0).await;
    // The Present_Value part is sent and retired; the Status_Flags part fails
    // and returns to the queue, uncounted.
    let first = h.notification().await;
    assert_eq!(
        vec![values(&first, TINY_APDU, false)],
        av1_apart(1.0, 1)[..1]
    );
    h.no_notification().await;
    // Only the part with the change's last value completes the reference.
    assert_eq!(av1_completed(&h).await, Some(PropertyValue::Real(0.0)));
    // The Max_Notification_Delay backstop sends the rest of the change, its
    // Present_Value not again.
    assert_eq!(take(&h, 1, TINY_APDU, false).await, av1_apart(1.0, 1)[1..]);
    h.no_notification().await;
    assert_eq!(av1_completed(&h).await, Some(PropertyValue::Real(1.0)));
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_rejected_confirmed_value_goes_out_again_before_the_rest_of_its_change() {
    let config = ServerConfig {
        cov_retry_timeout_ms: 10,
        ..ServerConfig::default()
    };
    let mut h = tiny_harness(config, true, 1).await;
    h.set_clock(1);
    h.write_local(1.0).await;
    let first = h.notification().await;
    assert_eq!(
        vec![values(&first, TINY_APDU, true)],
        av1_apart(1.0, 1)[..1]
    );
    // The subscriber refuses it: the Present_Value rejoins the rest of its
    // change, and after the hold-off the change goes out again, value by
    // value, in order.
    h.reject().await;
    h.no_notification().await;
    assert_eq!(take(&h, 2, TINY_APDU, true).await, av1_apart(1.0, 1));
    h.no_notification().await;
    assert_eq!(av1_completed(&h).await, Some(PropertyValue::Real(1.0)));
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 0);
    h.server.stop().await.unwrap();
}

/// DISABLE_INITIATION ends a confirmed report at its first retry while it is
/// sending a change value by value (#1327). The value rejoins its change, which
/// stays in delivery (#1163): when later changes captured meanwhile overflow
/// the 50-octet bound, eviction passes over it. Once communication is enabled
/// what is held goes out value by value in capture order, with no hold-off.
#[tokio::test(start_paused = true)]
async fn a_value_dcc_withdraws_goes_out_again_with_the_rest_of_its_change() {
    use bacnet_types::enums::EnableDisable;
    let config = ServerConfig {
        dcc_policy: crate::server::DccPolicy::LegacyPermissive,
        ..ServerConfig::default()
    };
    let mut h = tiny_harness(config, true, 1).await;
    h.set_clock(1);
    h.write_local(1.0).await;
    let first = h.notification().await;
    assert_eq!(
        vec![values(&first, TINY_APDU, true)],
        av1_apart(1.0, 1)[..1]
    );
    let (invoke_id, _) = h.take_confirmed();
    h.dcc(EnableDisable::DISABLE_INITIATION, None).await;
    h.workers_idle().await;
    assert!(!h.frames.lock().unwrap().iter().any(
        |apdu| matches!(apdu, Apdu::ConfirmedRequest(request) if request.invoke_id == invoke_id)
    ));
    for second in 2..=3 {
        h.set_clock(second);
        h.write_local(f32::from(second)).await;
    }
    h.no_notification().await;
    // Over the bound, eviction takes the middle change, never the one in
    // delivery nor the newest.
    assert_eq!(h.server.cov_counters().timed_changes_dropped, 1);
    h.dcc(EnableDisable::ENABLE, None).await;
    assert_eq!(
        take(&h, 4, TINY_APDU, true).await,
        [av1_apart(1.0, 1), av1_apart(3.0, 3)].concat()
    );
    h.no_notification().await;
    assert_eq!(av1_completed(&h).await, Some(PropertyValue::Real(3.0)));
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn an_oversized_value_is_counted_each_time_and_warned_once_per_admission() {
    for confirmed in [false, true] {
        let warnings = crate::cov::timed_store::DropWarningCount::default();
        let _guard = warnings.install();
        let (mut h, csv) = string_harness(confirmed).await;
        let seen = |h: &Harness| {
            (
                h.server.cov_counters().timed_changes_dropped,
                warnings.get(),
            )
        };
        let flags = |second| vec![vec![(SF, normal_flags(), Some(time(second)))]];
        // A Present_Value too long for any 206-octet notification is dropped
        // and counted each time; the rest of its change still goes out.
        for second in 1..=3 {
            h.set_clock(second);
            write_string(&h, csv, &"x".repeat(197 + usize::from(second))).await;
            let taken = take(&h, 1, SMALL_APDU, confirmed).await;
            assert_eq!(taken, flags(second), "confirmed: {confirmed}");
        }
        // The context warned once (#1039).
        assert_eq!(seen(&h), (3, 1), "confirmed: {confirmed}");
        // Re-admitting the reference warns afresh, once: its initial report
        // captures the long value again.
        h.subscribe_specs(confirmed, vec![(csv, vec![(PV, true)])])
            .await;
        assert_eq!(take(&h, 1, SMALL_APDU, confirmed).await, flags(3));
        h.set_clock(4);
        write_string(&h, csv, &"x".repeat(210)).await;
        assert_eq!(take(&h, 1, SMALL_APDU, confirmed).await, flags(4));
        h.no_notification().await;
        assert_eq!(seen(&h), (5, 2), "confirmed: {confirmed}");
        h.server.stop().await.unwrap();
    }
}

/// Change AV-1 to `second` at `second` and take the Present_Value part. Its
/// Status_Flags part stays queued: its send fails when unconfirmed, and a
/// confirmed report defers it until the Present_Value is acknowledged.
async fn hold_back_flags(h: &Harness, confirmed: bool, second: u8) {
    if !confirmed {
        h.fail_notification(1);
    }
    h.set_clock(second);
    h.write_local(f32::from(second)).await;
    let first = h.notification().await;
    assert_eq!(
        vec![values(&first, TINY_APDU, confirmed)],
        av1_apart(f32::from(second), second)[..1]
    );
    h.settle().await;
}

#[tokio::test(start_paused = true)]
async fn a_held_back_value_still_goes_out_after_a_newer_change() {
    for confirmed in [false, true] {
        let warnings = crate::cov::timed_store::DropWarningCount::default();
        let _guard = warnings.install();
        // The bound at a 50-octet subscriber holds two or three changes.
        // Newer changes that overflow it must not evict the value their
        // predecessor still owes (#1163): the unconfirmed context's sends
        // keep failing, its reports returning their changes, and the
        // confirmed one waits for its Ack. As in the next test, three newer
        // changes overflow the unconfirmed room and two the confirmed.
        let mut h = tiny_harness(ServerConfig::default(), confirmed, 1).await;
        hold_back_flags(&h, confirmed, 1).await;
        let newest = if confirmed { 3 } else { 4 };
        for second in 2..=newest {
            if !confirmed {
                h.fail_notification(0);
            }
            h.set_clock(second);
            h.write_local(f32::from(second)).await;
        }
        // The overflow evicts the change after the held one, warning once.
        assert_eq!(
            (
                h.server.cov_counters().timed_changes_dropped,
                warnings.get()
            ),
            (1, 1),
            "confirmed: {confirmed}"
        );
        if confirmed {
            // The newer changes wait behind the outstanding report.
            h.no_notification().await;
            h.ack().await;
        }
        // The held Status_Flags go first, after the backstop's delay when
        // unconfirmed, then the newer changes value by value; the reference
        // completes at the newest.
        let mut expected = av1_apart(1.0, 1).split_off(1);
        for second in 3..=newest {
            expected.extend(av1_apart(f32::from(second), second));
        }
        assert_eq!(
            take(&h, expected.len(), TINY_APDU, confirmed).await,
            expected,
            "confirmed: {confirmed}"
        );
        h.no_notification().await;
        assert_eq!(
            av1_completed(&h).await,
            Some(PropertyValue::Real(f32::from(newest)))
        );
        assert_eq!(h.server.cov_counters().timed_changes_dropped, 1);
        h.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn only_a_real_overflow_drops_a_change_and_never_the_one_in_delivery() {
    for confirmed in [false, true] {
        let mut h = tiny_harness(ServerConfig::default(), confirmed, 1).await;
        hold_back_flags(&h, confirmed, 1).await;
        // More changes queue behind the held Status_Flags, the unconfirmed
        // context blocked meanwhile and the confirmed one waiting for its
        // Ack, until they pass the room four notifications have for items:
        // 92 octets unconfirmed and 84 confirmed, whose header is longer.
        // The held part takes 19 and each change 33 (#1287).
        if !confirmed {
            h.server
                .comm_state
                .set_for_test(DccState::DisableInitiation);
        }
        let newest = if confirmed { 3 } else { 4 };
        for second in 2..=newest {
            h.set_clock(second);
            h.write_local(f32::from(second)).await;
        }
        // Over the bound: the change after the one in delivery is evicted,
        // and it is the only change counted.
        assert_eq!(
            h.server.cov_counters().timed_changes_dropped,
            1,
            "confirmed: {confirmed}"
        );
        if confirmed {
            h.ack().await;
        } else {
            h.server.comm_state.set_for_test(DccState::Enable);
        }
        // The Status_Flags still complete their change, before the rest.
        let mut expected = av1_apart(1.0, 1).split_off(1);
        for second in 3..=newest {
            expected.extend(av1_apart(f32::from(second), second));
        }
        assert_eq!(
            take(&h, expected.len(), TINY_APDU, confirmed).await,
            expected,
            "confirmed: {confirmed}"
        );
        h.no_notification().await;
        assert_eq!(
            av1_completed(&h).await,
            Some(PropertyValue::Real(f32::from(newest)))
        );
        assert_eq!(h.server.cov_counters().timed_changes_dropped, 1);
        h.server.stop().await.unwrap();
    }
}
