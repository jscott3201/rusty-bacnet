use super::*;
use bacnet_encoding::apdu::{decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_services::cov::{SubscribeCOVPropertyRequest, SubscribeCOVRequest};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier, Segmentation};
use bacnet_types::primitives::ObjectIdentifier;
use std::time::Instant;

fn device_object(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

fn analog_object(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

fn routed_device(
    instance: u32,
    router_mac: &[u8],
    remote_network: u16,
    remote_mac: &[u8],
) -> DiscoveredDevice {
    DiscoveredDevice {
        object_identifier: device_object(instance),
        mac_address: MacAddr::from_slice(router_mac),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NONE,
        max_segments_accepted: None,
        vendor_id: 42,
        last_seen: Instant::now(),
        source_network: Some(remote_network),
        source_address: Some(MacAddr::from_slice(remote_mac)),
    }
}

async fn send_routed_simple_ack<T: TransportPort>(
    transport: &T,
    client_mac: &[u8],
    source_network: u16,
    source_mac: &[u8],
    invoke_id: u8,
    service_choice: ConfirmedServiceChoice,
) {
    let ack = Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice,
    });
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &ack).unwrap();

    let npdu = Npdu {
        source: Some(NpduAddress {
            network: source_network,
            mac_address: MacAddr::from_slice(source_mac),
        }),
        payload: apdu_buf.freeze(),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();
    transport.send_unicast(&npdu_buf, client_mac).await.unwrap();
}

async fn send_local_simple_ack<T: TransportPort>(
    transport: &T,
    client_mac: &[u8],
    invoke_id: u8,
    service_choice: ConfirmedServiceChoice,
) {
    let ack = Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice,
    });
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &ack).unwrap();

    let npdu = Npdu {
        payload: apdu_buf.freeze(),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();
    transport.send_unicast(&npdu_buf, client_mac).await.unwrap();
}

async fn receive_confirmed_request(
    router_rx: &mut tokio::sync::mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
    client_mac: &[u8],
    expected_destination: Option<(u16, &[u8])>,
    service_choice: ConfirmedServiceChoice,
) -> (u8, bytes::Bytes) {
    let received = timeout(Duration::from_secs(2), router_rx.recv())
        .await
        .expect("peer timed out waiting for confirmed request")
        .expect("router channel closed");
    assert_eq!(&received.source_mac[..], client_mac);

    let npdu = decode_npdu(received.npdu).unwrap();
    if let Some((remote_network, remote_mac)) = expected_destination {
        let destination = npdu.destination.expect("routed NPDU destination");
        assert_eq!(destination.network, remote_network);
        assert_eq!(&destination.mac_address[..], remote_mac);
    } else {
        assert!(
            npdu.destination.is_none(),
            "direct request must not set DNET/DADR"
        );
    }

    let decoded = decode_apdu(npdu.payload).unwrap();
    let Apdu::ConfirmedRequest(request) = decoded else {
        panic!("expected confirmed request, got {decoded:?}");
    };
    assert_eq!(request.service_choice, service_choice);
    assert!(!request.segmented);

    (request.invoke_id, request.service_request)
}

async fn receive_routed_subscribe_cov(
    router_rx: &mut tokio::sync::mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
    client_mac: &[u8],
    remote_network: u16,
    remote_mac: &[u8],
) -> (u8, SubscribeCOVRequest) {
    let (invoke_id, service_request) = receive_confirmed_request(
        router_rx,
        client_mac,
        Some((remote_network, remote_mac)),
        ConfirmedServiceChoice::SUBSCRIBE_COV,
    )
    .await;
    let cov_request = SubscribeCOVRequest::decode(&service_request).unwrap();
    (invoke_id, cov_request)
}

async fn receive_local_subscribe_cov_property(
    peer_rx: &mut tokio::sync::mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
    client_mac: &[u8],
) -> (u8, SubscribeCOVPropertyRequest) {
    let (invoke_id, service_request) = receive_confirmed_request(
        peer_rx,
        client_mac,
        None,
        ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
    )
    .await;
    let cov_request = SubscribeCOVPropertyRequest::decode(&service_request).unwrap();
    (invoke_id, cov_request)
}

async fn receive_routed_subscribe_cov_property(
    router_rx: &mut tokio::sync::mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
    client_mac: &[u8],
    remote_network: u16,
    remote_mac: &[u8],
) -> (u8, SubscribeCOVPropertyRequest) {
    let (invoke_id, service_request) = receive_confirmed_request(
        router_rx,
        client_mac,
        Some((remote_network, remote_mac)),
        ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
    )
    .await;
    let cov_request = SubscribeCOVPropertyRequest::decode(&service_request).unwrap();
    (invoke_id, cov_request)
}

#[tokio::test]
async fn subscribe_cov_to_device_uses_routed_addressing() {
    let client_mac = vec![0x01];
    let router_mac = vec![0x02];
    let remote_network = 100;
    let remote_mac = vec![0x03, 0x04];
    let monitored_object = analog_object(10);
    let (client_transport, mut router_transport) =
        LoopbackTransport::pair(client_mac.clone(), router_mac.clone());
    let mut router_rx = router_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();

    client.device_table.lock().await.upsert(routed_device(
        2001,
        &router_mac,
        remote_network,
        &remote_mac,
    ));

    let request_task = tokio::spawn(async move {
        let result = client
            .subscribe_cov_to_device(2001, 77, monitored_object, true, Some(120))
            .await;
        client.stop().await.unwrap();
        result
    });

    let (invoke_id, request) =
        receive_routed_subscribe_cov(&mut router_rx, &client_mac, remote_network, &remote_mac)
            .await;
    assert_eq!(request.subscriber_process_identifier, 77);
    assert_eq!(request.monitored_object_identifier, monitored_object);
    assert_eq!(request.issue_confirmed_notifications, Some(true));
    assert_eq!(request.lifetime, Some(120));

    send_routed_simple_ack(
        &router_transport,
        &client_mac,
        remote_network,
        &remote_mac,
        invoke_id,
        ConfirmedServiceChoice::SUBSCRIBE_COV,
    )
    .await;

    assert!(request_task.await.unwrap().is_ok());
    router_transport.stop().await.unwrap();
}

#[tokio::test]
async fn unsubscribe_cov_to_device_uses_routed_addressing() {
    let client_mac = vec![0x11];
    let router_mac = vec![0x12];
    let remote_network = 101;
    let remote_mac = vec![0x13, 0x14];
    let monitored_object = analog_object(11);
    let (client_transport, mut router_transport) =
        LoopbackTransport::pair(client_mac.clone(), router_mac.clone());
    let mut router_rx = router_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();

    client.device_table.lock().await.upsert(routed_device(
        2002,
        &router_mac,
        remote_network,
        &remote_mac,
    ));

    let request_task = tokio::spawn(async move {
        let result = client
            .unsubscribe_cov_to_device(2002, 78, monitored_object)
            .await;
        client.stop().await.unwrap();
        result
    });

    let (invoke_id, request) =
        receive_routed_subscribe_cov(&mut router_rx, &client_mac, remote_network, &remote_mac)
            .await;
    assert_eq!(request.subscriber_process_identifier, 78);
    assert_eq!(request.monitored_object_identifier, monitored_object);
    assert_eq!(request.issue_confirmed_notifications, None);
    assert_eq!(request.lifetime, None);

    send_routed_simple_ack(
        &router_transport,
        &client_mac,
        remote_network,
        &remote_mac,
        invoke_id,
        ConfirmedServiceChoice::SUBSCRIBE_COV,
    )
    .await;

    assert!(request_task.await.unwrap().is_ok());
    router_transport.stop().await.unwrap();
}

#[tokio::test]
async fn subscribe_cov_property_sends_property_request() {
    let client_mac = vec![0x21];
    let device_mac = vec![0x22];
    let monitored_object = analog_object(12);
    let (client_transport, mut device_transport) =
        LoopbackTransport::pair(client_mac.clone(), device_mac.clone());
    let mut device_rx = device_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();

    let request_task = tokio::spawn(async move {
        let result = client
            .subscribe_cov_property(
                &device_mac,
                79,
                monitored_object,
                PropertyIdentifier::PRESENT_VALUE,
                Some(2),
                false,
                Some(300),
                Some(0.25),
            )
            .await;
        client.stop().await.unwrap();
        result
    });

    let (invoke_id, request) =
        receive_local_subscribe_cov_property(&mut device_rx, &client_mac).await;
    assert_eq!(request.subscriber_process_identifier, 79);
    assert_eq!(request.monitored_object_identifier, monitored_object);
    assert_eq!(
        request.monitored_property_identifier,
        PropertyIdentifier::PRESENT_VALUE
    );
    assert_eq!(request.monitored_property_array_index, Some(2));
    assert_eq!(request.issue_confirmed_notifications, Some(false));
    assert_eq!(request.lifetime, Some(300));
    assert_eq!(request.cov_increment, Some(0.25));

    send_local_simple_ack(
        &device_transport,
        &client_mac,
        invoke_id,
        ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
    )
    .await;

    assert!(request_task.await.unwrap().is_ok());
    device_transport.stop().await.unwrap();
}

#[tokio::test]
async fn subscribe_cov_property_to_device_uses_routed_addressing() {
    let client_mac = vec![0x31];
    let router_mac = vec![0x32];
    let remote_network = 102;
    let remote_mac = vec![0x33, 0x34];
    let monitored_object = analog_object(13);
    let (client_transport, mut router_transport) =
        LoopbackTransport::pair(client_mac.clone(), router_mac.clone());
    let mut router_rx = router_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();

    client.device_table.lock().await.upsert(routed_device(
        2003,
        &router_mac,
        remote_network,
        &remote_mac,
    ));

    let request_task = tokio::spawn(async move {
        let result = client
            .subscribe_cov_property_to_device(
                2003,
                80,
                monitored_object,
                PropertyIdentifier::PRESENT_VALUE,
                None,
                true,
                Some(600),
                Some(1.5),
            )
            .await;
        client.stop().await.unwrap();
        result
    });

    let (invoke_id, request) = receive_routed_subscribe_cov_property(
        &mut router_rx,
        &client_mac,
        remote_network,
        &remote_mac,
    )
    .await;
    assert_eq!(request.subscriber_process_identifier, 80);
    assert_eq!(request.monitored_object_identifier, monitored_object);
    assert_eq!(
        request.monitored_property_identifier,
        PropertyIdentifier::PRESENT_VALUE
    );
    assert_eq!(request.monitored_property_array_index, None);
    assert_eq!(request.issue_confirmed_notifications, Some(true));
    assert_eq!(request.lifetime, Some(600));
    assert_eq!(request.cov_increment, Some(1.5));

    send_routed_simple_ack(
        &router_transport,
        &client_mac,
        remote_network,
        &remote_mac,
        invoke_id,
        ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
    )
    .await;

    assert!(request_task.await.unwrap().is_ok());
    router_transport.stop().await.unwrap();
}

#[tokio::test]
async fn unsubscribe_cov_property_sends_cancellation() {
    let client_mac = vec![0x41];
    let device_mac = vec![0x42];
    let monitored_object = analog_object(14);
    let (client_transport, mut device_transport) =
        LoopbackTransport::pair(client_mac.clone(), device_mac.clone());
    let mut device_rx = device_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();

    let request_task = tokio::spawn(async move {
        let result = client
            .unsubscribe_cov_property(
                &device_mac,
                81,
                monitored_object,
                PropertyIdentifier::PRESENT_VALUE,
                Some(3),
            )
            .await;
        client.stop().await.unwrap();
        result
    });

    let (invoke_id, request) =
        receive_local_subscribe_cov_property(&mut device_rx, &client_mac).await;
    assert_eq!(request.subscriber_process_identifier, 81);
    assert_eq!(request.monitored_object_identifier, monitored_object);
    assert_eq!(
        request.monitored_property_identifier,
        PropertyIdentifier::PRESENT_VALUE
    );
    assert_eq!(request.monitored_property_array_index, Some(3));
    assert_eq!(request.issue_confirmed_notifications, None);
    assert_eq!(request.lifetime, None);
    assert_eq!(request.cov_increment, None);

    send_local_simple_ack(
        &device_transport,
        &client_mac,
        invoke_id,
        ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
    )
    .await;

    assert!(request_task.await.unwrap().is_ok());
    device_transport.stop().await.unwrap();
}

#[tokio::test]
async fn unsubscribe_cov_property_to_device_uses_routed_addressing() {
    let client_mac = vec![0x51];
    let router_mac = vec![0x52];
    let remote_network = 103;
    let remote_mac = vec![0x53, 0x54];
    let monitored_object = analog_object(15);
    let (client_transport, mut router_transport) =
        LoopbackTransport::pair(client_mac.clone(), router_mac.clone());
    let mut router_rx = router_transport.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();

    client.device_table.lock().await.upsert(routed_device(
        2004,
        &router_mac,
        remote_network,
        &remote_mac,
    ));

    let request_task = tokio::spawn(async move {
        let result = client
            .unsubscribe_cov_property_to_device(
                2004,
                82,
                monitored_object,
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await;
        client.stop().await.unwrap();
        result
    });

    let (invoke_id, request) = receive_routed_subscribe_cov_property(
        &mut router_rx,
        &client_mac,
        remote_network,
        &remote_mac,
    )
    .await;
    assert_eq!(request.subscriber_process_identifier, 82);
    assert_eq!(request.monitored_object_identifier, monitored_object);
    assert_eq!(
        request.monitored_property_identifier,
        PropertyIdentifier::PRESENT_VALUE
    );
    assert_eq!(request.monitored_property_array_index, None);
    assert_eq!(request.issue_confirmed_notifications, None);
    assert_eq!(request.lifetime, None);
    assert_eq!(request.cov_increment, None);

    send_routed_simple_ack(
        &router_transport,
        &client_mac,
        remote_network,
        &remote_mac,
        invoke_id,
        ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
    )
    .await;

    assert!(request_task.await.unwrap().is_ok());
    router_transport.stop().await.unwrap();
}
