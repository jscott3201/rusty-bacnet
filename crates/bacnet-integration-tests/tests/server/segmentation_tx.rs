use super::*;

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
