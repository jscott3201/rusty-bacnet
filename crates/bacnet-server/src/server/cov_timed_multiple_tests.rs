//! Timestamped COV-multiple reports carry each change's commit time (#856).
//!
//! The transport advances the Device clock when it sends the WriteProperty
//! SimpleACK. The server always sends that response after the mutation and
//! before its COV fanout, so preparation time differs from commit time without
//! any production hook.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_types::enums::EngineeringUnits;

#[tokio::test]
async fn initial_report_carries_admission_time() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.set_clock(1);
    *h.after_ack.lock().unwrap() = Some(at(2));
    h.subscribe(false).await;
    let initial = h.notification().await;
    assert_eq!(pv_rows(&initial), vec![(real(0.0), Some(time(1)))]);
    assert!(rows(&initial)
        .iter()
        .all(|(_, _, time_of_change)| *time_of_change == Some(time(1))));
    assert_eq!(envelope(&initial), Some((at(1).local_date, time(1))));
    h.server.stop().await.unwrap();
}

#[tokio::test]
async fn initial_report_captured_at_admission_survives_an_invalid_clock_at_preparation() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.set_clock(3);
    // Admission samples a valid clock; it is invalid by the time the initial
    // report is prepared.
    let mut invalid = at(3);
    invalid.local_time.hour = 24;
    *h.after_ack.lock().unwrap() = Some(invalid);
    h.subscribe(false).await;
    let initial = h.notification().await;
    assert_eq!(pv_rows(&initial), vec![(real(0.0), Some(time(3)))]);
    assert_eq!(envelope(&initial), Some((at(3).local_date, time(3))));
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn write_property_change_reports_commit_time_not_preparation_time() {
    for confirmed in [false, true] {
        let mut h = Harness::start(ServerConfig::default()).await;
        h.subscribe(confirmed).await;
        h.notification().await;
        if confirmed {
            h.ack().await;
            h.settle().await;
        }
        h.set_clock(10);
        h.write_pv(42.0, 20).await;
        let report = h.notification().await;
        assert_eq!(
            pv_rows(&report),
            vec![(real(42.0), Some(time(10)))],
            "confirmed={confirmed}"
        );
        assert!(rows(&report)
            .iter()
            .all(|(_, _, time_of_change)| *time_of_change == Some(time(10))));
        assert_eq!(envelope(&report), Some((at(10).local_date, time(10))));
        assert_eq!(
            *h.clock.0.lock().unwrap(),
            at(20),
            "prepared after the clock moved"
        );
        h.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn changes_held_while_notifications_are_suppressed_are_all_reported_in_order() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.subscribe(false).await;
    h.notification().await;
    // DISABLE_INITIATION suppresses notifications but not local changes.
    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    h.set_clock(11);
    h.write_local(10.0).await;
    h.set_clock(12);
    h.write_local(20.0).await;
    h.no_notification().await;
    h.server.comm_state.set_for_test(DccState::Enable);
    h.set_clock(13);
    h.write_local(10.0).await;
    let report = h.notification().await;
    assert_eq!(
        pv_rows(&report),
        vec![
            (real(10.0), Some(time(11))),
            (real(20.0), Some(time(12))),
            (real(10.0), Some(time(13))),
        ],
        "A-B-A history is conveyed, each change at its own time"
    );
    let flags: Vec<_> = rows(&report)
        .into_iter()
        .filter(|(property, _, _)| *property == SF)
        .map(|(_, _, time_of_change)| time_of_change)
        .collect();
    assert_eq!(flags, vec![Some(time(11)), Some(time(12)), Some(time(13))]);
    assert_eq!(envelope(&report), Some((at(13).local_date, time(13))));
    h.server.stop().await.unwrap();
}

#[tokio::test]
async fn failed_send_keeps_changes_for_the_next_notification() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.subscribe(false).await;
    h.notification().await;
    h.fail_notifications.store(true, Ordering::Release);
    h.set_clock(21);
    h.write_local(5.0).await;
    h.no_notification().await;
    h.fail_notifications.store(false, Ordering::Release);
    h.set_clock(22);
    h.write_local(6.0).await;
    let report = h.notification().await;
    assert_eq!(
        pv_rows(&report),
        vec![(real(5.0), Some(time(21))), (real(6.0), Some(time(22)))]
    );
    // Retired once sent: an unchanged fanout conveys nothing further.
    h.set_clock(23);
    h.write_local(6.0).await;
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

/// Hold-off after a failed confirmed report with a 10 ms retry timeout: one
/// full retry cycle, the timeout times the first attempt and every retry.
const HOLD_OFF: Duration = Duration::from_millis(10 * (DEFAULT_APDU_RETRIES as u64 + 1));

#[tokio::test(start_paused = true)]
async fn confirmed_changes_retire_once_acknowledged() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.subscribe(true).await;
    h.notification().await;
    h.ack().await;
    h.settle().await;
    h.set_clock(41);
    h.write_local(1.0).await;
    assert_eq!(
        pv_rows(&h.notification().await),
        vec![(real(1.0), Some(time(41)))]
    );
    // Captured while that report is outstanding, so it waits (#896).
    h.set_clock(42);
    h.write_local(2.0).await;
    h.no_notification().await;
    // The Ack retires 1.0 and the held change follows without another write.
    h.ack().await;
    assert_eq!(
        pv_rows(&h.notification().await),
        vec![(real(2.0), Some(time(42)))]
    );
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn confirmed_changes_unacknowledged_return_for_the_next_report() {
    let mut h = Harness::start(ServerConfig {
        cov_retry_timeout_ms: 10,
        ..ServerConfig::default()
    })
    .await;
    h.subscribe(true).await;
    h.notification().await;
    h.ack().await;
    h.settle().await;
    h.set_clock(45);
    h.write_local(5.0).await;
    h.notification().await;
    // Transmitted with every retry but never acknowledged; then the context
    // holds off for one retry cycle (#896).
    h.workers_idle().await;
    tokio::time::sleep(HOLD_OFF).await;
    h.set_clock(46);
    h.write_local(6.0).await;
    assert_eq!(
        pv_rows(&h.notification().await),
        vec![(real(5.0), Some(time(45))), (real(6.0), Some(time(46)))]
    );
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn confirmed_changes_never_transmitted_return_for_the_next_report() {
    let mut h = Harness::start(ServerConfig {
        cov_retry_timeout_ms: 10,
        ..ServerConfig::default()
    })
    .await;
    h.subscribe(true).await;
    h.notification().await;
    h.ack().await;
    h.settle().await;
    h.fail_notifications.store(true, Ordering::Release);
    h.set_clock(43);
    h.write_local(3.0).await;
    h.workers_idle().await;
    tokio::time::sleep(HOLD_OFF).await;
    h.fail_notifications.store(false, Ordering::Release);
    h.set_clock(44);
    h.write_local(4.0).await;
    assert_eq!(
        pv_rows(&h.notification().await),
        vec![(real(3.0), Some(time(43))), (real(4.0), Some(time(44)))]
    );
    h.server.stop().await.unwrap();
}

/// AV-1 alarms above 80 and reports through Notification Class 0, whose
/// local-broadcast recipient makes the event notification observable.

#[tokio::test]
async fn alarm_status_change_carries_its_transition_time() {
    let mut h = Harness::start_with(ServerConfig::default(), high_limit_alarm).await;
    h.subscribe(false).await;
    h.notification().await;
    h.set_clock(40);
    // The event notification goes out before COV fanout; move the clock then.
    *h.after_broadcast.lock().unwrap() = Some(at(50));
    h.write_local(90.0).await;
    let report = h.notification().await;
    assert_eq!(
        *h.clock.0.lock().unwrap(),
        at(50),
        "event notification was sent"
    );
    let flags: Vec<_> = rows(&report)
        .into_iter()
        .filter(|(property, _, _)| *property == SF)
        .collect();
    assert_eq!(flags.len(), 2, "normal then in-alarm flags: {flags:?}");
    assert_ne!(
        flags[0].1, flags[1].1,
        "the transition changed Status_Flags"
    );
    assert!(
        flags
            .iter()
            .all(|(_, _, time_of_change)| *time_of_change == Some(time(40))),
        "both captured when committed, not when prepared: {flags:?}"
    );
    assert_eq!(envelope(&report), Some((at(40).local_date, time(40))));
    h.server.stop().await.unwrap();
}

fn av2() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 2).unwrap()
}

#[tokio::test]
async fn any_notification_to_the_context_conveys_every_pending_timestamped_change() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(
            AnalogValueObject::new(2, "AV-2", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
        ))
        .unwrap();
    })
    .await;
    h.subscribe_specs(
        false,
        vec![(av1(), vec![(PV, true)]), (av2(), vec![(PV, true)])],
    )
    .await;
    h.notification().await;
    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    h.set_clock(51);
    h.write_local_to(av1(), 11.0).await;
    h.no_notification().await;
    h.server.comm_state.set_for_test(DccState::Enable);
    h.set_clock(52);
    h.write_local_to(av2(), 12.0).await;
    let report = h.notification().await;
    let items: Vec<_> = report
        .list_of_cov_notifications
        .iter()
        .map(|item| {
            let pv: Vec<_> = item
                .list_of_values
                .iter()
                .filter(|value| value.property_identifier == PV)
                .map(|value| (value.value.clone(), value.time_of_change))
                .collect();
            (item.monitored_object_identifier, pv)
        })
        .collect();
    assert!(
        items.contains(&(av1(), vec![(real(11.0), Some(time(51)))])),
        "AV-1's held change travels with AV-2's report: {items:?}"
    );
    assert!(items.contains(&(av2(), vec![(real(12.0), Some(time(52)))])));
    assert_eq!(envelope(&report), Some((at(52).local_date, time(52))));
    // Retired with that notification.
    h.set_clock(53);
    h.write_local_to(av2(), 12.0).await;
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test]
async fn an_explicit_untimestamped_selector_is_never_repeated_or_timestamped() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.subscribe_specs(false, vec![(av1(), vec![(PV, true), (SF, false)])])
        .await;
    h.notification().await;
    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    h.set_clock(54);
    h.write_local(10.0).await;
    h.set_clock(55);
    h.write_local(20.0).await;
    h.server.comm_state.set_for_test(DccState::Enable);
    h.set_clock(56);
    h.write_local(30.0).await;
    let report = h.notification().await;
    assert_eq!(
        pv_rows(&report),
        vec![
            (real(10.0), Some(time(54))),
            (real(20.0), Some(time(55))),
            (real(30.0), Some(time(56))),
        ]
    );
    let flags: Vec<_> = rows(&report)
        .into_iter()
        .filter(|(property, _, _)| *property == SF)
        .collect();
    // One current row only, and untimestamped: the explicit selector governs
    // its coordinate even though Status_Flags did not change in this round,
    // over the timestamped PV reference's companion (§13.17.3.1.2.4).
    assert_eq!(
        flags.iter().map(|(_, _, time)| *time).collect::<Vec<_>>(),
        vec![None],
        "Status_Flags is neither history nor timestamped: {flags:?}"
    );
    h.server.stop().await.unwrap();
}

/// Take AV-1 out of service, which sets its OUT_OF_SERVICE Status_Flags bit.
async fn out_of_service(h: &Harness) {
    h.server
        .write_local(
            &av1(),
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
}

/// Status_Flags times of a single-object notification.
fn flag_times(
    report: &bacnet_services::cov_multiple::COVNotificationMultipleRequest,
) -> Vec<Option<bacnet_types::primitives::Time>> {
    rows(report)
        .into_iter()
        .filter(|(property, _, _)| *property == SF)
        .map(|(_, _, time_of_change)| time_of_change)
        .collect()
}

#[tokio::test]
async fn an_explicit_timestamped_selector_that_did_not_change_keeps_its_last_change_time() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.set_clock(1);
    h.subscribe_specs(false, vec![(av1(), vec![(PV, false), (SF, true)])])
        .await;
    let initial = h.notification().await;
    assert_eq!(flag_times(&initial), vec![Some(time(1))], "admission time");
    // Status_Flags changes at 3: its own selector captures that.
    h.set_clock(3);
    out_of_service(&h).await;
    assert_eq!(flag_times(&h.notification().await), vec![Some(time(3))]);
    // Only PV changes at 5. The untimestamped PV reference carries Status_Flags
    // along, and the field still reports the time its own timestamped selector
    // last saw it change (§13.17.3.1.2.4; #987).
    h.set_clock(5);
    h.write_local(10.0).await;
    let report = h.notification().await;
    assert_eq!(pv_rows(&report), vec![(real(10.0), None)]);
    assert_eq!(flag_times(&report), vec![Some(time(3))]);
    assert_eq!(
        envelope(&report),
        Some((at(3).local_date, time(3))),
        "the last change conveyed"
    );
    h.server.stop().await.unwrap();
}

#[tokio::test]
async fn a_timestamped_companion_keeps_its_time_over_an_unchanged_selector() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.set_clock(1);
    h.subscribe_specs(false, vec![(av1(), vec![(PV, true), (SF, true)])])
        .await;
    assert_eq!(flag_times(&h.notification().await), vec![Some(time(1))]);
    // PV changes at 5 and Status_Flags does not. PV's change carries
    // Status_Flags with its own time; the unchanged Status_Flags selector only
    // fills in a missing time, so it leaves that one alone (#987).
    h.set_clock(5);
    h.write_local(10.0).await;
    let report = h.notification().await;
    assert_eq!(pv_rows(&report), vec![(real(10.0), Some(time(5)))]);
    assert_eq!(flag_times(&report), vec![Some(time(5))]);
    assert_eq!(envelope(&report), Some((at(5).local_date, time(5))));
    h.server.stop().await.unwrap();
}

#[tokio::test]
async fn the_header_names_each_notifications_own_newest_change_even_if_older() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(
            AnalogValueObject::new(2, "AV-2", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
        ))
        .unwrap();
    })
    .await;
    h.set_clock(1);
    h.subscribe_specs(
        false,
        vec![
            (av1(), vec![(PV, false), (SF, true)]),
            (av2(), vec![(PV, true)]),
        ],
    )
    .await;
    h.notification().await;
    h.set_clock(3);
    out_of_service(&h).await;
    assert_eq!(
        envelope(&h.notification().await),
        Some((at(3).local_date, time(3)))
    );
    h.set_clock(4);
    h.write_local_to(av2(), 7.0).await;
    assert_eq!(
        envelope(&h.notification().await),
        Some((at(4).local_date, time(4)))
    );
    // Only AV-1's PV changes at 5. Its Status_Flags field keeps the time of
    // its change at 3, the newest change this notification conveys, so the
    // header goes back from 4 to 3: it describes each notification alone.
    h.set_clock(5);
    h.write_local(10.0).await;
    let report = h.notification().await;
    assert_eq!(pv_rows(&report), vec![(real(10.0), None)]);
    assert_eq!(flag_times(&report), vec![Some(time(3))]);
    assert_eq!(envelope(&report), Some((at(3).local_date, time(3))));
    h.server.stop().await.unwrap();
}

#[tokio::test]
async fn without_a_valid_clock_an_uncaptured_timestamped_field_is_left_out() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.set_clock(1);
    h.subscribe_specs(false, vec![(av1(), vec![(PV, false), (SF, true)])])
        .await;
    h.notification().await;
    // Status_Flags changes while the Device clock is invalid, so nothing
    // captures or times it. The untimestamped PV reference still reports, but
    // the new Status_Flags value its sibling asked to timestamp stays out
    // rather than going out untimed (#987).
    let mut invalid = at(3);
    invalid.local_time.hour = 24;
    *h.clock.0.lock().unwrap() = invalid;
    out_of_service(&h).await;
    let report = h.notification().await;
    assert_eq!(pv_rows(&report), vec![(real(0.0), None)]);
    assert!(flag_times(&report).is_empty(), "{report:?}");
    assert_eq!(envelope(&report), None);
    // With a valid clock again, the next report times it.
    h.set_clock(7);
    h.write_local(10.0).await;
    let report = h.notification().await;
    assert_eq!(flag_times(&report), vec![Some(time(7))]);
    h.server.stop().await.unwrap();
}

#[tokio::test]
async fn a_field_below_the_increment_reports_the_time_its_value_was_committed() {
    const VALUE_SOURCE: PropertyIdentifier = PropertyIdentifier::VALUE_SOURCE;
    let mut h = Harness::start(ServerConfig::default()).await;
    h.pv_increment = 100.0;
    h.set_clock(1);
    h.subscribe_specs(
        false,
        vec![(av1(), vec![(PV, true), (VALUE_SOURCE, false)])],
    )
    .await;
    assert_eq!(
        pv_rows(&h.notification().await),
        vec![(real(0.0), Some(time(1)))]
    );
    // B: 12.0 moves less than the PV selector's increment, so only the
    // Value_Source report carries it. It is timed with its commit at 3, not
    // with the preparation at 4 (#987).
    h.set_clock(3);
    h.write_pv(12.0, 4).await;
    assert_eq!(
        pv_rows(&h.notification().await),
        vec![(real(12.0), Some(time(3)))]
    );
    // Still B: another writer's later Value_Source report reports 3 again.
    h.set_clock(5);
    h.write_local(12.0).await;
    assert_eq!(
        pv_rows(&h.notification().await),
        vec![(real(12.0), Some(time(3)))]
    );
    // A again, at 7: its own commit time, not the first A's.
    h.set_clock(7);
    h.write_pv(0.0, 8).await;
    assert_eq!(
        pv_rows(&h.notification().await),
        vec![(real(0.0), Some(time(7)))]
    );
    h.server.stop().await.unwrap();
}
