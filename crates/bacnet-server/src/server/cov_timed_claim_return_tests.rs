//! When a confirmed COV-multiple report's timestamped history comes back to
//! its references (#2320): at once when its worker ends the report without
//! an Ack, and at the report's own deadline when the worker can't.
//!
//! The worker holds the history as a claim lodged with the histories until
//! the instant the table gave the report. Time is paused, and the timeout is
//! long enough to tell the instants apart: the hold-off after a failure ends
//! four timeouts after it, the report's deadline five after it began.
use super::cov_wire_test_support::*;
use super::*;
use tokio::time::Instant as TokioInstant;

/// One attempt's timeout.
const TIMEOUT_MS: u64 = 1_000;
/// The context's Max_Notification_Delay, shorter than the hold-off and the
/// deadline, so neither test waits on it.
const DELAY: u32 = 2;
/// How late after its instant a report may be seen: the harness looks for
/// it once a millisecond.
const SLACK: Duration = Duration::from_millis(100);

/// A harness with AV-1's Present_Value subscribed for timestamped confirmed
/// reports, its initial report acknowledged.
async fn subscribed() -> Harness {
    let mut h = Harness::start(ServerConfig {
        cov_retry_timeout_ms: TIMEOUT_MS,
        ..ServerConfig::default()
    })
    .await;
    h.subscribe_with_delay(true, vec![(av1(), vec![(PV, true)])], DELAY)
        .await;
    h.notification().await;
    h.ack().await;
    h.settle().await;
    h
}

/// A rejected report's history is in the report sent when the hold-off
/// ends: its worker returned it at the rejection. Had it only come back at
/// the report's deadline, a timeout later, the hold-off's end would have
/// found nothing to send.
#[tokio::test(start_paused = true)]
async fn a_rejected_reports_history_goes_out_again_when_the_hold_off_ends() {
    let mut h = subscribed().await;
    h.set_clock(20);
    h.write_local(1.0).await;
    h.notification().await;
    let rejected = TokioInstant::now();
    h.reject().await;
    h.workers_idle().await;
    let hold_off = Duration::from_millis(TIMEOUT_MS * (u64::from(DEFAULT_APDU_RETRIES) + 1));
    tokio::time::sleep_until(rejected + hold_off - SLACK).await;
    h.no_notification().await;
    let report = h.notification().await;
    let elapsed = rejected.elapsed();
    assert!(
        elapsed >= hold_off && elapsed < hold_off + SLACK,
        "sent {elapsed:?} after the rejection, not when the {hold_off:?} hold-off ended"
    );
    assert_eq!(pv_rows(&report), vec![(real(1.0), Some(time(20)))]);
    h.ack().await;
    h.settle().await;
    tokio::time::sleep(Duration::from_secs(30)).await;
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

/// A worker that has its Ack but can't take the table to settle (this test
/// holds it across the deadline) leaves its claim lodged. The histories give
/// it back at the report's own deadline, the instant the table ends the
/// report, and the report that follows carries the history: not a moment
/// later, and the late Ack retires nothing.
#[tokio::test(start_paused = true)]
async fn a_claim_its_worker_cannot_settle_returns_at_the_reports_deadline() {
    let mut h = subscribed().await;
    h.set_clock(22);
    let begun = (TokioInstant::now(), crate::runtime_clock::core_now());
    h.write_local(1.0).await;
    h.notification().await;
    let deadline = Duration::from_millis(TIMEOUT_MS * (u64::from(DEFAULT_APDU_RETRIES) + 2));

    let table = h.server.cov_table.read().await;
    h.ack().await;
    tokio::time::sleep_until(begun.0 + deadline - Duration::from_millis(1)).await;
    // Nothing else is pending, so the store's one deadline is the claim's.
    let lodged = table.timed().lock().next_deadline();
    assert_eq!(
        lodged,
        Some(begun.1 + bacnet_time::Duration::saturating_from_core(deadline)),
        "the claim ends with its report, {deadline:?} after the table began it"
    );
    tokio::time::sleep_until(begun.0 + deadline).await;
    // The deadline wait is the server's task: let it take its turn, the
    // clock standing still.
    let mut turns = 0;
    while table.timed().lock().next_deadline() == lodged {
        turns += 1;
        assert!(
            turns < 1_000,
            "the claim is still lodged at its report's deadline"
        );
        tokio::task::yield_now().await;
    }
    drop(table);

    let report = h.notification().await;
    let elapsed = begun.0.elapsed();
    assert!(
        elapsed >= deadline && elapsed < deadline + SLACK,
        "sent {elapsed:?} after the report began, not at its {deadline:?} deadline"
    );
    assert_eq!(pv_rows(&report), vec![(real(1.0), Some(time(22)))]);
    h.ack().await;
    h.settle().await;
    tokio::time::sleep(Duration::from_secs(30)).await;
    h.no_notification().await;
    h.server.stop().await.unwrap();
}
