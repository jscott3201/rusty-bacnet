use super::*;
use bacnet_client::client::{ClientOptions, ConfirmedCOVNotificationResponse};
use bacnet_encoding::apdu::ConfirmedRequest;
use bacnet_services::{common::BACnetPropertyValue, cov::COVNotificationRequest};
use bacnet_types::{
    enums::{AbortReason, ConfirmedServiceChoice, ObjectType, PropertyIdentifier, RejectReason},
    primitives::ObjectIdentifier,
};
use bytes::{Bytes, BytesMut};

pub(super) fn cov(process: u32) -> Apdu {
    let mut payload = BytesMut::new();
    COVNotificationRequest {
        subscriber_process_identifier: process,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap(),
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        time_remaining: 60,
        list_of_values: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0x44, 0x42, 0x48, 0, 0],
            priority: None,
        }],
    }
    .encode(&mut payload)
    .unwrap();
    confirmed(
        ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
        process as u8,
        payload.freeze(),
    )
}
#[tokio::test]
async fn client_cov_ack_reject_silence_and_stale_application_delivery() {
    let ca = TestCa::new();
    let (mut f, port) = Fixture::new(&ca).await;
    let options = ClientOptions::default().with_confirmed_cov_notification_ack_policy(|n| match n
        .notification
        .subscriber_process_identifier
    {
        2 => ConfirmedCOVNotificationResponse::Reject(RejectReason::OTHER),
        3 => ConfirmedCOVNotificationResponse::NoResponse,
        _ => ConfirmedCOVNotificationResponse::Ack,
    });
    let mut client = BACnetClient::start_with_options(ClientConfig::default(), port, options)
        .await
        .unwrap();
    let mut notifications = client.cov_notifications();
    let mut a = f.peer(ca.tls("A"), None).await;
    for process in 1..=3 {
        let admitted = f.capture(&mut a, &cov(process), routed()).await;
        f.feed(admitted).await;
        let received = bounded(notifications.recv()).await.unwrap();
        assert_eq!(received.notification.subscriber_process_identifier, process);
        match process {
            1 => assert!(matches!(f.response(&a).await.1, Apdu::SimpleAck(r) if r.invoke_id == 1)),
            2 => assert!(
                matches!(f.response(&a).await.1, Apdu::Reject(r) if r.invoke_id == 2 && r.reject_reason == RejectReason::OTHER)
            ),
            _ => f.barrier(&mut a, 60).await,
        }
    }
    let held = f.capture(&mut a, &cov(4), None).await;
    let mut b = f.peer(ca.tls("B"), None).await;
    // Capture B first to prove replacement commit, then release already-admitted A.
    let fresh = f.capture(&mut b, &cov(5), None).await;
    f.feed(held).await;
    f.feed(fresh).await;
    assert_eq!(
        bounded(notifications.recv())
            .await
            .unwrap()
            .notification
            .subscriber_process_identifier,
        4
    );
    assert_eq!(
        bounded(notifications.recv())
            .await
            .unwrap()
            .notification
            .subscriber_process_identifier,
        5
    );
    assert!(matches!(f.response(&b).await.1, Apdu::SimpleAck(r) if r.invoke_id == 5));
    f.barrier(&mut b, 61).await;
    client.stop().await.unwrap();
    f.listener.stop().await;
}

#[tokio::test]
async fn client_event_ack_malformed_reject_and_segmented_abort() {
    let ca = TestCa::new();
    let (mut f, port) = Fixture::new(&ca).await;
    let mut client = BACnetClient::start(ClientConfig::default(), port)
        .await
        .unwrap();
    let mut events = client.event_notifications();
    let mut peer = f.peer(ca.tls("event"), None).await;
    // Independent existing Clause 13 event wire vector: out-of-range transition.
    let payload = Bytes::from_static(&[
        0x09, 42, 0x1c, 2, 0, 0, 100, 0x2c, 0, 0, 0, 7, 0x3e, 0x19, 5, 0x3f, 0x49, 2, 0x59, 16,
        0x69, 5, 0x89, 0, 0x99, 1, 0xa9, 0, 0xb9, 3, 0xce, 0x5e, 0x0c, 0x42, 0x48, 0, 0, 0x1a, 4,
        0, 0x2c, 0x3f, 0x80, 0, 0, 0x3c, 0x42, 0x40, 0, 0, 0x5f, 0xcf,
    ]);
    let admitted = f
        .capture(
            &mut peer,
            &confirmed(
                ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
                70,
                payload,
            ),
            None,
        )
        .await;
    f.feed(admitted).await;
    bounded(events.recv()).await.unwrap();
    assert!(matches!(f.response(&peer).await.1, Apdu::SimpleAck(r) if r.invoke_id == 70));
    for service in [
        ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
        ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
    ] {
        let admitted = f
            .capture(&mut peer, &confirmed(service, 71, Bytes::new()), None)
            .await;
        f.feed(admitted).await;
        assert!(matches!(f.response(&peer).await.1, Apdu::Reject(r) if r.invoke_id == 71));
    }
    let Apdu::ConfirmedRequest(request) = unsupported(72) else {
        unreachable!()
    };
    let segmented = Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: true,
        more_follows: true,
        sequence_number: Some(0),
        proposed_window_size: Some(1),
        ..request
    });
    let admitted = f.capture(&mut peer, &segmented, None).await;
    f.feed(admitted).await;
    assert!(
        matches!(f.response(&peer).await.1, Apdu::Abort(r) if r.invoke_id == 72 && r.sent_by_server && r.abort_reason == AbortReason::SEGMENTATION_NOT_SUPPORTED)
    );
    client.stop().await.unwrap();
    f.listener.stop().await;
}
