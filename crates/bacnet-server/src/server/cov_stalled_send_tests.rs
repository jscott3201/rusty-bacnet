//! A confirmed COV report whose send stalls past its flight's deadline
//! (#2303), as a BACnet/SC send waiting on a hub or a direct dial can.
//!
//! The worker stops half a timeout before the table's deadline and reports
//! a failure, so its coordinate holds off: no second report starts while the
//! stalled one could still send, and nothing the stalled send carried lands
//! after a newer value. A worker that reports nothing at all, as an aborted
//! one, is fenced by the next report past its deadline. An unconfirmed
//! send stalling ahead of a confirmed report in one fanout leaves the
//! report its window. Time is paused.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_services::cov::SubscribeCOVRequest;

/// One attempt's timeout: the retry cycle is four of them, 400 ms, the
/// worker's cutoff 450 ms, the deadline 500 ms and the hold-off after a
/// failure 400 ms.
const TIMEOUT_MS: u64 = 100;

/// Present_Value of each ConfirmedCOVNotification the subscriber received,
/// in order.
fn received(h: &Harness) -> Vec<f32> {
    h.frames
        .lock()
        .unwrap()
        .iter()
        .filter_map(|apdu| match apdu {
            Apdu::ConfirmedRequest(request)
                if request.service_choice == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION =>
            {
                let report = COVNotificationRequest::decode(&request.service_request).unwrap();
                let value = report
                    .list_of_values
                    .iter()
                    .find(|value| value.property_identifier == PV)?;
                let (PropertyValue::Real(pv), _) =
                    bacnet_encoding::primitives::decode_application_value(&value.value, 0).ok()?
                else {
                    return None;
                };
                Some(pv)
            }
            _ => None,
        })
        .collect()
}

/// Queue AV-1's subscription for a fresh look, as an Ack's follow-up or the
/// timestamped backstop does: the path that takes no expiry pass first.
async fn revisit(h: &Harness) {
    let table = h.server.cov_table.read().await;
    let keys: Vec<_> = table.entries().map(|entry| entry.key().clone()).collect();
    table.revisits().request(keys);
}

/// A harness with AV-1 subscribed for confirmed notifications, its initial
/// report acknowledged and the log cleared.
async fn subscribed() -> Harness {
    let mut h = Harness::start(ServerConfig {
        cov_retry_timeout_ms: TIMEOUT_MS,
        ..ServerConfig::default()
    })
    .await;
    let mut body = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: 2303,
        monitored_object_identifier: av1(),
        issue_confirmed_notifications: Some(true),
        lifetime: Some(300),
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV, body).await;
    h.cov_notification().await;
    h.ack().await;
    h.settle().await;
    h.frames.lock().unwrap().clear();
    h
}

/// AV-1's Present_Value set to `value` without a fanout.
async fn set_quietly(h: &Harness, value: f32) {
    h.server
        .database()
        .write()
        .await
        .get_mut(&av1())
        .unwrap()
        .write_property_from(
            PV,
            None,
            PropertyValue::Real(value),
            Some(8),
            &crate::command_source::test_origin(),
        )
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_stalled_report_ends_before_its_deadline_and_no_stale_value_follows_a_newer_one() {
    let mut h = subscribed().await;

    // The report of 10 stalls in its first send.
    let (entered, release) = h.hold_notification_barrier(true);
    h.write_local(10.0).await;
    tokio::time::timeout(Duration::from_secs(1), entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    // Past the deadline. The worker was cut off before it and failed.
    tokio::time::sleep(Duration::from_millis(5 * TIMEOUT_MS + 100)).await;

    // A newer value, looked at again without an expiry pass, inside the
    // hold-off: the coordinate stays held off, so no second report starts.
    set_quietly(&h, 20.0).await;
    revisit(&h).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        h.server.notification_transactions.active_count() <= 1,
        "one report at a time"
    );
    assert_eq!(
        received(&h),
        Vec::<f32>::new(),
        "held off after the failure"
    );

    // The stalled send was cut off with its report: releasing it sends
    // nothing.
    release.add_permits(1);
    tokio::time::sleep(Duration::from_millis(2 * TIMEOUT_MS)).await;
    assert_eq!(received(&h), Vec::<f32>::new(), "nothing stale lands");

    // After the hold-off the newer value goes out, alone.
    tokio::time::sleep(Duration::from_millis(4 * TIMEOUT_MS)).await;
    revisit(&h).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let values = received(&h);
    assert_eq!(values.first(), Some(&20.0), "{values:?}");
    assert!(values.iter().all(|value| *value == 20.0), "{values:?}");
    assert!(h.server.notification_transactions.active_count() <= 1);
    h.server.stop().await.unwrap();
}

/// A worker that reports nothing (aborted; the test begins the report
/// itself and runs no worker) leaves its report marked past the deadline.
/// The next report the server begins there fences it and wakes the worker:
/// the server's table wakes fenced workers as soon as a report begins, on
/// the revisit path too, which takes no expiry pass.
#[tokio::test(start_paused = true)]
async fn the_next_report_past_the_deadline_wakes_a_worker_that_never_reported() {
    let mut h = subscribed().await;
    let abandoned = {
        let mut table = h.server.cov_table.write().await;
        let sub = table.entries().next().unwrap().clone();
        let report = table.prepare_completion(&sub).unwrap();
        table
            .begin_confirmed(report, [&sub].map(|sub| (sub, placeholder())))
            .unwrap()
    };
    // Before the deadline the coordinate is busy: a newer value waits.
    set_quietly(&h, 20.0).await;
    revisit(&h).await;
    tokio::time::sleep(Duration::from_millis(4 * TIMEOUT_MS)).await;
    assert_eq!(received(&h), Vec::<f32>::new());
    // Past it, the next report goes out and wakes the abandoned worker.
    tokio::time::sleep(Duration::from_millis(2 * TIMEOUT_MS)).await;
    revisit(&h).await;
    tokio::time::timeout(Duration::from_millis(TIMEOUT_MS), abandoned.fenced())
        .await
        .expect("the next report fences the abandoned one");
    assert_eq!(received(&h).first(), Some(&20.0));
    h.server.stop().await.unwrap();
}

/// An observation for a report these tests never acknowledge.
fn placeholder() -> bacnet_device_core::cov::CovObservation {
    bacnet_device_core::cov::CovObservation::new(
        bacnet_device_core::cov::CovSample::new(&bacnet_types::primitives::PropertyValue::Real(
            0.0,
        ))
        .unwrap(),
        None,
    )
    .unwrap()
}

/// An unconfirmed send that stalls ahead of a confirmed report in the same
/// fanout leaves that report its whole answer window (#2303): the report's
/// worker starts before the stalled send, so the report goes out while the
/// send waits and is acknowledged in its window, and the stalled send lands
/// after. A report sent only once the stall ended would have lost the time
/// its flight had run since evaluation.
#[tokio::test(start_paused = true)]
async fn an_unconfirmed_stall_ahead_keeps_a_confirmed_window() {
    let mut h = Harness::start(ServerConfig {
        cov_retry_timeout_ms: TIMEOUT_MS,
        ..ServerConfig::default()
    })
    .await;
    for (process, confirmed) in [(1u32, false), (2u32, true)] {
        let mut body = BytesMut::new();
        SubscribeCOVRequest {
            subscriber_process_identifier: process,
            monitored_object_identifier: av1(),
            issue_confirmed_notifications: Some(confirmed),
            lifetime: Some(300),
        }
        .encode(&mut body)
        .unwrap();
        h.request(ConfirmedServiceChoice::SUBSCRIBE_COV, body).await;
        h.cov_notification().await;
        if confirmed {
            h.ack().await;
        }
        h.settle().await;
    }
    // The first COV notification on the link, process 1's unconfirmed one,
    // is held there.
    let (entered, release) = h.hold_notification_barrier(true);
    h.write_pv(10.0, 1).await;
    tokio::time::timeout(Duration::from_secs(1), entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let confirmed = tokio::time::timeout(Duration::from_millis(20), h.cov_notification())
        .await
        .expect("the confirmed report goes out while the send stalls");
    assert_eq!(confirmed.subscriber_process_identifier, 2);
    h.ack().await;
    // The stall outlasts the report's whole retry cycle.
    tokio::time::sleep(Duration::from_millis(5 * TIMEOUT_MS)).await;
    release.add_permits(1);
    let unconfirmed = h.cov_notification().await;
    assert_eq!(unconfirmed.subscriber_process_identifier, 1);
    let baseline = {
        let table = h.server.cov_table.read().await;
        table
            .entries()
            .find(|entry| entry.subscriber_process_identifier == 2)
            .and_then(|entry| entry.last_notified_observation.clone())
            .map(|observation| observation.sample().value().clone())
    };
    assert_eq!(
        baseline,
        Some(PropertyValue::Real(10.0)),
        "the confirmed report was acknowledged in its window"
    );
    h.server.stop().await.unwrap();
}
