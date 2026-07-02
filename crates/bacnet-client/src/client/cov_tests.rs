use super::*;
use bacnet_encoding::apdu::{decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_services::cov::SubscribeCOVRequest;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, Segmentation};
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
) {
    let ack = Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice: ConfirmedServiceChoice::SUBSCRIBE_COV,
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

async fn receive_routed_subscribe_cov(
    router_rx: &mut tokio::sync::mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
    client_mac: &[u8],
    remote_network: u16,
    remote_mac: &[u8],
) -> (u8, SubscribeCOVRequest) {
    let received = timeout(Duration::from_secs(2), router_rx.recv())
        .await
        .expect("router timed out waiting for SubscribeCOV")
        .expect("router channel closed");
    assert_eq!(&received.source_mac[..], client_mac);

    let npdu = decode_npdu(received.npdu).unwrap();
    let destination = npdu.destination.expect("routed NPDU destination");
    assert_eq!(destination.network, remote_network);
    assert_eq!(&destination.mac_address[..], remote_mac);

    let decoded = decode_apdu(npdu.payload).unwrap();
    let Apdu::ConfirmedRequest(request) = decoded else {
        panic!("expected SubscribeCOV confirmed request, got {decoded:?}");
    };
    assert_eq!(
        request.service_choice,
        ConfirmedServiceChoice::SUBSCRIBE_COV
    );
    assert!(!request.segmented);

    let cov_request = SubscribeCOVRequest::decode(&request.service_request).unwrap();
    (request.invoke_id, cov_request)
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
    )
    .await;

    assert!(request_task.await.unwrap().is_ok());
    router_transport.stop().await.unwrap();
}
