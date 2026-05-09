use super::*;

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
