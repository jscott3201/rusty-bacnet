//! A confirmed COV report whose send stalls past its flight's deadline
//! (#2303), as a BACnet/SC send waiting on a hub or a direct dial can.
//!
//! The worker stops at its retry budget, one timeout before the table's
//! deadline, and reports a failure, so its coordinate holds off: no second
//! report starts while the stalled one could still send, and nothing the
//! stalled send carried lands after a newer value. Time is paused.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_services::cov::SubscribeCOVRequest;

/// One attempt's timeout: the budget is four of them, 400 ms, the deadline
/// 500 ms and the hold-off after a failure 400 ms.
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

#[tokio::test(start_paused = true)]
async fn a_stalled_report_ends_at_its_budget_and_no_stale_value_follows_a_newer_one() {
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

    // The report of 10 stalls in its first send.
    let (entered, release) = h.hold_notification_barrier(true);
    h.write_local(10.0).await;
    tokio::time::timeout(Duration::from_secs(1), entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    // Past the deadline. The worker stopped at its budget and failed.
    tokio::time::sleep(Duration::from_millis(5 * TIMEOUT_MS + 100)).await;

    // A newer value, looked at again without an expiry pass, inside the
    // hold-off: the coordinate stays held off, so no second report starts.
    h.server
        .database()
        .write()
        .await
        .get_mut(&av1())
        .unwrap()
        .write_property_from(
            PV,
            None,
            PropertyValue::Real(20.0),
            Some(8),
            &crate::command_source::test_origin(),
        )
        .unwrap();
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
