//! End-to-end integration tests for the BACnet client that require a real server.

use bacnet_client::client::{BACnetClient, COVEvent, COVNotificationDelivery};
use bacnet_encoding::primitives::encode_app_real;
use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_services::cov_multiple::{
    COVNotificationValue, COVReference, COVSubscriptionSpecification,
    SubscribeCOVPropertyMultipleRequest,
};
use bacnet_types::constructed::PropertyReference;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;
use std::net::Ipv4Addr;
use tokio::time::{Duration, timeout};

#[tokio::test]
async fn cov_subscribe_and_notification() {
    // Set up a server with an AnalogOutput (writable without out-of-service)
    let mut db = ObjectDatabase::new();
    let ao = AnalogOutputObject::new(1, "AO-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    db.add(Box::new(ao)).unwrap();

    // Add a device object so COV notifications have an initiating-device-identifier
    let dev = DeviceObject::new(DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(1234, 0)
            .expect("valid Device identity")
            .with_name("Test-Dev"),
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(dev)).unwrap();

    let mut server = BACnetServer::bip_builder()
        .device(bacnet_objects::DeviceIdentity::new(1234, 0).unwrap())
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

    let initial_notification = timeout(Duration::from_secs(2), cov_rx.recv())
        .await
        .expect("Timed out waiting for initial COV notification")
        .expect("COV channel closed")
        .into_object()
        .expect("a single-object notification");
    assert_eq!(
        initial_notification
            .notification
            .monitored_object_identifier,
        ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap()
    );
    assert!(
        initial_notification
            .notification
            .list_of_values
            .iter()
            .any(|pv| pv.property_identifier == PropertyIdentifier::PRESENT_VALUE)
    );

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
    let received = timeout(Duration::from_secs(2), cov_rx.recv())
        .await
        .expect("Timed out waiting for COV notification")
        .expect("COV channel closed")
        .into_object()
        .expect("a single-object notification");

    assert_eq!(
        received.notification.monitored_object_identifier,
        ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap()
    );
    assert_eq!(
        received.notification.initiating_device_identifier,
        ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap()
    );
    // Should contain at least Present_Value
    assert!(
        received
            .notification
            .list_of_values
            .iter()
            .any(|pv| pv.property_identifier == PropertyIdentifier::PRESENT_VALUE)
    );

    // Cleanup
    server.stop().await.unwrap();
    client.stop().await.unwrap();
}

/// A server with a writable AO:1 and its Device (1234), and its MAC.
async fn ao_server() -> (BACnetServer<bacnet_transport::bip::BipTransport>, Vec<u8>) {
    let mut db = ObjectDatabase::new();
    let ao = AnalogOutputObject::new(1, "AO-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    db.add(Box::new(ao)).unwrap();
    let dev = DeviceObject::new(DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(1234, 0)
            .expect("valid Device identity")
            .with_name("Test-Dev"),
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(dev)).unwrap();
    let server = BACnetServer::bip_builder()
        .device(bacnet_objects::DeviceIdentity::new(1234, 0).unwrap())
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(db)
        .build()
        .await
        .unwrap();
    let mac = server.local_mac().to_vec();
    (server, mac)
}

/// AO:1's Present_Value as the next COV notification, a COV-multiple one
/// from device 1234 for `process`, reports it.
async fn next_multiple(
    rx: &mut tokio::sync::broadcast::Receiver<COVEvent>,
    process: u32,
    confirmed: bool,
) -> COVNotificationValue {
    let event = timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("timed out waiting for a COV-multiple notification")
        .expect("COV channel closed");
    let COVEvent::Multiple(received) = event else {
        panic!("expected a COV-multiple notification: {event:?}");
    };
    let expected = if confirmed {
        COVNotificationDelivery::Confirmed
    } else {
        COVNotificationDelivery::Unconfirmed
    };
    assert_eq!(received.delivery, expected);
    let notification = received.notification;
    assert_eq!(notification.subscriber_process_identifier, process);
    assert_eq!(
        notification.initiating_device_identifier,
        ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap()
    );
    let [object] = &notification.list_of_cov_notifications[..] else {
        panic!("one object: {:?}", notification.list_of_cov_notifications);
    };
    assert_eq!(
        object.monitored_object_identifier,
        ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap()
    );
    let value = object
        .list_of_values
        .iter()
        .rev()
        .find(|value| value.property_identifier == PropertyIdentifier::PRESENT_VALUE)
        .expect("Present_Value")
        .clone();
    // Only the confirmed subscription below is timestamped: its values
    // carry their time of change, and the notification the time of its
    // last.
    assert_eq!(value.time_of_change.is_some(), confirmed);
    assert_eq!(notification.timestamp.is_some(), confirmed);
    value
}

/// A SubscribeCOVPropertyMultiple to AO:1's Present_Value: confirmed and
/// timestamped, then unconfirmed and not (#2654). Each subscription's
/// notifications reach `cov_notifications()` as `COVEvent::Multiple`, and
/// none comes once it is cancelled.
#[tokio::test]
async fn cov_property_multiple_notifies_and_cancels() {
    let (mut server, server_mac) = ao_server().await;
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();
    let mut cov_rx = client.cov_notifications();
    let ao1 = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
    let encoded = |value: f32| {
        let mut octets = BytesMut::new();
        encode_app_real(&mut octets, value);
        octets.to_vec()
    };

    for (process, confirmed) in [(21u8, true), (22, false)] {
        let process_id = u32::from(process);
        let mut request = SubscribeCOVPropertyMultipleRequest {
            subscriber_process_identifier: process_id,
            issue_confirmed_notifications: confirmed,
            lifetime: Some(300),
            max_notification_delay: Some(1),
            list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
                monitored_object_identifier: ao1,
                list_of_cov_references: vec![COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    },
                    cov_increment: None,
                    timestamped: confirmed,
                }],
            }],
        };
        client
            .subscribe_cov_property_multiple(&server_mac, &request)
            .await
            .unwrap();
        // The first notification reports the value as it stands.
        next_multiple(&mut cov_rx, process_id, confirmed).await;

        let new_value = f32::from(process);
        client
            .write_property(
                &server_mac,
                ao1,
                PropertyIdentifier::PRESENT_VALUE,
                None,
                encoded(new_value),
                Some(16),
            )
            .await
            .unwrap();
        let reported = next_multiple(&mut cov_rx, process_id, confirmed).await;
        assert_eq!(reported.value, encoded(new_value));

        // Cancel the whole context: no references and no timing.
        request.lifetime = None;
        request.max_notification_delay = None;
        request.list_of_cov_subscription_specifications.clear();
        client
            .subscribe_cov_property_multiple(&server_mac, &request)
            .await
            .unwrap();
        client
            .write_property(
                &server_mac,
                ao1,
                PropertyIdentifier::PRESENT_VALUE,
                None,
                encoded(new_value + 10.0),
                Some(16),
            )
            .await
            .unwrap();
        assert!(
            timeout(Duration::from_millis(2500), cov_rx.recv())
                .await
                .is_err(),
            "a notification after the cancel"
        );
    }

    server.stop().await.unwrap();
    client.stop().await.unwrap();
}
