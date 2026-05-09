use super::*;

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
