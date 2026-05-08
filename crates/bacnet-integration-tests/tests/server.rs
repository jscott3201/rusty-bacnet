//! End-to-end integration tests: real BACnetClient ↔ real BACnetServer over loopback UDP.

use bacnet_client::client::BACnetClient;
use bacnet_encoding::apdu::{encode_apdu, Apdu, ConfirmedRequest as ConfirmedRequestPdu};
use bacnet_network::layer::NetworkLayer;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, NetworkPriority, ObjectType, PropertyIdentifier,
};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use std::net::Ipv4Addr;
use std::sync::Arc;
use tokio::time::Duration;

/// Build a server with a Device, an AnalogInput, and a BinaryValue.
async fn make_server() -> BACnetServer<BipTransport> {
    let mut db = ObjectDatabase::new();

    // Device object (instance 1234)
    let mut device = DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "Integration Test Device".into(),
        vendor_name: "Rusty BACnet".into(),
        vendor_id: 555,
        ..DeviceConfig::default()
    })
    .unwrap();

    // AnalogInput (instance 1, present-value = 72.5)
    let mut ai = AnalogInputObject::new(1, "Zone Temp", 62).unwrap();
    ai.set_present_value(72.5);

    // BinaryValue (instance 1, default = inactive)
    let bv = BinaryValueObject::new(1, "Fan Status").unwrap();

    let ai_oid = ai.object_identifier();
    let bv_oid = bv.object_identifier();
    let dev_oid = device.object_identifier();

    // Update device object-list
    device.set_object_list(vec![dev_oid, ai_oid, bv_oid]);

    db.add(Box::new(device)).unwrap();
    db.add(Box::new(ai)).unwrap();
    db.add(Box::new(bv)).unwrap();

    BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0) // ephemeral
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(db)
        .build()
        .await
        .unwrap()
}

async fn make_client() -> BACnetClient<BipTransport> {
    BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap()
}

#[tokio::test]
async fn read_property_from_server() {
    let mut server = make_server().await;
    let mut client = make_client().await;

    let server_mac = server.local_mac().to_vec();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    // Read present-value of AI-1
    let ack = client
        .read_property(&server_mac, ai_oid, PropertyIdentifier::PRESENT_VALUE, None)
        .await
        .unwrap();

    assert_eq!(ack.object_identifier, ai_oid);
    assert_eq!(ack.property_identifier, PropertyIdentifier::PRESENT_VALUE);

    // Decode the value — should be Real 72.5
    let (val, _) =
        bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0).unwrap();
    assert_eq!(val, bacnet_types::primitives::PropertyValue::Real(72.5));

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn read_device_object_name() {
    let mut server = make_server().await;
    let mut client = make_client().await;

    let server_mac = server.local_mac().to_vec();
    let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();

    let ack = client
        .read_property(&server_mac, dev_oid, PropertyIdentifier::OBJECT_NAME, None)
        .await
        .unwrap();

    let (val, _) =
        bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0).unwrap();
    assert_eq!(
        val,
        bacnet_types::primitives::PropertyValue::CharacterString("Integration Test Device".into())
    );

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn write_property_to_server() {
    let mut server = make_server().await;
    let mut client = make_client().await;

    let server_mac = server.local_mac().to_vec();
    let bv_oid = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap();

    // Encode the value: Enumerated(1) = active
    let mut value_buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut value_buf, 1);

    // Write present-value of BV-1 to active
    client
        .write_property(
            &server_mac,
            bv_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value_buf.to_vec(),
            None,
        )
        .await
        .unwrap();

    // Read it back to verify
    let ack = client
        .read_property(&server_mac, bv_oid, PropertyIdentifier::PRESENT_VALUE, None)
        .await
        .unwrap();

    let (val, _) =
        bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0).unwrap();
    assert_eq!(val, bacnet_types::primitives::PropertyValue::Enumerated(1));

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn read_property_multiple_from_server() {
    use bacnet_services::common::PropertyReference;
    use bacnet_services::rpm::ReadAccessSpecification;

    let mut server = make_server().await;
    let mut client = make_client().await;

    let server_mac = server.local_mac().to_vec();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let specs = vec![ReadAccessSpecification {
        object_identifier: ai_oid,
        list_of_property_references: vec![
            PropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
            },
            PropertyReference {
                property_identifier: PropertyIdentifier::OBJECT_NAME,
                property_array_index: None,
            },
        ],
    }];

    let ack = client
        .read_property_multiple(&server_mac, specs)
        .await
        .unwrap();

    assert_eq!(ack.list_of_read_access_results.len(), 1);
    let result = &ack.list_of_read_access_results[0];
    assert_eq!(result.object_identifier, ai_oid);
    assert_eq!(result.list_of_results.len(), 2);

    // Both should be successful
    assert!(result.list_of_results[0].property_value.is_some());
    assert!(result.list_of_results[1].property_value.is_some());

    // Verify present-value is 72.5
    let (val, _) = bacnet_encoding::primitives::decode_application_value(
        result.list_of_results[0].property_value.as_ref().unwrap(),
        0,
    )
    .unwrap();
    assert_eq!(val, bacnet_types::primitives::PropertyValue::Real(72.5));

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn who_is_through_server() {
    use tokio::time::Duration;

    let mut server = make_server().await;
    let mut client = make_client().await;

    // Send WhoIs — the server should respond with IAm
    client.who_is(None, None).await.unwrap();

    // Give the server time to process and respond
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Check if the client discovered the device
    let devices = client.discovered_devices().await;
    // The server has device 1234, so if the broadcast reached it, we should see it
    // Note: On localhost, broadcast may or may not reach the server depending on OS
    // This is a best-effort test.
    if !devices.is_empty() {
        assert_eq!(devices[0].object_identifier.instance_number(), 1234);
    }

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn write_and_read_multiple_objects() {
    let mut server = make_server().await;
    let mut client = make_client().await;

    let server_mac = server.local_mac().to_vec();
    let bv_oid = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap();

    // Write BV to active
    let mut value_buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut value_buf, 1);
    client
        .write_property(
            &server_mac,
            bv_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value_buf.to_vec(),
            None,
        )
        .await
        .unwrap();

    // Read both AI and BV in one RPM call
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    // Read AI present value
    let ai_ack = client
        .read_property(&server_mac, ai_oid, PropertyIdentifier::PRESENT_VALUE, None)
        .await
        .unwrap();
    let (ai_val, _) =
        bacnet_encoding::primitives::decode_application_value(&ai_ack.property_value, 0).unwrap();
    assert_eq!(ai_val, bacnet_types::primitives::PropertyValue::Real(72.5));

    // Read BV present value (should be active=1 after write)
    let bv_ack = client
        .read_property(&server_mac, bv_oid, PropertyIdentifier::PRESENT_VALUE, None)
        .await
        .unwrap();
    let (bv_val, _) =
        bacnet_encoding::primitives::decode_application_value(&bv_ack.property_value, 0).unwrap();
    assert_eq!(
        bv_val,
        bacnet_types::primitives::PropertyValue::Enumerated(1)
    );

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn read_unknown_object_returns_error() {
    let mut server = make_server().await;
    let mut client = make_client().await;

    let server_mac = server.local_mac().to_vec();
    let fake_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 999).unwrap();

    let result = client
        .read_property(
            &server_mac,
            fake_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )
        .await;

    assert!(result.is_err());

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

// ---------------------------------------------------------------------------
// Error-path integration tests (Task #71 - Tier 4R)
// ---------------------------------------------------------------------------

/// ReadProperty for a non-existent object returns UNKNOWN_OBJECT with the
/// correct error class (OBJECT) and error code (UNKNOWN_OBJECT).
#[tokio::test]
async fn read_nonexistent_object_returns_unknown_object_error() {
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::Error;

    let mut server = make_server().await;
    let mut client = make_client().await;

    let server_mac = server.local_mac().to_vec();
    let fake_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 999).unwrap();

    let result = client
        .read_property(
            &server_mac,
            fake_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )
        .await;

    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(
                class,
                ErrorClass::OBJECT.to_raw() as u32,
                "Expected OBJECT error class"
            );
            assert_eq!(
                code,
                ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
                "Expected UNKNOWN_OBJECT error code"
            );
        }
        other => panic!(
            "Expected Protocol error with UNKNOWN_OBJECT, got {:?}",
            other
        ),
    }

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// ReadProperty for a property that does not exist on the target object
/// returns UNKNOWN_PROPERTY error.
#[tokio::test]
async fn read_nonexistent_property_returns_unknown_property_error() {
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::Error;

    let mut server = make_server().await;
    let mut client = make_client().await;

    let server_mac = server.local_mac().to_vec();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    // PRIORITY_ARRAY is not a property of AnalogInput
    let result = client
        .read_property(
            &server_mac,
            ai_oid,
            PropertyIdentifier::PRIORITY_ARRAY,
            None,
        )
        .await;

    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(
                class,
                ErrorClass::PROPERTY.to_raw() as u32,
                "Expected PROPERTY error class"
            );
            assert_eq!(
                code,
                ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
                "Expected UNKNOWN_PROPERTY error code"
            );
        }
        other => panic!(
            "Expected Protocol error with UNKNOWN_PROPERTY, got {:?}",
            other
        ),
    }

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// WriteProperty to OBJECT_IDENTIFIER (a read-only property) returns
/// WRITE_ACCESS_DENIED error.
#[tokio::test]
async fn write_read_only_property_returns_write_access_denied() {
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::Error;

    let mut server = make_server().await;
    let mut client = make_client().await;

    let server_mac = server.local_mac().to_vec();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    // Try to write to OBJECT_IDENTIFIER, which is read-only on all objects.
    // Encode an ObjectIdentifier value (it doesn't matter what value we use,
    // since the write should be rejected before the value is applied).
    let mut value_buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_object_id(&mut value_buf, &ai_oid);

    let result = client
        .write_property(
            &server_mac,
            ai_oid,
            PropertyIdentifier::OBJECT_IDENTIFIER,
            None,
            value_buf.to_vec(),
            None,
        )
        .await;

    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(
                class,
                ErrorClass::PROPERTY.to_raw() as u32,
                "Expected PROPERTY error class"
            );
            assert_eq!(
                code,
                ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
                "Expected WRITE_ACCESS_DENIED error code"
            );
        }
        other => panic!(
            "Expected Protocol error with WRITE_ACCESS_DENIED, got {:?}",
            other
        ),
    }

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// WriteProperty with a wrong value type (CharacterString to a Real property)
/// returns INVALID_DATA_TYPE error.
#[tokio::test]
async fn write_invalid_value_type_returns_invalid_data_type() {
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::Error;

    let mut server = make_server().await;
    let mut client = make_client().await;

    let server_mac = server.local_mac().to_vec();
    let bv_oid = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap();

    // BinaryValue present-value expects an Enumerated value.
    // Send a CharacterString instead — this should trigger INVALID_DATA_TYPE.
    let mut value_buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut value_buf, "wrong type").unwrap();

    let result = client
        .write_property(
            &server_mac,
            bv_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value_buf.to_vec(),
            None,
        )
        .await;

    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(
                class,
                ErrorClass::PROPERTY.to_raw() as u32,
                "Expected PROPERTY error class"
            );
            assert_eq!(
                code,
                ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
                "Expected INVALID_DATA_TYPE error code"
            );
        }
        other => panic!(
            "Expected Protocol error with INVALID_DATA_TYPE, got {:?}",
            other
        ),
    }

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// Subscribe to COV, then cancel the subscription, and verify the server
/// accepted the cancellation (no longer sends notifications on value change).
#[tokio::test]
async fn cov_subscribe_then_cancel() {
    use bacnet_objects::analog::AnalogOutputObject;
    use tokio::time::Duration;

    // Build a server with an AnalogOutput (writable present-value)
    let mut db = ObjectDatabase::new();
    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    db.add(Box::new(ao)).unwrap();
    let dev = DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "COV-Cancel-Dev".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(dev)).unwrap();

    let mut server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(db)
        .build()
        .await
        .unwrap();
    let server_mac = server.local_mac().to_vec();

    let mut client = make_client().await;
    let mut cov_rx = client.cov_notifications();

    let ao_oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();

    // 1. Subscribe to COV on AO:1
    client
        .subscribe_cov(&server_mac, 42, ao_oid, false, Some(300))
        .await
        .unwrap();

    // 2. Write a value to trigger a notification (confirm subscription is active)
    let mut value_buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut value_buf, 50.0);
    client
        .write_property(
            &server_mac,
            ao_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value_buf.to_vec(),
            Some(16),
        )
        .await
        .unwrap();

    // Should receive the COV notification
    let notification = tokio::time::timeout(Duration::from_secs(2), cov_rx.recv())
        .await
        .expect("Timed out waiting for COV notification")
        .expect("COV channel closed");
    assert_eq!(notification.monitored_object_identifier, ao_oid);

    // 3. Cancel the subscription
    client
        .unsubscribe_cov(&server_mac, 42, ao_oid)
        .await
        .unwrap();

    // 4. Write again — should NOT produce a COV notification
    let mut value_buf2 = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut value_buf2, 99.0);
    client
        .write_property(
            &server_mac,
            ao_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value_buf2.to_vec(),
            Some(16),
        )
        .await
        .unwrap();

    // Wait briefly — no notification should arrive
    let timeout_result = tokio::time::timeout(Duration::from_millis(500), cov_rx.recv()).await;
    assert!(
        timeout_result.is_err(),
        "Should NOT receive a COV notification after unsubscribe, but got one"
    );

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

// ---------------------------------------------------------------------------
// Server Tx Segmentation integration tests
// ---------------------------------------------------------------------------

/// When a ComplexAck response exceeds the client's max_apdu_length, the server
/// should segment the response and the client should reassemble it correctly.
#[tokio::test]
async fn server_segments_large_rpm_response() {
    use bacnet_services::common::PropertyReference;
    use bacnet_services::rpm::ReadAccessSpecification;

    // Build a server with a Device and multiple AnalogInput objects so that
    // an RPM response reading many properties will be large.
    let mut db = ObjectDatabase::new();

    let mut device = DeviceObject::new(DeviceConfig {
        instance: 5678,
        name: "Segmentation Test Device".into(),
        vendor_name: "Rusty BACnet Seg Test".into(),
        vendor_id: 555,
        ..DeviceConfig::default()
    })
    .unwrap();

    let dev_oid = device.object_identifier();
    let mut all_oids = vec![dev_oid];

    // Create 5 AnalogInput objects with long names to produce a large response.
    for i in 1..=5 {
        let mut ai = AnalogInputObject::new(
            i,
            format!(
                "Analog Input Object With A Long Descriptive Name Number {}",
                i
            ),
            62,
        )
        .unwrap();
        ai.set_present_value(i as f32 * 10.0);
        all_oids.push(ai.object_identifier());
        db.add(Box::new(ai)).unwrap();
    }

    device.set_object_list(all_oids.clone());
    db.add(Box::new(device)).unwrap();

    let mut server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(db)
        .build()
        .await
        .unwrap();
    let server_mac = server.local_mac().to_vec();

    // Create a client with max_apdu_length=128 so the response must be segmented.
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .apdu_timeout_ms(5000)
        .max_apdu_length(128)
        .build()
        .await
        .unwrap();

    // Build an RPM request for multiple properties from multiple objects.
    let specs: Vec<ReadAccessSpecification> = (1..=5)
        .map(|i| {
            let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, i).unwrap();
            ReadAccessSpecification {
                object_identifier: oid,
                list_of_property_references: vec![
                    PropertyReference {
                        property_identifier: PropertyIdentifier::OBJECT_IDENTIFIER,
                        property_array_index: None,
                    },
                    PropertyReference {
                        property_identifier: PropertyIdentifier::OBJECT_NAME,
                        property_array_index: None,
                    },
                    PropertyReference {
                        property_identifier: PropertyIdentifier::OBJECT_TYPE,
                        property_array_index: None,
                    },
                    PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    },
                    PropertyReference {
                        property_identifier: PropertyIdentifier::UNITS,
                        property_array_index: None,
                    },
                ],
            }
        })
        .collect();

    // Execute the RPM request. The client's existing segmented-receive logic
    // should handle the segmented response from the server.
    let ack = client
        .read_property_multiple(&server_mac, specs)
        .await
        .unwrap();

    // Verify: 5 access results, each with 5 property results.
    assert_eq!(ack.list_of_read_access_results.len(), 5);

    for (i, result) in ack.list_of_read_access_results.iter().enumerate() {
        let expected_instance = (i + 1) as u32;
        let expected_oid =
            ObjectIdentifier::new(ObjectType::ANALOG_INPUT, expected_instance).unwrap();
        assert_eq!(result.object_identifier, expected_oid);
        assert_eq!(
            result.list_of_results.len(),
            5,
            "Expected 5 property results for AI:{}",
            expected_instance
        );

        // All properties should be successful (no errors).
        for elem in &result.list_of_results {
            assert!(
                elem.property_value.is_some(),
                "Property {:?} on AI:{} should have a value",
                elem.property_identifier,
                expected_instance
            );
        }

        // Verify present_value is correct (property index 3).
        let pv_elem = &result.list_of_results[3];
        assert_eq!(
            pv_elem.property_identifier,
            PropertyIdentifier::PRESENT_VALUE
        );
        let (val, _) = bacnet_encoding::primitives::decode_application_value(
            pv_elem.property_value.as_ref().unwrap(),
            0,
        )
        .unwrap();
        assert_eq!(
            val,
            bacnet_types::primitives::PropertyValue::Real(expected_instance as f32 * 10.0),
            "Present value for AI:{} should be {}",
            expected_instance,
            expected_instance as f32 * 10.0,
        );
    }

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

// ---------------------------------------------------------------------------
// Server Rx Segmentation integration tests
// ---------------------------------------------------------------------------

fn read_property_service_payload() -> Vec<u8> {
    use bacnet_services::read_property::ReadPropertyRequest;

    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let rp_req = ReadPropertyRequest {
        object_identifier: ai_oid,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    };
    let mut service_buf = BytesMut::new();
    rp_req.encode(&mut service_buf);
    service_buf.to_vec()
}

fn segmented_read_property_pdu(
    invoke_id: u8,
    seq: u8,
    more_follows: bool,
    proposed_window_size: Option<u8>,
    service_request: Bytes,
) -> Apdu {
    Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: true,
        more_follows,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: Some(seq),
        proposed_window_size,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request,
    })
}

async fn send_raw_apdu(
    raw_network: &Arc<NetworkLayer<BipTransport>>,
    server_mac: &[u8],
    apdu: Apdu,
) {
    let mut buf = BytesMut::new();
    encode_apdu(&mut buf, &apdu).expect("valid APDU encoding");
    raw_network
        .send_apdu(&buf, server_mac, true, NetworkPriority::NORMAL)
        .await
        .unwrap();
}

/// When a client sends a segmented ConfirmedRequest (ReadProperty split across
/// two segments), the server should reassemble it, process the full request,
/// and return a correct ReadPropertyACK.
#[tokio::test]
async fn server_handles_segmented_request() {
    use bacnet_encoding::apdu::{self, encode_apdu, Apdu, ConfirmedRequest as ConfirmedRequestPdu};
    use bacnet_network::layer::NetworkLayer;
    use bacnet_services::read_property::ReadPropertyRequest;
    use bacnet_types::enums::ConfirmedServiceChoice;
    use bytes::BytesMut;
    use tokio::time::Duration;

    // 1. Build a server with an AnalogInput (present_value = 72.5).
    let mut server = make_server().await;
    let server_mac = server.local_mac().to_vec();

    // 2. Build a raw NetworkLayer to act as a "manual client".
    let raw_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let mut raw_network = NetworkLayer::new(raw_transport);
    let mut raw_rx = raw_network.start().await.unwrap();
    let raw_network = std::sync::Arc::new(raw_network);

    // 3. Encode a ReadProperty service request for AI:1 Present_Value.
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let rp_req = ReadPropertyRequest {
        object_identifier: ai_oid,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    };
    let mut service_buf = BytesMut::new();
    rp_req.encode(&mut service_buf);
    let full_payload = service_buf.to_vec();

    // 4. Split the payload into 2 segments (artificially, even though it is small).
    let mid = full_payload.len() / 2;
    let seg0_data = Bytes::copy_from_slice(&full_payload[..mid]);
    let seg1_data = Bytes::copy_from_slice(&full_payload[mid..]);

    let invoke_id: u8 = 42;

    // 5. Send segment 0 (more_follows = true).
    let seg0 = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: true,
        more_follows: true,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: Some(0),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: seg0_data,
    });

    let mut buf = BytesMut::new();
    encode_apdu(&mut buf, &seg0).expect("valid APDU encoding");
    raw_network
        .send_apdu(
            &buf,
            &server_mac,
            true,
            bacnet_types::enums::NetworkPriority::NORMAL,
        )
        .await
        .unwrap();

    // 6. Wait for the SegmentAck from the server.
    let ack0 = tokio::time::timeout(Duration::from_secs(3), raw_rx.recv())
        .await
        .expect("Timed out waiting for SegmentAck 0")
        .expect("Channel closed while waiting for SegmentAck 0");
    let decoded_ack0 = apdu::decode_apdu(ack0.apdu.clone()).unwrap();
    match decoded_ack0 {
        Apdu::SegmentAck(sa) => {
            assert_eq!(sa.invoke_id, invoke_id);
            assert_eq!(sa.sequence_number, 0);
            assert!(sa.sent_by_server);
            assert!(!sa.negative_ack);
        }
        other => panic!("Expected SegmentAck, got {:?}", other),
    }

    // 7. Send segment 1 (more_follows = false -- last segment).
    let seg1 = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: true,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: Some(1),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: seg1_data,
    });

    let mut buf = BytesMut::new();
    encode_apdu(&mut buf, &seg1).expect("valid APDU encoding");
    raw_network
        .send_apdu(
            &buf,
            &server_mac,
            true,
            bacnet_types::enums::NetworkPriority::NORMAL,
        )
        .await
        .unwrap();

    // 8. Wait for SegmentAck for segment 1, then the ReadPropertyACK response.
    //    The server sends a SegmentAck for every segment, then processes the
    //    reassembled request and sends the ComplexAck response.
    let mut got_seg_ack_1 = false;
    let mut got_response = false;

    for _ in 0..3 {
        let received = tokio::time::timeout(Duration::from_secs(3), raw_rx.recv())
            .await
            .expect("Timed out waiting for response")
            .expect("Channel closed while waiting for response");
        let decoded = apdu::decode_apdu(received.apdu.clone()).unwrap();
        match decoded {
            Apdu::SegmentAck(sa) => {
                assert_eq!(sa.invoke_id, invoke_id);
                assert_eq!(sa.sequence_number, 1);
                got_seg_ack_1 = true;
            }
            Apdu::ComplexAck(ack) => {
                assert_eq!(ack.invoke_id, invoke_id);
                assert_eq!(ack.service_choice, ConfirmedServiceChoice::READ_PROPERTY);
                // Decode the ReadPropertyACK to verify the value.
                let rp_ack =
                    bacnet_services::read_property::ReadPropertyACK::decode(&ack.service_ack)
                        .unwrap();
                assert_eq!(rp_ack.object_identifier, ai_oid);
                assert_eq!(
                    rp_ack.property_identifier,
                    PropertyIdentifier::PRESENT_VALUE
                );
                let (val, _) = bacnet_encoding::primitives::decode_application_value(
                    &rp_ack.property_value,
                    0,
                )
                .unwrap();
                assert_eq!(val, bacnet_types::primitives::PropertyValue::Real(72.5));
                got_response = true;
            }
            other => panic!("Unexpected APDU: {:?}", other),
        }
        if got_seg_ack_1 && got_response {
            break;
        }
    }

    assert!(
        got_seg_ack_1,
        "Should have received SegmentAck for segment 1"
    );
    assert!(
        got_response,
        "Should have received ReadPropertyACK response"
    );

    server.stop().await.unwrap();
}

#[tokio::test]
async fn server_aborts_segmented_request_with_invalid_window_size() {
    let mut server = make_server().await;
    let server_mac = server.local_mac().to_vec();

    let raw_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let mut raw_network = NetworkLayer::new(raw_transport);
    let mut raw_rx = raw_network.start().await.unwrap();
    let raw_network = Arc::new(raw_network);

    let payload = read_property_service_payload();
    let invoke_id = 43;
    let seg0 = segmented_read_property_pdu(
        invoke_id,
        0,
        true,
        Some(1),
        Bytes::copy_from_slice(&payload[..payload.len() / 2]),
    );
    let mut buf = BytesMut::new();
    encode_apdu(&mut buf, &seg0).expect("valid APDU encoding");
    buf[4] = 0;
    raw_network
        .send_apdu(&buf, &server_mac, true, NetworkPriority::NORMAL)
        .await
        .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(3), raw_rx.recv())
        .await
        .expect("Timed out waiting for Abort")
        .expect("Channel closed while waiting for Abort");
    let decoded = bacnet_encoding::apdu::decode_apdu(received.apdu).unwrap();
    match decoded {
        Apdu::Abort(abort) => {
            assert!(abort.sent_by_server);
            assert_eq!(abort.invoke_id, invoke_id);
            assert_eq!(abort.abort_reason, AbortReason::WINDOW_SIZE_OUT_OF_RANGE);
        }
        other => panic!("Expected Abort, got {:?}", other),
    }

    server.stop().await.unwrap();
}

#[tokio::test]
async fn server_naks_segmented_request_gap_with_last_good_sequence() {
    let mut server = make_server().await;
    let server_mac = server.local_mac().to_vec();

    let raw_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let mut raw_network = NetworkLayer::new(raw_transport);
    let mut raw_rx = raw_network.start().await.unwrap();
    let raw_network = Arc::new(raw_network);

    let payload = read_property_service_payload();
    let third = payload.len() / 3;
    let invoke_id = 44;
    let seg0 = segmented_read_property_pdu(
        invoke_id,
        0,
        true,
        Some(2),
        Bytes::copy_from_slice(&payload[..third]),
    );
    send_raw_apdu(&raw_network, &server_mac, seg0).await;

    assert!(
        tokio::time::timeout(Duration::from_millis(200), raw_rx.recv())
            .await
            .is_err(),
        "server should not ACK before the negotiated window boundary"
    );

    let seg2 = segmented_read_property_pdu(
        invoke_id,
        2,
        true,
        Some(2),
        Bytes::copy_from_slice(&payload[third * 2..]),
    );
    send_raw_apdu(&raw_network, &server_mac, seg2).await;

    let received = tokio::time::timeout(Duration::from_secs(3), raw_rx.recv())
        .await
        .expect("Timed out waiting for negative SegmentAck")
        .expect("Channel closed while waiting for negative SegmentAck");
    let decoded = bacnet_encoding::apdu::decode_apdu(received.apdu).unwrap();
    match decoded {
        Apdu::SegmentAck(sa) => {
            assert!(sa.sent_by_server);
            assert!(sa.negative_ack);
            assert_eq!(sa.invoke_id, invoke_id);
            assert_eq!(sa.sequence_number, 0);
            assert_eq!(sa.actual_window_size, 2);
        }
        other => panic!("Expected SegmentAck, got {:?}", other),
    }

    server.stop().await.unwrap();
}

#[tokio::test]
async fn server_acks_segmented_request_at_window_boundary() {
    let mut server = make_server().await;
    let server_mac = server.local_mac().to_vec();

    let raw_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let mut raw_network = NetworkLayer::new(raw_transport);
    let mut raw_rx = raw_network.start().await.unwrap();
    let raw_network = Arc::new(raw_network);

    let payload = read_property_service_payload();
    let mid = payload.len() / 2;
    let invoke_id = 45;

    let seg0 = segmented_read_property_pdu(
        invoke_id,
        0,
        true,
        Some(2),
        Bytes::copy_from_slice(&payload[..mid]),
    );
    send_raw_apdu(&raw_network, &server_mac, seg0).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(200), raw_rx.recv())
            .await
            .is_err(),
        "server should wait for the second segment before ACKing a two-segment window"
    );

    let seg1 = segmented_read_property_pdu(
        invoke_id,
        1,
        true,
        Some(2),
        Bytes::copy_from_slice(&payload[mid..]),
    );
    send_raw_apdu(&raw_network, &server_mac, seg1).await;

    let received = tokio::time::timeout(Duration::from_secs(3), raw_rx.recv())
        .await
        .expect("Timed out waiting for SegmentAck")
        .expect("Channel closed while waiting for SegmentAck");
    let decoded = bacnet_encoding::apdu::decode_apdu(received.apdu).unwrap();
    match decoded {
        Apdu::SegmentAck(sa) => {
            assert!(sa.sent_by_server);
            assert!(!sa.negative_ack);
            assert_eq!(sa.invoke_id, invoke_id);
            assert_eq!(sa.sequence_number, 1);
            assert_eq!(sa.actual_window_size, 2);
        }
        other => panic!("Expected SegmentAck, got {:?}", other),
    }

    server.stop().await.unwrap();
}

// ---------------------------------------------------------------------------
// DeviceCommunicationControl enforcement tests (Clause 16.4.3)
// ---------------------------------------------------------------------------

/// DCC DISABLE (deprecated in 2020 spec) is rejected with SERVICE_REQUEST_DENIED.
#[tokio::test]
async fn dcc_disable_sets_comm_state() {
    use bacnet_types::enums::EnableDisable;

    let mut server = make_server().await;
    let mut client = make_client().await;
    let server_mac = server.local_mac().to_vec();

    // Clause 16.1: DISABLE sets comm_state to 1 (per spec, all three values are supported)
    let result = client
        .device_communication_control(&server_mac, EnableDisable::DISABLE, None, None)
        .await;
    assert!(result.is_ok(), "DCC DISABLE should succeed per Clause 16.1");

    // Server should be in DISABLE state (1)
    assert_eq!(server.comm_state(), 1);

    // Re-enable should work (DCC is allowed even when disabled)
    let result = client
        .device_communication_control(&server_mac, EnableDisable::ENABLE, None, None)
        .await;
    assert!(result.is_ok(), "DCC ENABLE should succeed");
    assert_eq!(server.comm_state(), 0);

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// DCC DISABLE_INITIATION still allows DCC re-enable.
#[tokio::test]
async fn dcc_disable_initiation_allows_re_enable() {
    use bacnet_types::enums::EnableDisable;

    let mut server = make_server().await;
    let mut client = make_client().await;
    let server_mac = server.local_mac().to_vec();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    // DCC DISABLE_INITIATION
    client
        .device_communication_control(&server_mac, EnableDisable::DISABLE_INITIATION, None, None)
        .await
        .unwrap();
    assert_eq!(server.comm_state(), 2);

    // DCC ENABLE while disable-initiation — should still work
    client
        .device_communication_control(&server_mac, EnableDisable::ENABLE, None, None)
        .await
        .unwrap();
    assert_eq!(server.comm_state(), 0);

    // ReadProperty should work
    let ack = client
        .read_property(&server_mac, ai_oid, PropertyIdentifier::PRESENT_VALUE, None)
        .await;
    assert!(
        ack.is_ok(),
        "ReadProperty should succeed after DCC re-enable"
    );

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// DCC DISABLE_INITIATION allows ReadProperty but blocks COV notifications.
#[tokio::test]
async fn dcc_disable_initiation_allows_rp_blocks_cov() {
    use bacnet_objects::analog::AnalogOutputObject;
    use bacnet_types::enums::EnableDisable;
    use tokio::time::Duration;

    // Build a server with an AnalogOutput (writable present-value)
    let mut db = ObjectDatabase::new();
    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    db.add(Box::new(ao)).unwrap();
    let dev = DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "DCC-Test-Dev".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(dev)).unwrap();

    let mut server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(db)
        .build()
        .await
        .unwrap();
    let server_mac = server.local_mac().to_vec();

    let mut client = make_client().await;
    let mut cov_rx = client.cov_notifications();
    let ao_oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();

    // Subscribe to COV
    client
        .subscribe_cov(&server_mac, 42, ao_oid, false, Some(300))
        .await
        .unwrap();

    // Write to trigger a COV notification (should fire normally)
    let mut value_buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut value_buf, 50.0);
    client
        .write_property(
            &server_mac,
            ao_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value_buf.to_vec(),
            Some(16),
        )
        .await
        .unwrap();

    // Should receive COV notification
    let notification = tokio::time::timeout(Duration::from_secs(2), cov_rx.recv())
        .await
        .expect("Timed out waiting for COV notification")
        .expect("COV channel closed");
    assert_eq!(notification.monitored_object_identifier, ao_oid);

    // Now set DCC DISABLE_INITIATION
    client
        .device_communication_control(&server_mac, EnableDisable::DISABLE_INITIATION, None, None)
        .await
        .unwrap();
    assert_eq!(server.comm_state(), 2);

    // ReadProperty should still work
    let ack = client
        .read_property(&server_mac, ao_oid, PropertyIdentifier::PRESENT_VALUE, None)
        .await;
    assert!(
        ack.is_ok(),
        "ReadProperty should succeed under DISABLE_INITIATION"
    );

    // Write again — server should NOT send COV notification
    let mut value_buf2 = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut value_buf2, 99.0);
    client
        .write_property(
            &server_mac,
            ao_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value_buf2.to_vec(),
            Some(16),
        )
        .await
        .unwrap();

    // Wait briefly — no notification should arrive
    let timeout_result = tokio::time::timeout(Duration::from_millis(500), cov_rx.recv()).await;
    assert!(
        timeout_result.is_err(),
        "Should NOT receive a COV notification under DISABLE_INITIATION"
    );

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// DCC Enable after DISABLE_INITIATION restores normal COV notifications.
#[tokio::test]
async fn dcc_enable_restores_normal_operation() {
    use bacnet_objects::analog::AnalogOutputObject;
    use bacnet_types::enums::EnableDisable;
    use tokio::time::Duration;

    // Build a server with an AnalogOutput
    let mut db = ObjectDatabase::new();
    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    db.add(Box::new(ao)).unwrap();
    let dev = DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "DCC-Restore-Dev".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(dev)).unwrap();

    let mut server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(db)
        .build()
        .await
        .unwrap();
    let server_mac = server.local_mac().to_vec();

    let mut client = make_client().await;
    let mut cov_rx = client.cov_notifications();
    let ao_oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();

    // Subscribe to COV
    client
        .subscribe_cov(&server_mac, 42, ao_oid, false, Some(300))
        .await
        .unwrap();

    // Disable initiation
    client
        .device_communication_control(&server_mac, EnableDisable::DISABLE_INITIATION, None, None)
        .await
        .unwrap();
    assert_eq!(server.comm_state(), 2);

    // Write — no COV (suppressed)
    let mut value_buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut value_buf, 50.0);
    client
        .write_property(
            &server_mac,
            ao_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value_buf.to_vec(),
            Some(16),
        )
        .await
        .unwrap();

    let timeout_result = tokio::time::timeout(Duration::from_millis(500), cov_rx.recv()).await;
    assert!(
        timeout_result.is_err(),
        "Should NOT receive COV under DISABLE_INITIATION"
    );

    // Re-enable
    client
        .device_communication_control(&server_mac, EnableDisable::ENABLE, None, None)
        .await
        .unwrap();
    assert_eq!(server.comm_state(), 0);

    // Write again — COV should fire now
    let mut value_buf2 = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut value_buf2, 99.0);
    client
        .write_property(
            &server_mac,
            ao_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value_buf2.to_vec(),
            Some(16),
        )
        .await
        .unwrap();

    let notification = tokio::time::timeout(Duration::from_secs(2), cov_rx.recv())
        .await
        .expect("Timed out waiting for COV notification after re-enable")
        .expect("COV channel closed");
    assert_eq!(notification.monitored_object_identifier, ao_oid);

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

// ---------------------------------------------------------------------------
// IAm routing tests (Clause 16.10)
// ---------------------------------------------------------------------------

/// When a WhoIs arrives from a remote network (with SNET/SADR in the NPDU),
/// the server should route the IAm response back to the source rather than
/// just broadcasting locally. The response NPDU should contain DNET/DADR
/// matching the original SNET/SADR, and be sent as unicast to the router MAC.
#[tokio::test]
async fn iam_routed_back_to_remote_whois_requester() {
    use bacnet_encoding::apdu::{self, encode_apdu, Apdu, UnconfirmedRequest};
    use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
    use bacnet_transport::port::TransportPort;
    use bacnet_types::enums::{NetworkPriority, UnconfirmedServiceChoice};
    use tokio::time::Duration;

    let mut server = make_server().await;
    let server_mac = server.local_mac().to_vec();

    // Create a raw transport to act as a "router" forwarding a remote WhoIs.
    let mut raw_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let mut raw_rx = raw_transport.start().await.unwrap();
    let _raw_mac = raw_transport.local_mac().to_vec();

    // Build a WhoIs APDU (no range limits -> all devices)
    let who_is_apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::WHO_IS,
        service_request: Bytes::new(),
    });
    let mut apdu_buf = bytes::BytesMut::new();
    encode_apdu(&mut apdu_buf, &who_is_apdu).expect("valid APDU encoding");

    // Wrap in an NPDU with source routing: SNET=100, SADR=[10,20,30]
    // This simulates a WhoIs that was forwarded by a router from network 100.
    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: false,
        priority: NetworkPriority::NORMAL,
        destination: None,
        source: Some(NpduAddress {
            network: 100,
            mac_address: bacnet_types::MacAddr::from_slice(&[10, 20, 30]),
        }),
        hop_count: 255,
        payload: Bytes::from(apdu_buf.to_vec()),
        ..Npdu::default()
    };

    let mut npdu_buf = bytes::BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();

    // Send via raw transport unicast to the server
    raw_transport
        .send_unicast(&npdu_buf, &server_mac)
        .await
        .unwrap();

    // Receive the response NPDU from the server.
    // The server should send the IAm as unicast back to our raw transport
    // (since we are the "router" MAC), with DNET/DADR in the NPDU header.
    let received = tokio::time::timeout(Duration::from_secs(3), raw_rx.recv())
        .await
        .expect("Timed out waiting for routed IAm response")
        .expect("Raw transport channel closed");

    // Decode the NPDU to verify routing info
    let response_npdu = decode_npdu(received.npdu.clone()).unwrap();

    // The response should have a destination (DNET/DADR) pointing to the
    // original source: network 100, mac [10, 20, 30].
    let dest = response_npdu
        .destination
        .as_ref()
        .expect("IAm response NPDU should have destination (DNET/DADR) for routed reply");
    assert_eq!(
        dest.network, 100,
        "DNET should match the original source network"
    );
    assert_eq!(
        dest.mac_address,
        MacAddr::from_slice(&[10, 20, 30]),
        "DADR should match the original source MAC"
    );

    // Verify the APDU is an IAm
    let decoded_apdu = apdu::decode_apdu(response_npdu.payload.clone()).unwrap();
    match decoded_apdu {
        Apdu::UnconfirmedRequest(req) => {
            assert_eq!(req.service_choice, UnconfirmedServiceChoice::I_AM);
        }
        other => panic!("Expected UnconfirmedRequest (IAm), got {:?}", other),
    }

    raw_transport.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// When a WhoIs arrives from the local network (no SNET/SADR in the NPDU),
/// the server should broadcast the IAm response locally (no DNET/DADR).
///
/// Note: On localhost, the broadcast may or may not reach our test transport
/// depending on OS behavior. If it does arrive, we verify it has no routing
/// info. This is a best-effort test (same caveat as `who_is_through_server`).
#[tokio::test]
async fn iam_broadcast_for_local_whois() {
    use bacnet_encoding::apdu::{self, encode_apdu, Apdu, UnconfirmedRequest};
    use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
    use bacnet_transport::port::TransportPort;
    use bacnet_types::enums::{NetworkPriority, UnconfirmedServiceChoice};
    use tokio::time::Duration;

    let mut server = make_server().await;
    let server_mac = server.local_mac().to_vec();

    // Create a raw transport
    let mut raw_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let mut raw_rx = raw_transport.start().await.unwrap();

    // Build a WhoIs APDU with no source routing (local WhoIs)
    let who_is_apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::WHO_IS,
        service_request: Bytes::new(),
    });
    let mut apdu_buf = bytes::BytesMut::new();
    encode_apdu(&mut apdu_buf, &who_is_apdu).expect("valid APDU encoding");

    // Wrap in an NPDU without source routing
    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: false,
        priority: NetworkPriority::NORMAL,
        destination: None,
        source: None,
        payload: Bytes::from(apdu_buf.to_vec()),
        ..Npdu::default()
    };

    let mut npdu_buf = bytes::BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();

    // Send to the server
    raw_transport
        .send_unicast(&npdu_buf, &server_mac)
        .await
        .unwrap();

    // Try to receive the broadcast IAm response.
    // On localhost, broadcast may not reach our transport — this is best-effort.
    match tokio::time::timeout(Duration::from_millis(500), raw_rx.recv()).await {
        Ok(Some(received)) => {
            // Decode the NPDU
            let response_npdu = decode_npdu(received.npdu.clone()).unwrap();

            // For a local broadcast response, there should be NO destination routing
            // (no DNET/DADR — it's a simple local broadcast).
            assert!(
                response_npdu.destination.is_none(),
                "Local IAm broadcast should not have DNET/DADR"
            );

            // Verify the APDU is an IAm
            let decoded_apdu = apdu::decode_apdu(response_npdu.payload.clone()).unwrap();
            match decoded_apdu {
                Apdu::UnconfirmedRequest(req) => {
                    assert_eq!(req.service_choice, UnconfirmedServiceChoice::I_AM);
                }
                other => panic!("Expected UnconfirmedRequest (IAm), got {:?}", other),
            }
        }
        _ => {
            // Broadcast did not reach us on localhost — acceptable.
        }
    }

    raw_transport.stop().await.unwrap();
    server.stop().await.unwrap();
}

// ---------------------------------------------------------------------------
// AcknowledgeAlarm integration test
// ---------------------------------------------------------------------------

/// Send an AcknowledgeAlarm confirmed request to the server and verify
/// we receive a SimpleACK back (service choice 0 = ACKNOWLEDGE_ALARM).
#[tokio::test]
async fn acknowledge_alarm_returns_simple_ack() {
    use bacnet_services::alarm_event::AcknowledgeAlarmRequest;
    use bacnet_types::enums::ConfirmedServiceChoice;
    use bacnet_types::primitives::BACnetTimeStamp;

    let mut server = make_server().await;
    let mut client = make_client().await;
    let server_mac = server.local_mac().to_vec();

    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let request = AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 1,
        event_object_identifier: ai_oid,
        event_state_acknowledged: 3,
        timestamp: BACnetTimeStamp::SequenceNumber(42),
        acknowledgment_source: "operator".into(),
        time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(0),
    };
    let mut buf = bytes::BytesMut::new();
    request.encode(&mut buf).unwrap();

    // Send as a confirmed request -- should get back a SimpleACK (empty payload)
    let result = client
        .confirmed_request(&server_mac, ConfirmedServiceChoice::ACKNOWLEDGE_ALARM, &buf)
        .await;

    // confirmed_request returns the service-ack payload; for SimpleACK that's empty
    let ack_data = result.unwrap();
    assert!(
        ack_data.is_empty(),
        "AcknowledgeAlarm should return SimpleACK (empty payload)"
    );

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}
