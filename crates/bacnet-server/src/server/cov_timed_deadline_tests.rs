//! Max_Notification_Delay bounds how long timestamped changes stay queued
//! (135-2020 §13.1, §13.16.1.1.4; #856, part 2).
//!
//! Changes are reported as soon as they happen. These tests cover changes
//! that are still queued afterwards, because a send failed, a confirmed report
//! went unanswered or DISABLE_INITIATION held them, and nothing else changes:
//! they go out once the delay, measured from the earliest of them, has passed.
//! Time is paused, so Tokio jumps to each deadline while the test waits.
use super::cov_wire_test_support::*;
use super::*;
use tokio::time::Instant as TokioInstant;

const DELAY: u32 = 10;

/// Wait until just before `DELAY` seconds after `since`; nothing goes out yet.
async fn nothing_before_the_deadline(h: &Harness, since: TokioInstant) {
    tokio::time::sleep_until(
        since + Duration::from_secs(u64::from(DELAY)) - Duration::from_millis(100),
    )
    .await;
    h.no_notification().await;
}

/// The next notification, and that it went out at the deadline.
async fn notification_at_the_deadline(
    h: &Harness,
    since: TokioInstant,
) -> bacnet_services::cov_multiple::COVNotificationMultipleRequest {
    let report = h.notification().await;
    let elapsed = since.elapsed();
    let delay = Duration::from_secs(u64::from(DELAY));
    assert!(
        elapsed >= delay && elapsed < delay + Duration::from_secs(1),
        "sent {elapsed:?} after the earliest change, not at the {delay:?} deadline"
    );
    report
}

#[tokio::test(start_paused = true)]
async fn failed_sends_are_retried_at_the_delay_from_the_earliest_change() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.subscribe_with_delay(false, vec![(av1(), vec![(PV, true)])], DELAY)
        .await;
    h.notification().await;
    h.fail_notifications.store(true, Ordering::Release);
    h.set_clock(21);
    let earliest = TokioInstant::now();
    h.write_local(5.0).await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    h.set_clock(25);
    h.write_local(6.0).await;
    h.fail_notifications.store(false, Ordering::Release);
    nothing_before_the_deadline(&h, earliest).await;
    let report = notification_at_the_deadline(&h, earliest).await;
    assert_eq!(
        pv_rows(&report),
        vec![(real(5.0), Some(time(21))), (real(6.0), Some(time(25)))]
    );
    // Delivered: nothing is left for a later deadline.
    tokio::time::sleep(Duration::from_secs(u64::from(DELAY) * 3)).await;
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn changes_held_by_disable_initiation_go_out_at_the_deadline() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.subscribe_with_delay(false, vec![(av1(), vec![(PV, true)])], DELAY)
        .await;
    h.notification().await;
    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    h.set_clock(11);
    let earliest = TokioInstant::now();
    h.write_local(10.0).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    h.server.comm_state.set_for_test(DccState::Enable);
    nothing_before_the_deadline(&h, earliest).await;
    let report = notification_at_the_deadline(&h, earliest).await;
    assert_eq!(pv_rows(&report), vec![(real(10.0), Some(time(11)))]);
    h.server.stop().await.unwrap();
}

/// Hold-off after a failed confirmed report with a 10 ms retry timeout.
const HOLD_OFF: Duration = Duration::from_millis(10 * (DEFAULT_APDU_RETRIES as u64 + 1));

#[tokio::test(start_paused = true)]
async fn an_unacknowledged_confirmed_report_is_resent_at_the_deadline() {
    let mut h = Harness::start(ServerConfig {
        cov_retry_timeout_ms: 10,
        ..ServerConfig::default()
    })
    .await;
    h.subscribe_with_delay(true, vec![(av1(), vec![(PV, true)])], DELAY)
        .await;
    h.notification().await;
    h.ack().await;
    h.settle().await;
    h.set_clock(45);
    let earliest = TokioInstant::now();
    h.write_local(5.0).await;
    h.notification().await;
    // Never acknowledged: the retries run out and the context holds off.
    h.workers_idle().await;
    tokio::time::sleep(HOLD_OFF).await;
    nothing_before_the_deadline(&h, earliest).await;
    let report = notification_at_the_deadline(&h, earliest).await;
    assert_eq!(pv_rows(&report), vec![(real(5.0), Some(time(45)))]);
    h.ack().await;
    h.settle().await;
    tokio::time::sleep(Duration::from_secs(u64::from(DELAY) * 3)).await;
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_delivered_change_owes_no_deadline_notification() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.subscribe_with_delay(false, vec![(av1(), vec![(PV, true)])], DELAY)
        .await;
    h.notification().await;
    h.set_clock(30);
    h.write_local(3.0).await;
    assert_eq!(
        pv_rows(&h.notification().await),
        vec![(real(3.0), Some(time(30)))]
    );
    tokio::time::sleep(Duration::from_secs(u64::from(DELAY) * 3)).await;
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

fn av1_timed() -> Vec<(ObjectIdentifier, Vec<(PropertyIdentifier, bool)>)> {
    vec![(av1(), vec![(PV, true)])]
}

/// Timestamped histories and backstop waits the server still holds.
async fn held(h: &Harness) -> (usize, usize) {
    h.server.cov_table.read().await.timed().lock().held()
}

/// Assert that a notification taken now went out less than a second after
/// `since`.
fn promptly(since: TokioInstant, what: &str) {
    let elapsed = since.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "{what}: sent {elapsed:?} later"
    );
}

#[tokio::test(start_paused = true)]
async fn cancelling_with_a_deadline_pending_sends_nothing_and_holds_nothing() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.subscribe_with_delay(false, av1_timed(), DELAY).await;
    h.notification().await;
    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    h.set_clock(12);
    h.write_local(10.0).await;
    h.cancel_specs(false, av1_timed()).await;
    h.server.comm_state.set_for_test(DccState::Enable);
    tokio::time::sleep(Duration::from_secs(u64::from(DELAY) * 3)).await;
    h.no_notification().await;
    assert_eq!(held(&h).await, (0, 0));
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn expiry_with_a_deadline_pending_sends_nothing_and_holds_nothing() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.subscribe_with_delay(false, av1_timed(), DELAY).await;
    h.notification().await;
    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    h.set_clock(13);
    h.write_local(10.0).await;
    h.server.cov_table.write().await.expire_all_for_test();
    h.server.comm_state.set_for_test(DccState::Enable);
    tokio::time::sleep(Duration::from_secs(u64::from(DELAY) * 3)).await;
    h.no_notification().await;
    h.server.cov_table.write().await.purge_expired();
    assert_eq!(held(&h).await, (0, 0));
    h.server.stop().await.unwrap();
}

fn permissive_dcc() -> ServerConfig {
    ServerConfig {
        dcc_policy: crate::server::DccPolicy::LegacyPermissive,
        ..ServerConfig::default()
    }
}

#[tokio::test(start_paused = true)]
async fn enabling_communication_after_the_deadline_sends_at_once() {
    use bacnet_types::enums::EnableDisable;
    let mut h = Harness::start(permissive_dcc()).await;
    h.subscribe_with_delay(false, av1_timed(), DELAY).await;
    h.notification().await;
    h.dcc(EnableDisable::DISABLE_INITIATION, None).await;
    h.settle().await;
    h.set_clock(14);
    let earliest = TokioInstant::now();
    h.write_local(10.0).await;
    // The deadline passes while initiation is disabled; that attempt is
    // blocked and the next would wait another delay.
    tokio::time::sleep_until(earliest + Duration::from_secs(u64::from(DELAY) + 2)).await;
    h.no_notification().await;
    let enabled = TokioInstant::now();
    h.dcc(EnableDisable::ENABLE, None).await;
    let report = h.notification().await;
    promptly(enabled, "after ENABLE");
    assert_eq!(pv_rows(&report), vec![(real(10.0), Some(time(14)))]);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn the_dcc_timer_reenabling_communication_sends_at_once() {
    use bacnet_types::enums::EnableDisable;
    const SLOW: u32 = 25;
    let mut h = Harness::start(permissive_dcc()).await;
    h.subscribe_with_delay(false, av1_timed(), SLOW).await;
    h.notification().await;
    let disabled = TokioInstant::now();
    h.dcc(EnableDisable::DISABLE_INITIATION, Some(1)).await;
    h.settle().await;
    h.set_clock(15);
    h.write_local(10.0).await;
    // Blocked attempts at 25 s and 50 s; the next would come at 75 s.
    let expiry = disabled + Duration::from_secs(60);
    tokio::time::sleep_until(expiry - Duration::from_millis(100)).await;
    h.no_notification().await;
    let report = h.notification().await;
    let elapsed = disabled.elapsed();
    assert!(
        elapsed >= Duration::from_secs(60) && elapsed < Duration::from_secs(61),
        "sent {elapsed:?} after DISABLE_INITIATION for one minute"
    );
    assert_eq!(pv_rows(&report), vec![(real(10.0), Some(time(15)))]);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn reinitialize_restart_resumes_overdue_cov_without_another_write() {
    use bacnet_services::device_mgmt::ReinitializeDeviceRequest;
    use bacnet_types::enums::{EnableDisable, ReinitializedState};
    for state in [ReinitializedState::WARMSTART, ReinitializedState::COLDSTART] {
        let mut config = permissive_dcc();
        config.on_reinitialize = Some(Arc::new(|_, _| Ok(())));
        let mut h = Harness::start(config).await;
        h.subscribe_with_delay(false, av1_timed(), DELAY).await;
        h.notification().await;
        h.dcc(EnableDisable::DISABLE_INITIATION, None).await;
        h.settle().await;
        h.set_clock(14);
        let earliest = TokioInstant::now();
        h.write_local(10.0).await;
        tokio::time::sleep_until(earliest + Duration::from_secs(u64::from(DELAY) + 2)).await;
        h.no_notification().await;
        let mut data = BytesMut::new();
        ReinitializeDeviceRequest {
            reinitialized_state: state,
            password: None,
        }
        .encode(&mut data)
        .unwrap();
        let accepted = TokioInstant::now();
        h.request(ConfirmedServiceChoice::REINITIALIZE_DEVICE, data)
            .await;
        let report = h.notification().await;
        promptly(accepted, "after accepted restart");
        assert_eq!(pv_rows(&report), vec![(real(10.0), Some(time(14)))]);
        h.server.stop().await.unwrap();
    }
}

/// A confirmed report that DISABLE_INITIATION ends at its first retry (#1327)
/// returns its history to the queue with no hold-off: once communication is
/// enabled again the overdue change goes out at once, its time kept.
#[tokio::test(start_paused = true)]
async fn history_of_a_report_dcc_ends_at_a_retry_goes_out_once_enabled() {
    use bacnet_types::enums::EnableDisable;
    let mut h = Harness::start(permissive_dcc()).await;
    h.subscribe_with_delay(true, av1_timed(), DELAY).await;
    h.notification().await;
    h.ack().await;
    h.settle().await;
    h.set_clock(19);
    let changed = TokioInstant::now();
    h.write_local(1.0).await;
    h.notification().await;
    let (invoke_id, _) = h.take_confirmed();
    h.dcc(EnableDisable::DISABLE_INITIATION, None).await;
    h.workers_idle().await;
    let ended = changed.elapsed();
    assert!(
        (Duration::from_millis(2_900)..Duration::from_millis(3_050)).contains(&ended),
        "lease freed {ended:?} after the report"
    );
    // The deadline passes while initiation is disabled; nothing goes out,
    // not even a retry of the withdrawn report.
    tokio::time::sleep_until(changed + Duration::from_secs(u64::from(DELAY) + 2)).await;
    h.no_notification().await;
    assert!(!h.frames.lock().unwrap().iter().any(
        |apdu| matches!(apdu, Apdu::ConfirmedRequest(request) if request.invoke_id == invoke_id)
    ));
    let enabled = TokioInstant::now();
    h.dcc(EnableDisable::ENABLE, None).await;
    let report = h.notification().await;
    promptly(enabled, "after ENABLE");
    assert_eq!(pv_rows(&report), vec![(real(1.0), Some(time(19)))]);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_report_outstanding_at_the_deadline_leaves_the_follow_up_to_its_ack() {
    let mut h = Harness::start(ServerConfig {
        cov_retry_timeout_ms: 30_000,
        ..ServerConfig::default()
    })
    .await;
    h.subscribe_with_delay(true, av1_timed(), DELAY).await;
    h.notification().await;
    h.ack().await;
    h.settle().await;
    h.set_clock(16);
    h.write_local(1.0).await;
    h.notification().await; // outstanding, not yet acknowledged
    tokio::time::sleep(Duration::from_secs(1)).await;
    h.set_clock(17);
    let held_change = TokioInstant::now();
    h.write_local(2.0).await;
    // Its deadline passes while the first report is still outstanding.
    tokio::time::sleep_until(held_change + Duration::from_secs(u64::from(DELAY) + 4)).await;
    h.no_notification().await;
    let acked = TokioInstant::now();
    h.ack().await;
    let report = h.notification().await;
    promptly(acked, "after the Ack");
    assert_eq!(pv_rows(&report), vec![(real(2.0), Some(time(17)))]);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_confirmed_hold_off_moves_the_retry_to_its_end() {
    const SLOW: u32 = 30;
    let mut h = Harness::start(ServerConfig {
        cov_retry_timeout_ms: 5_000,
        ..ServerConfig::default()
    })
    .await;
    h.subscribe_with_delay(true, av1_timed(), SLOW).await;
    h.notification().await;
    h.ack().await;
    h.settle().await;
    h.set_clock(18);
    let earliest = TokioInstant::now();
    h.write_local(1.0).await;
    h.notification().await;
    // Four unanswered attempts end at 20 s, and the hold-off lasts another
    // 20 s. The deadline (30 s) falls inside it, so the retry waits for its
    // end, not for another delay.
    let hold_off = Duration::from_millis(5_000 * (DEFAULT_APDU_RETRIES as u64 + 1));
    tokio::time::sleep_until(earliest + hold_off * 2 - Duration::from_millis(100)).await;
    h.no_notification().await;
    let report = h.notification().await;
    let elapsed = earliest.elapsed();
    assert!(
        elapsed >= hold_off * 2 && elapsed < hold_off * 2 + Duration::from_secs(1),
        "resent {elapsed:?} after the change, not when the hold-off ended"
    );
    assert_eq!(pv_rows(&report), vec![(real(1.0), Some(time(18)))]);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_failing_context_is_retried_once_per_delay() {
    const FAST: u32 = 2;
    let mut h = Harness::start(ServerConfig::default()).await;
    h.subscribe_with_delay(false, av1_timed(), FAST).await;
    h.notification().await;
    let attempts = |h: &Harness| h.server.cov_counters().notifications_sent;
    let before = attempts(&h);
    h.fail_notifications.store(true, Ordering::Release);
    h.set_clock(19);
    let earliest = TokioInstant::now();
    h.write_local(5.0).await;
    for (at_ms, expected) in [(1_900, 1), (2_100, 2), (3_900, 2), (4_100, 3), (6_100, 4)] {
        tokio::time::sleep_until(earliest + Duration::from_millis(at_ms)).await;
        assert_eq!(attempts(&h) - before, expected, "attempts by {at_ms} ms");
    }
    h.fail_notifications.store(false, Ordering::Release);
    let report = h.notification().await;
    assert_eq!(attempts(&h) - before, 5);
    assert_eq!(pv_rows(&report), vec![(real(5.0), Some(time(19)))]);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_renewal_with_a_shorter_delay_brings_the_deadline_forward() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.subscribe_with_delay(false, av1_timed(), 30).await;
    h.notification().await;
    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    h.set_clock(20);
    let earliest = TokioInstant::now();
    h.write_local(10.0).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    h.server.comm_state.set_for_test(DccState::Enable);
    // A lifetime renewal listing no references changes the context's delay.
    h.subscribe_with_delay(false, Vec::new(), 5).await;
    let report = h.notification().await;
    let elapsed = earliest.elapsed();
    assert!(
        elapsed >= Duration::from_secs(5) && elapsed < Duration::from_secs(6),
        "sent {elapsed:?} after the change, not at the renewed 5 s deadline"
    );
    assert_eq!(pv_rows(&report), vec![(real(10.0), Some(time(20)))]);
    h.server.stop().await.unwrap();
}
