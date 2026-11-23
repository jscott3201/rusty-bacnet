//! The other confirmed forms refused an invoke ID (#2661): SubscribeCOVProperty,
//! and a COV-multiple context with untimestamped and timestamped references.
//! A timestamped context's history goes back to its references when its
//! report is refused, and the report that goes once an ID comes back
//! carries every row once, in capture order.
use super::cov_owed_id_tests::{lease_all, owed, owed_counts, pv_of, refusals};
use super::cov_wire_test_support::*;
use super::*;
use bacnet_services::cov::SubscribeCOVPropertyRequest;

/// A confirmed SubscribeCOVProperty on AV-1's Present_Value is reported
/// once an ID comes back.
#[tokio::test(start_paused = true)]
async fn a_property_subscription_refused_an_id_goes_once_one_comes_back() {
    let mut h = Harness::start(ServerConfig::default()).await;
    let mut body = BytesMut::new();
    SubscribeCOVPropertyRequest {
        subscriber_process_identifier: 2661,
        monitored_object_identifier: av1(),
        issue_confirmed_notifications: Some(true),
        lifetime: Some(300),
        monitored_property_identifier: PV,
        monitored_property_array_index: None,
        cov_increment: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY, body)
        .await;
    response(&h).await.expect("subscribed");
    h.cov_notification().await;
    h.ack().await;
    h.settle().await;

    let mut held = lease_all(&h);
    h.write_local(42.0).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    h.no_notification().await;
    drop(held.pop());
    assert_eq!(pv_of(&h.cov_notification().await), real(42.0));
    h.ack().await;
    h.settle().await;
    h.no_notification().await;
    assert_eq!(owed_counts(&h), (1, 1, 0));
}

/// An untimestamped confirmed COV-multiple reference: the context's report
/// goes once an ID comes back, with the latest value.
#[tokio::test(start_paused = true)]
async fn an_untimestamped_context_refused_an_id_goes_once_one_comes_back() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.subscribe_specs(true, vec![(av1(), vec![(PV, false)])])
        .await;
    h.notification().await;
    h.ack().await;
    h.settle().await;

    let mut held = lease_all(&h);
    h.write_local(42.0).await;
    h.write_local(43.0).await;
    assert_eq!(owed(&h).await.len(), 1);
    tokio::time::sleep(Duration::from_secs(60)).await;
    h.no_notification().await;
    drop(held.pop());
    assert_eq!(pv_rows(&h.notification().await), vec![(real(43.0), None)]);
    h.ack().await;
    h.settle().await;
    h.no_notification().await;
}

/// A timestamped context refused an ID keeps its rows: the backstop finds
/// no ID either, and each refusal gives the history back. Once one comes
/// back the report carries every row once, in capture order, and nothing
/// follows its Ack.
#[tokio::test(start_paused = true)]
async fn a_timestamped_context_refused_an_id_keeps_its_rows() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.subscribe_with_delay(true, vec![(av1(), vec![(PV, true)])], 2)
        .await;
    h.notification().await;
    h.ack().await;
    h.settle().await;

    let mut held = lease_all(&h);
    h.set_clock(10);
    h.write_local(1.0).await;
    h.set_clock(11);
    h.write_local(2.0).await;
    tokio::time::sleep(Duration::from_secs(20)).await;
    h.no_notification().await;
    // The backstop tries at its spacing, not in a spin.
    assert!(refusals(&h) <= 12, "{} refusals", refusals(&h));
    assert_eq!(owed(&h).await.len(), 1);

    drop(held.pop());
    let report = h.notification().await;
    assert_eq!(
        pv_rows(&report),
        vec![(real(1.0), Some(time(10))), (real(2.0), Some(time(11)))]
    );
    h.ack().await;
    h.settle().await;
    tokio::time::sleep(Duration::from_secs(30)).await;
    h.no_notification().await;
}
