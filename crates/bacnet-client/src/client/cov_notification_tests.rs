use super::*;
use bacnet_encoding::apdu::{decode_apdu, encode_apdu, UnconfirmedRequest};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier, RejectReason};
use bacnet_types::primitives::ObjectIdentifier;
use tokio::sync::broadcast::error::RecvError;

fn cov_notification(process_id: u32) -> COVNotificationRequest {
    COVNotificationRequest {
        subscriber_process_identifier: process_id,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap(),
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        time_remaining: 60,
        list_of_values: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0x44, 0x42, 0x48, 0x00, 0x00],
            priority: None,
        }],
    }
}

fn received_cov_notification(process_id: u32) -> ReceivedCOVNotification {
    ReceivedCOVNotification::new(
        cov_notification(process_id),
        &[],
        &None,
        COVNotificationDelivery::Unconfirmed,
    )
}

async fn send_cov_notification<T: TransportPort>(
    transport: &T,
    client_mac: &[u8],
    apdu: Apdu,
    source_network: Option<(u16, &[u8])>,
) {
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &apdu).unwrap();

    let npdu = Npdu {
        source: source_network.map(|(network, mac)| NpduAddress {
            network,
            mac_address: MacAddr::from_slice(mac),
        }),
        payload: apdu_buf.freeze(),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();
    transport.send_unicast(&npdu_buf, client_mac).await.unwrap();
}

async fn receive_peer_apdu(
    peer_rx: &mut tokio::sync::mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
) -> Apdu {
    receive_peer_npdu_apdu(peer_rx).await.1
}

async fn receive_peer_npdu_apdu(
    peer_rx: &mut tokio::sync::mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
) -> (Npdu, Apdu) {
    let received = timeout(Duration::from_secs(2), peer_rx.recv())
        .await
        .expect("peer timed out waiting for response")
        .expect("peer channel closed");
    let npdu = decode_npdu(received.npdu).unwrap();
    let apdu = decode_apdu(npdu.payload.clone()).unwrap();
    (npdu, apdu)
}

#[tokio::test]
async fn cov_notification_channel_uses_configured_capacity() {
    assert_eq!(ClientOptions::default().cov_channel_capacity, 64);
    let max_options = ClientOptions::default().with_cov_channel_capacity(MAX_COV_CHANNEL_CAPACITY);
    assert!(max_options.validate().is_ok());
    for capacity in [0, MAX_COV_CHANNEL_CAPACITY + 1] {
        let (bad_transport, _) = LoopbackTransport::pair(vec![0x10], vec![0x11]);
        assert!(BACnetClient::generic_builder()
            .transport(bad_transport)
            .cov_channel_capacity(capacity)
            .build()
            .await
            .is_err());
    }
    let (transport, _) = LoopbackTransport::pair(vec![0x20], vec![0x21]);
    let mut client = BACnetClient::generic_builder()
        .transport(transport)
        .cov_channel_capacity(1)
        .build()
        .await
        .unwrap();
    let mut rx = client.cov_notifications();
    client.cov_tx.send(received_cov_notification(1)).unwrap();
    client.cov_tx.send(received_cov_notification(2)).unwrap();
    assert!(matches!(rx.recv().await, Err(RecvError::Lagged(1))));
    assert_eq!(
        rx.recv()
            .await
            .unwrap()
            .notification
            .subscriber_process_identifier,
        2
    );
    client.stop().await.unwrap();
}

#[tokio::test]
async fn unconfirmed_cov_notification_includes_source_and_delivery() {
    let client_mac = vec![0x01];
    let peer_mac = vec![0x02];
    let routed_source_mac = vec![0x03, 0x04];
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), peer_mac.clone());
    let mut _peer_rx = peer_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let mut rx = client.cov_notifications();

    let notification = cov_notification(10);
    let mut service_request = BytesMut::new();
    notification.encode(&mut service_request);
    send_cov_notification(
        &peer_transport,
        &client_mac,
        Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
            service_request: service_request.freeze(),
        }),
        Some((200, &routed_source_mac)),
    )
    .await;

    let received = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timed out waiting for COV notification")
        .expect("COV channel closed");
    assert_eq!(received.delivery, COVNotificationDelivery::Unconfirmed);
    assert_eq!(&received.source_mac[..], &peer_mac);
    assert_eq!(received.source_network, Some(200));
    assert_eq!(
        received.source_address.as_deref(),
        Some(routed_source_mac.as_slice())
    );
    assert_eq!(
        received.notification.monitored_object_identifier,
        notification.monitored_object_identifier
    );

    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn confirmed_cov_notification_enqueues_and_sends_simple_ack() {
    let client_mac = vec![0x11];
    let peer_mac = vec![0x12];
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), peer_mac.clone());
    let mut peer_rx = peer_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let mut rx = client.cov_notifications();

    let notification = cov_notification(11);
    let mut service_request = BytesMut::new();
    notification.encode(&mut service_request);
    send_cov_notification(
        &peer_transport,
        &client_mac,
        Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: true,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 44,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
            service_request: service_request.freeze(),
        }),
        None,
    )
    .await;

    let received = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timed out waiting for COV notification")
        .expect("COV channel closed");
    assert_eq!(received.delivery, COVNotificationDelivery::Confirmed);
    assert_eq!(&received.source_mac[..], &peer_mac);
    assert_eq!(received.source_network, None);
    assert_eq!(received.source_address, None);

    let Apdu::SimpleAck(ack) = receive_peer_apdu(&mut peer_rx).await else {
        panic!("expected SimpleAck response");
    };
    assert_eq!(ack.invoke_id, 44);
    assert_eq!(
        ack.service_choice,
        ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION
    );

    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn confirmed_cov_notification_routes_ack_to_npdu_source() {
    let client_mac = vec![0x41];
    let router_mac = vec![0x42];
    let remote_mac = vec![0x43, 0x44];
    let (client_transport, mut router_transport) =
        LoopbackTransport::pair(client_mac.clone(), router_mac.clone());
    let mut router_rx = router_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let mut rx = client.cov_notifications();

    let notification = cov_notification(14);
    let mut service_request = BytesMut::new();
    notification.encode(&mut service_request);
    send_cov_notification(
        &router_transport,
        &client_mac,
        Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: true,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 47,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
            service_request: service_request.freeze(),
        }),
        Some((300, &remote_mac)),
    )
    .await;

    let received = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timed out waiting for COV notification")
        .expect("COV channel closed");
    assert_eq!(received.delivery, COVNotificationDelivery::Confirmed);
    assert_eq!(&received.source_mac[..], &router_mac);
    assert_eq!(received.source_network, Some(300));
    assert_eq!(
        received.source_address.as_deref(),
        Some(remote_mac.as_slice())
    );

    let (npdu, apdu) = receive_peer_npdu_apdu(&mut router_rx).await;
    assert_eq!(
        npdu.destination,
        Some(NpduAddress {
            network: 300,
            mac_address: MacAddr::from_slice(&remote_mac),
        })
    );
    let Apdu::SimpleAck(ack) = apdu else {
        panic!("expected routed SimpleAck response");
    };
    assert_eq!(ack.invoke_id, 47);
    assert_eq!(
        ack.service_choice,
        ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION
    );

    client.stop().await.unwrap();
    router_transport.stop().await.unwrap();
}

#[tokio::test]
async fn confirmed_cov_notification_ack_policy_can_reject() {
    let client_mac = vec![0x21];
    let peer_mac = vec![0x22];
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), peer_mac.clone());
    let mut peer_rx = peer_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .confirmed_cov_notification_ack_policy(|received| {
            assert_eq!(received.delivery, COVNotificationDelivery::Confirmed);
            ConfirmedCOVNotificationResponse::Reject(RejectReason::OTHER)
        })
        .build()
        .await
        .unwrap();
    let mut rx = client.cov_notifications();

    let notification = cov_notification(12);
    let mut service_request = BytesMut::new();
    notification.encode(&mut service_request);
    send_cov_notification(
        &peer_transport,
        &client_mac,
        Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: true,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 45,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
            service_request: service_request.freeze(),
        }),
        None,
    )
    .await;

    let received = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timed out waiting for COV notification")
        .expect("COV channel closed");
    assert_eq!(received.notification.subscriber_process_identifier, 12);

    let Apdu::Reject(reject) = receive_peer_apdu(&mut peer_rx).await else {
        panic!("expected Reject response");
    };
    assert_eq!(reject.invoke_id, 45);
    assert_eq!(reject.reject_reason, RejectReason::OTHER);

    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn confirmed_cov_notification_ack_policy_can_suppress_response() {
    let client_mac = vec![0x31];
    let peer_mac = vec![0x32];
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), peer_mac.clone());
    let mut peer_rx = peer_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .confirmed_cov_notification_ack_policy(|_| ConfirmedCOVNotificationResponse::NoResponse)
        .build()
        .await
        .unwrap();
    let mut rx = client.cov_notifications();

    let notification = cov_notification(13);
    let mut service_request = BytesMut::new();
    notification.encode(&mut service_request);
    send_cov_notification(
        &peer_transport,
        &client_mac,
        Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: true,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 46,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
            service_request: service_request.freeze(),
        }),
        None,
    )
    .await;

    let received = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timed out waiting for COV notification")
        .expect("COV channel closed");
    assert_eq!(received.notification.subscriber_process_identifier, 13);
    assert!(timeout(Duration::from_millis(100), peer_rx.recv())
        .await
        .is_err());

    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}
