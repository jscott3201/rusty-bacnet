//! End-to-end integration tests for the BACnet client.
//!
//! Proves the complete stack: client API -> encode service ->
//! APDU -> NPDU -> BVLL -> UDP -> receive -> decode -> respond ->
//! encode -> UDP -> receive -> decode -> return typed result.

use bacnet_client::client::BACnetClient;
use bacnet_encoding::apdu::{
    self, encode_apdu, Apdu, ComplexAck, UnconfirmedRequest as UnconfirmedRequestPdu,
};
use bacnet_encoding::primitives::encode_app_real;
use bacnet_network::layer::NetworkLayer;
use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_server::server::BACnetServer;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{
    ConfirmedServiceChoice, NetworkPriority, ObjectType, PropertyIdentifier, Segmentation,
    UnconfirmedServiceChoice,
};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::{Bytes, BytesMut};
use std::net::Ipv4Addr;
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn full_read_property_round_trip() {
    // Start the client
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();

    // Start a fake server on a separate transport
    let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut net_b = NetworkLayer::new(transport_b);
    let mut rx_b = net_b.start().await.unwrap();
    let b_mac = net_b.local_mac().to_vec();

    // Spawn the fake server: receives ReadProperty, responds with ReadPropertyACK
    let server_handle = tokio::spawn(async move {
        let received = timeout(Duration::from_secs(2), rx_b.recv())
            .await
            .expect("Server timed out waiting for request")
            .expect("Server channel closed");

        let decoded = apdu::decode_apdu(received.apdu.clone()).unwrap();
        if let Apdu::ConfirmedRequest(req) = decoded {
            assert_eq!(req.service_choice, ConfirmedServiceChoice::READ_PROPERTY);

            // Decode the service request
            let rp_req = ReadPropertyRequest::decode(&req.service_request).unwrap();
            assert_eq!(
                rp_req.object_identifier,
                ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
            );
            assert_eq!(
                rp_req.property_identifier,
                PropertyIdentifier::PRESENT_VALUE
            );

            // Build a ReadPropertyACK with a Real value (72.5)
            let ack = ReadPropertyACK {
                object_identifier: rp_req.object_identifier,
                property_identifier: rp_req.property_identifier,
                property_array_index: rp_req.property_array_index,
                property_value: vec![0x44, 0x42, 0x90, 0x00, 0x00], // app-tagged Real 72.5
            };
            let mut ack_buf = BytesMut::new();
            ack.encode(&mut ack_buf);

            let complex_ack = Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: req.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                service_ack: Bytes::from(ack_buf.to_vec()),
            });
            let mut buf = BytesMut::new();
            encode_apdu(&mut buf, &complex_ack);

            net_b
                .send_apdu(&buf, &received.source_mac, false, NetworkPriority::NORMAL)
                .await
                .unwrap();
        } else {
            panic!("Expected ConfirmedRequest, got {:?}", decoded);
        }

        net_b.stop().await.unwrap();
    });

    // Client sends ReadProperty
    let ack = client
        .read_property(
            &b_mac,
            ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )
        .await
        .unwrap();

    // Verify the response
    assert_eq!(
        ack.object_identifier,
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
    );
    assert_eq!(ack.property_identifier, PropertyIdentifier::PRESENT_VALUE);
    assert!(ack.property_array_index.is_none());
    assert_eq!(ack.property_value, vec![0x44, 0x42, 0x90, 0x00, 0x00]);

    // Cleanup
    server_handle.await.unwrap();
    client.stop().await.unwrap();
}

#[tokio::test]
async fn who_is_broadcast() {
    // Verify that who_is sends a broadcast without error.
    // Use LOCALHOST as broadcast address since 255.255.255.255 isn't routable in tests.
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .build()
        .await
        .unwrap();

    // WhoIs is fire-and-forget, so it should succeed immediately
    client.who_is(None, None).await.unwrap();
    client.who_is(Some(1000), Some(2000)).await.unwrap();

    client.stop().await.unwrap();
}

#[tokio::test]
async fn device_discovery_via_iam() {
    // Client that will discover devices
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();

    // "Server B" that will send an IAm to the client
    let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut net_b = NetworkLayer::new(transport_b);
    net_b.start().await.unwrap();

    // Build and send an IAm directly to the client
    let i_am = IAmRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 5678).unwrap(),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NONE,
        vendor_id: 42,
    };
    let mut service_buf = BytesMut::new();
    i_am.encode(&mut service_buf);

    let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
        service_choice: UnconfirmedServiceChoice::I_AM,
        service_request: Bytes::from(service_buf.to_vec()),
    });
    let mut buf = BytesMut::new();
    encode_apdu(&mut buf, &pdu);

    net_b
        .send_apdu(&buf, client.local_mac(), false, NetworkPriority::NORMAL)
        .await
        .unwrap();

    // Give the dispatch task a moment to process
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Client should now have the device in its discovery table
    let devices = client.discovered_devices().await;
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].object_identifier.instance_number(), 5678);
    assert_eq!(devices[0].vendor_id, 42);
    assert_eq!(devices[0].max_apdu_length, 1476);

    // Also test get_device
    let dev = client.get_device(5678).await;
    assert!(dev.is_some());
    assert!(client.get_device(9999).await.is_none());

    net_b.stop().await.unwrap();
    client.stop().await.unwrap();
}

#[tokio::test]
async fn cov_subscribe_and_notification() {
    // Set up a server with an AnalogOutput (writable without out-of-service)
    let mut db = ObjectDatabase::new();
    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    db.add(Box::new(ao)).unwrap();

    // Add a device object so COV notifications have an initiating-device-identifier
    use bacnet_objects::device::{DeviceConfig, DeviceObject};
    let dev = DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "Test-Dev".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(dev)).unwrap();

    let mut server = BACnetServer::builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(db)
        .build()
        .await
        .unwrap();
    let server_mac = server.local_mac().to_vec();

    // Start a client
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();

    let mut cov_rx = client.cov_notifications();

    // Subscribe to COV on AO:1 (unconfirmed notifications, 5 min lifetime)
    client
        .subscribe_cov(
            &server_mac,
            1, // process identifier
            ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
            false, // unconfirmed
            Some(300),
        )
        .await
        .unwrap();

    // Write a new value to AO:1 — this should trigger a COV notification
    let mut value_buf = BytesMut::new();
    encode_app_real(&mut value_buf, 99.0);
    client
        .write_property(
            &server_mac,
            ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value_buf.to_vec(),
            Some(16),
        )
        .await
        .unwrap();

    // Receive the COV notification
    let notification = timeout(Duration::from_secs(2), cov_rx.recv())
        .await
        .expect("Timed out waiting for COV notification")
        .expect("COV channel closed");

    assert_eq!(
        notification.monitored_object_identifier,
        ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap()
    );
    assert_eq!(
        notification.initiating_device_identifier,
        ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap()
    );
    // Should contain at least Present_Value
    assert!(notification
        .list_of_values
        .iter()
        .any(|pv| pv.property_identifier == PropertyIdentifier::PRESENT_VALUE));

    // Cleanup
    server.stop().await.unwrap();
    client.stop().await.unwrap();
}
