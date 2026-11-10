//! The hold-off after a failed confirmed COV-multiple report follows the
//! table's decision (#2419).
//!
//! A failure the table counted holds the context's Max_Notification_Delay
//! backstop (Clause 13.16.1.1.4) off until the hold-off ends, no shorter and
//! no longer. A failure the table ignored, because the report's
//! subscription was replaced or cancelled meanwhile, leaves the backstop of
//! the subscription made since where it was.
//!
//! The context watches AV-1's timestamped Present_Value under the default
//! timing: 3 s per attempt and three retries, so a report nobody answers
//! fails 12 s after it began and a hold-off lasts 12 s. Time is paused.
use super::*;
use bacnet_encoding::apdu::ErrorPdu;
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;

/// Max_Notification_Delay of the contexts here, shorter than a hold-off.
const DELAY: Duration = Duration::from_secs(5);
/// One full retry cycle under the default timing.
const HOLD_OFF: Duration = Duration::from_secs(12);

/// Subscribe the context, or list its reference again, with `delay`.
async fn subscribe(h: &mut Harness, delay: Duration) {
    h.subscribe_with_delay(true, av1_timed(), delay.as_secs() as u32)
        .await;
}

/// Answer one confirmed notification with an Error.
async fn refuse(h: &Harness, (invoke_id, service_choice): (u8, ConfirmedServiceChoice)) {
    h.respond(Apdu::Error(ErrorPdu {
        invoke_id,
        service_choice,
        error_class: bacnet_types::enums::ErrorClass::SERVICES,
        error_code: ErrorCode::OTHER,
        error_data: Bytes::new(),
    }))
    .await;
}

/// The next new notification and how long after `since` it went out. It
/// waits well past a hold-off, so a late report says how late it was.
async fn next_report(
    h: &Harness,
    since: TokioInstant,
) -> (Duration, COVNotificationMultipleRequest) {
    // `notification` gives up after 5 s.
    for _ in 0..16 {
        let waited = tokio::time::timeout(Duration::from_secs(4), h.notification()).await;
        if let Ok(report) = waited {
            return (since.elapsed(), report);
        }
    }
    panic!("no COV-multiple notification {:?} on", since.elapsed());
}

/// The next new notification, which goes out `due` after `since`: not
/// before, and less than a second later.
async fn report_due(
    h: &Harness,
    since: TokioInstant,
    due: Duration,
) -> COVNotificationMultipleRequest {
    tokio::time::sleep_until(since + due - Duration::from_millis(100)).await;
    h.no_notification().await;
    let (elapsed, report) = next_report(h, since).await;
    assert!(
        elapsed >= due && elapsed < due + Duration::from_secs(1),
        "the report went out {elapsed:?} on; it was due {due:?} on"
    );
    report
}

/// Wait, on the paused clock, until `probe` holds.
async fn eventually(what: &str, probe: impl Fn() -> bool) {
    let probed = async {
        while !probe() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(1), probed)
        .await
        .unwrap_or_else(|_| panic!("{what}"));
}

/// Wait until the one confirmed worker has its report's outcome, so its
/// transaction is over, yet still holds its in-flight slot, which it frees
/// after ending the report in the table: it is waiting for the table.
async fn worker_waits_for_the_table(h: &Harness) {
    eventually("the old report's worker waits for the table", || {
        h.server.notification_transactions.active_count() == 0
            && h.server.cov_in_flight.available_permits() == 254
    })
    .await;
}

/// Wait until every confirmed worker has ended its report in the table and
/// freed its in-flight slot.
async fn workers_done(h: &Harness) {
    h.workers_idle().await;
    eventually("every worker freed its slot", || {
        h.server.cov_in_flight.available_permits() == 255
    })
    .await;
}

/// Hold initiation back without a DeviceCommunicationControl request, so
/// what is captured meanwhile waits for the backstop: lifting it this way
/// doesn't send overdue changes at once, as a request's ENABLE does.
fn hold_back(h: &Harness, held: bool) {
    h.server.comm_state.set_for_test(if held {
        DccState::DisableInitiation
    } else {
        DccState::Enable
    });
}

/// The subscription is cancelled and made again while its report is out;
/// the cancelled subscription's report runs on, and is then refused.
#[tokio::test(start_paused = true)]
async fn a_cancelled_subscriptions_refused_report_leaves_the_new_backstop_alone() {
    let mut h = Harness::start(ServerConfig::default()).await;
    subscribe(&mut h, DELAY).await;
    h.notification().await;
    let old = h.take_confirmed();
    h.cancel_specs(true, Vec::new()).await;
    h.settle().await;
    subscribe(&mut h, DELAY).await;
    h.notification().await;
    h.ack().await;
    h.settle().await;

    // A change of the new subscription that waits for the backstop.
    hold_back(&h, true);
    h.set_clock(20);
    let changed = TokioInstant::now();
    h.write_local(5.0).await;
    // The table knows no report of the cancelled subscription.
    refuse(&h, old).await;
    workers_done(&h).await;
    hold_back(&h, false);
    let report = report_due(&h, changed, DELAY).await;
    assert_eq!(pv_rows(&report), vec![(real(5.0), Some(time(20)))]);
    h.server.stop().await.unwrap();
}

/// The same, with the cancelled subscription's report never answered: its
/// retries run out 12 s after it began.
#[tokio::test(start_paused = true)]
async fn a_cancelled_subscriptions_unanswered_report_leaves_the_new_backstop_alone() {
    let mut h = Harness::start(ServerConfig::default()).await;
    let began = TokioInstant::now();
    subscribe(&mut h, DELAY).await;
    h.notification().await;
    h.cancel_specs(true, Vec::new()).await;
    h.settle().await;
    subscribe(&mut h, DELAY).await;
    h.notification().await;
    h.ack().await;
    h.settle().await;

    // After the old report's last retry, at 9 s, a change of the new
    // subscription that waits for the backstop.
    tokio::time::sleep_until(began + Duration::from_secs(10)).await;
    hold_back(&h, true);
    h.set_clock(30);
    let changed = TokioInstant::now();
    h.write_local(5.0).await;
    workers_done(&h).await;
    let failed = began.elapsed();
    assert!(
        failed >= HOLD_OFF && failed < HOLD_OFF + Duration::from_millis(100),
        "the old report failed {failed:?} on"
    );
    hold_back(&h, false);
    let report = report_due(&h, changed, DELAY).await;
    assert_eq!(pv_rows(&report), vec![(real(5.0), Some(time(30)))]);
    h.server.stop().await.unwrap();
}

/// The reference is listed again while its report is out, which fences the
/// report, at the moment its worker has the subscriber's refusal and waits
/// for the table. The worker then finds its report fenced.
#[tokio::test(start_paused = true)]
async fn a_replaced_subscriptions_refused_report_leaves_the_new_backstop_alone() {
    let mut h = Harness::start(ServerConfig::default()).await;
    let began = TokioInstant::now();
    subscribe(&mut h, DELAY).await;
    h.notification().await;
    let old = h.take_confirmed();
    // The new subscription's initial report waits for the backstop.
    hold_back(&h, true);

    // Behind this guard the re-subscription queues for the table first and
    // the old report's worker, with its refusal, second. A wait on the
    // paused clock lets every task run until it blocks, so the
    // re-subscription is queued before the refusal arrives.
    let cov_table = Arc::clone(&h.server.cov_table);
    let table = cov_table.write().await;
    h.set_clock(7);
    subscribe(&mut h, DELAY).await;
    h.settle().await;
    refuse(&h, old).await;
    worker_waits_for_the_table(&h).await;
    drop(table);
    workers_done(&h).await;
    assert_eq!(response(&h).await, Ok(()), "the re-subscription");
    hold_back(&h, false);

    // The earliest pending change is the old report's, captured when the
    // context was first subscribed; the renewal's own capture follows it.
    let report = report_due(&h, began, DELAY).await;
    assert_eq!(
        pv_rows(&report),
        vec![(real(0.0), Some(time(0))), (real(0.0), Some(time(7)))]
    );
    h.server.stop().await.unwrap();
}

/// The same fence, at the moment the old report's retries run out.
#[tokio::test(start_paused = true)]
async fn a_replaced_subscriptions_unanswered_report_leaves_the_new_backstop_alone() {
    let mut h = Harness::start(ServerConfig::default()).await;
    let began = TokioInstant::now();
    subscribe(&mut h, DELAY).await;
    h.notification().await;
    // After the last retry, at 9 s: the re-subscription queues for the
    // table behind this guard, and the worker joins it when its retries
    // run out at 12 s.
    tokio::time::sleep_until(began + HOLD_OFF - Duration::from_millis(100)).await;
    hold_back(&h, true);
    let cov_table = Arc::clone(&h.server.cov_table);
    let table = cov_table.write().await;
    h.set_clock(7);
    subscribe(&mut h, DELAY).await;
    h.settle().await;
    worker_waits_for_the_table(&h).await;
    let failed = began.elapsed();
    assert!(
        failed >= HOLD_OFF && failed < HOLD_OFF + Duration::from_millis(100),
        "the old report failed {failed:?} on"
    );
    drop(table);
    workers_done(&h).await;
    assert_eq!(response(&h).await, Ok(()), "the re-subscription");
    hold_back(&h, false);

    // The old report's change came back when its retries ran out, long
    // overdue, and that attempt was held back: the next is one delay on.
    let report = report_due(&h, began, HOLD_OFF + DELAY).await;
    assert_eq!(
        pv_rows(&report),
        vec![(real(0.0), Some(time(0))), (real(0.0), Some(time(7)))]
    );
    h.server.stop().await.unwrap();
}

/// Nothing held back: the new subscription's own report is refused, and
/// the cancelled subscription's report, still retrying, is refused 5 s
/// later. The follow-up waits out the new report's hold-off only.
#[tokio::test(start_paused = true)]
async fn a_cancelled_subscriptions_late_failure_adds_nothing_to_a_real_hold_off() {
    let mut h = Harness::start(ServerConfig::default()).await;
    subscribe(&mut h, DELAY).await;
    h.notification().await;
    let old = h.take_confirmed();
    h.cancel_specs(true, Vec::new()).await;
    h.settle().await;
    subscribe(&mut h, DELAY).await;
    h.notification().await;
    let failed = TokioInstant::now();
    h.reject().await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    refuse(&h, old).await;
    workers_done(&h).await;
    let report = report_due(&h, failed, HOLD_OFF).await;
    assert_eq!(pv_rows(&report), vec![(real(0.0), Some(time(0)))]);
    h.server.stop().await.unwrap();
}

/// A real failure, where only the worker's own step moves the backstop.
/// The backstop sent the report, in two parts to a small subscriber, so
/// the second part stayed pending and the backstop's next attempt would
/// wait another Max_Notification_Delay, here longer than the hold-off.
#[tokio::test(start_paused = true)]
async fn a_refused_backstop_report_goes_out_again_when_its_hold_off_ends() {
    const SLOW: Duration = Duration::from_secs(30);
    let mut h = Harness::start(ServerConfig::default()).await;
    h.request_max_apdu = 50;
    subscribe(&mut h, SLOW).await;
    while h.server.notification_transactions.active_count() > 0 {
        h.notification().await;
        h.ack().await;
        h.settle().await;
    }

    // Two changes that wait for the backstop, which sends the first part
    // of their report 30 s after the earlier one.
    hold_back(&h, true);
    h.set_clock(40);
    let changed = TokioInstant::now();
    h.write_local(5.0).await;
    h.set_clock(41);
    h.write_local(6.0).await;
    hold_back(&h, false);
    let first = report_due(&h, changed, SLOW).await;
    assert_eq!(pv_rows(&first), vec![(real(5.0), Some(time(40)))]);

    // Refused: the context holds off for 12 s, and the backstop tries again
    // when that ends, not a whole delay later.
    let failed = TokioInstant::now();
    h.reject().await;
    workers_done(&h).await;
    let again = report_due(&h, failed, HOLD_OFF).await;
    assert_eq!(pv_rows(&again), vec![(real(5.0), Some(time(40)))]);
    h.server.stop().await.unwrap();
}
