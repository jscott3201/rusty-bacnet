//! End-to-end integration tests for the BACnet client that require a real server.

use bacnet_client::client::BACnetClient;
use bacnet_encoding::primitives::encode_app_real;
use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;
use std::net::Ipv4Addr;
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn cov_subscribe_and_notification() {
    // Set up a server with an AnalogOutput (writable without out-of-service)
    let mut db = ObjectDatabase::new();
    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    db.add(Box::new(ao)).unwrap();

    // Add a device object so COV notifications have an initiating-device-identifier
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
