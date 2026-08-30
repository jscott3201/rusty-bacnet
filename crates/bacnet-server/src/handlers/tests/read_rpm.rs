use super::*;

#[test]
fn read_property_handler_success() {
    let db = make_db_with_ai();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let request = ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    handle_read_property(&db, &buf, &mut ack_buf).unwrap();
    let ack_bytes = ack_buf.to_vec();
    let ack = ReadPropertyACK::decode(&ack_bytes).unwrap();
    assert_eq!(ack.object_identifier, oid);
    assert_eq!(ack.property_identifier, PropertyIdentifier::PRESENT_VALUE);

    // Decode the value
    let (val, _) =
        bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0).unwrap();
    assert_eq!(val, bacnet_types::primitives::PropertyValue::Real(72.5));
}

fn active_cov_subscription_db() -> (ObjectDatabase, ObjectIdentifier, Vec<u8>) {
    use bacnet_objects::device::{DeviceConfig, DeviceObject};
    use bacnet_types::constructed::{
        BACnetCOVSubscription, BACnetObjectPropertyReference, BACnetRecipient,
        BACnetRecipientProcess,
    };

    let oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    let mut device = DeviceObject::new(DeviceConfig {
        instance: 1,
        name: "COV Device".into(),
        ..Default::default()
    })
    .unwrap();
    let subscription = BACnetCOVSubscription {
        recipient: BACnetRecipientProcess {
            recipient: BACnetRecipient::Device(
                ObjectIdentifier::new(ObjectType::DEVICE, 7).unwrap(),
            ),
            process_identifier: 7,
        },
        monitored_property_reference: BACnetObjectPropertyReference::new_indexed(
            ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 3).unwrap(),
            87,
            2,
        ),
        issue_confirmed_notifications: true,
        time_remaining: 300,
        cov_increment: Some(0.5),
    };

    let expected = vec![
        0x0E, 0x0E, 0x0C, 0x02, 0x00, 0x00, 0x07, 0x0F, 0x19, 0x07, 0x0F, 0x1E, 0x0C, 0x00, 0x40,
        0x00, 0x03, 0x19, 0x57, 0x29, 0x02, 0x1F, 0x29, 0x01, 0x3A, 0x01, 0x2C, 0x4C, 0x3F, 0x00,
        0x00, 0x00,
    ];
    let mut encoded = BytesMut::new();
    bacnet_encoding::constructed::encode_cov_subscription_list(
        &mut encoded,
        std::slice::from_ref(&subscription),
    );
    assert_eq!(encoded.as_ref(), expected);
    device.add_cov_subscription(subscription);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(device)).unwrap();
    (db, oid, expected)
}

#[test]
fn active_cov_subscriptions_read_property_preserves_constructed_bytes() {
    let (db, oid, expected) = active_cov_subscription_db();
    let request = ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS,
        property_array_index: None,
    };
    let mut request_buf = BytesMut::new();
    request.encode(&mut request_buf);

    let mut response_buf = BytesMut::new();
    handle_read_property(&db, &request_buf, &mut response_buf).unwrap();
    let ack = ReadPropertyACK::decode(&response_buf).unwrap();

    assert_eq!(ack.property_value, expected);
}

#[test]
fn active_cov_subscriptions_read_property_multiple_preserves_constructed_bytes() {
    use bacnet_services::common::PropertyReference;
    use bacnet_services::rpm::{ReadAccessSpecification, ReadPropertyMultipleRequest};

    let (db, oid, expected) = active_cov_subscription_db();
    let request = ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS,
                property_array_index: None,
            }],
        }],
    };
    let mut request_buf = BytesMut::new();
    request.encode(&mut request_buf);

    let mut response_buf = BytesMut::new();
    handle_read_property_multiple(&db, &request_buf, &mut response_buf).unwrap();
    let ack = ReadPropertyMultipleACK::decode(&response_buf).unwrap();
    let result = &ack.list_of_read_access_results[0].list_of_results[0];

    assert_eq!(result.property_value.as_deref(), Some(expected.as_slice()));
    assert!(result.error.is_none());
}

#[test]
fn read_property_handler_serves_multistate_event_time_stamps_count() {
    let db = make_db_with_msi();
    let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 1).unwrap();
    let request = ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::EVENT_TIME_STAMPS,
        property_array_index: Some(0),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    handle_read_property(&db, &buf, &mut ack_buf).unwrap();
    let ack = ReadPropertyACK::decode(&ack_buf.to_vec()).unwrap();
    assert_eq!(ack.property_array_index, Some(0));
    let (value, end) =
        bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0).unwrap();
    assert_eq!(value, bacnet_types::primitives::PropertyValue::Unsigned(3));
    assert_eq!(end, ack.property_value.len());
}

#[test]
fn read_property_handler_rejects_multistate_event_time_stamps_out_of_bounds_index() {
    let db = make_db_with_msi();
    let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 1).unwrap();
    let request = ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::EVENT_TIME_STAMPS,
        property_array_index: Some(4),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    match handle_read_property(&db, &buf, &mut ack_buf).unwrap_err() {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
            assert_eq!(code, ErrorCode::INVALID_ARRAY_INDEX.to_raw() as u32);
        }
        other => panic!("expected Protocol error, got: {other:?}"),
    }
}

#[test]
fn read_property_unknown_object() {
    let db = make_db_with_ai();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 99).unwrap();

    let request = ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    let result = handle_read_property(&db, &buf, &mut ack_buf);
    assert!(result.is_err());
}

#[test]
fn read_property_unknown_property() {
    let db = make_db_with_ai();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let request = ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
        property_array_index: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    let result = handle_read_property(&db, &buf, &mut ack_buf);
    assert!(result.is_err());
}

#[test]
fn write_property_handler_success() {
    let mut db = ObjectDatabase::new();
    let bv = bacnet_objects::binary::BinaryValueObject::new(1, "BV-1").unwrap();
    db.add(Box::new(bv)).unwrap();

    let oid = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap();

    // Encode write request: set present-value to active (1)
    let mut value_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut value_buf, 1);

    let request = WritePropertyRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: value_buf.to_vec(),
        priority: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    handle_write_property(&mut db, &buf).unwrap();

    // Verify the value was written
    let obj = db.get(&oid).unwrap();
    let val = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, bacnet_types::primitives::PropertyValue::Enumerated(1));
}

#[test]
fn rpm_handler_success() {
    let db = make_db_with_ai();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    use bacnet_services::common::PropertyReference;
    use bacnet_services::rpm::ReadAccessSpecification;

    let request = bacnet_services::rpm::ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid,
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
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    handle_read_property_multiple(&db, &buf, &mut ack_buf).unwrap();
    let ack_bytes = ack_buf.to_vec();
    let ack = bacnet_services::rpm::ReadPropertyMultipleACK::decode(&ack_bytes).unwrap();

    assert_eq!(ack.list_of_read_access_results.len(), 1);
    let result = &ack.list_of_read_access_results[0];
    assert_eq!(result.object_identifier, oid);
    assert_eq!(result.list_of_results.len(), 2);

    // Both should be successful
    assert!(result.list_of_results[0].property_value.is_some());
    assert!(result.list_of_results[1].property_value.is_some());

    // Verify present-value is Real(72.5)
    let (val, _) = bacnet_encoding::primitives::decode_application_value(
        result.list_of_results[0].property_value.as_ref().unwrap(),
        0,
    )
    .unwrap();
    assert_eq!(val, bacnet_types::primitives::PropertyValue::Real(72.5));
}

#[test]
fn rpm_handler_unknown_property_returns_inline_error() {
    let db = make_db_with_ai();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    use bacnet_services::common::PropertyReference;
    use bacnet_services::rpm::ReadAccessSpecification;

    let request = bacnet_services::rpm::ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: vec![
                PropertyReference {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                },
                PropertyReference {
                    property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
                    property_array_index: None,
                },
            ],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    handle_read_property_multiple(&db, &buf, &mut ack_buf).unwrap();
    let ack_bytes = ack_buf.to_vec();
    let ack = bacnet_services::rpm::ReadPropertyMultipleACK::decode(&ack_bytes).unwrap();

    let result = &ack.list_of_read_access_results[0];
    assert!(result.list_of_results[0].property_value.is_some()); // present-value ok
    assert!(result.list_of_results[1].error.is_some()); // priority-array unknown
}

#[test]
fn rpm_handler_unknown_object_returns_inline_error() {
    let db = make_db_with_ai();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 99).unwrap();

    use bacnet_services::common::PropertyReference;
    use bacnet_services::rpm::ReadAccessSpecification;

    let request = bacnet_services::rpm::ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
            }],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    handle_read_property_multiple(&db, &buf, &mut ack_buf).unwrap();
    let ack_bytes = ack_buf.to_vec();
    let ack = bacnet_services::rpm::ReadPropertyMultipleACK::decode(&ack_bytes).unwrap();

    let result = &ack.list_of_read_access_results[0];
    assert!(result.list_of_results[0].error.is_some());
}

#[test]
fn rpm_handler_all_properties_expanded() {
    let db = make_db_with_ai();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    use bacnet_services::common::PropertyReference;
    use bacnet_services::rpm::ReadAccessSpecification;

    let request = bacnet_services::rpm::ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::ALL,
                property_array_index: None,
            }],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    handle_read_property_multiple(&db, &buf, &mut ack_buf).unwrap();
    let ack_bytes = ack_buf.to_vec();
    let ack = bacnet_services::rpm::ReadPropertyMultipleACK::decode(&ack_bytes).unwrap();

    assert_eq!(ack.list_of_read_access_results.len(), 1);
    let result = &ack.list_of_read_access_results[0];
    assert_eq!(result.object_identifier, oid);

    // AnalogInputObject.property_list() returns multiple properties
    let obj = db.get(&oid).unwrap();
    let expected_props = obj.property_list();
    assert!(
        expected_props.len() > 2,
        "sanity: AI should have many properties"
    );
    assert_eq!(result.list_of_results.len(), expected_props.len());

    // Verify each result matches the expected property identifier
    for (elem, &expected_pid) in result.list_of_results.iter().zip(expected_props.iter()) {
        assert_eq!(elem.property_identifier, expected_pid);
    }

    // Verify present-value is included and correct
    let pv_elem = result
        .list_of_results
        .iter()
        .find(|e| e.property_identifier == PropertyIdentifier::PRESENT_VALUE)
        .expect("PRESENT_VALUE should be in ALL results");
    assert!(pv_elem.property_value.is_some());
}

#[test]
fn rpm_all_includes_multistate_event_history() {
    let db = make_db_with_msi();
    let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 1).unwrap();
    use bacnet_services::common::PropertyReference;
    use bacnet_services::rpm::ReadAccessSpecification;

    let request = bacnet_services::rpm::ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::ALL,
                property_array_index: None,
            }],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    let mut ack_buf = BytesMut::new();
    handle_read_property_multiple(&db, &buf, &mut ack_buf).unwrap();
    let ack = bacnet_services::rpm::ReadPropertyMultipleACK::decode(&ack_buf.to_vec()).unwrap();
    let results = &ack.list_of_read_access_results[0].list_of_results;

    let timestamps = results
        .iter()
        .find(|result| result.property_identifier == PropertyIdentifier::EVENT_TIME_STAMPS)
        .expect("EVENT_TIME_STAMPS missing from RPM ALL")
        .property_value
        .as_ref()
        .expect("EVENT_TIME_STAMPS must succeed");
    let mut offset = 0;
    for _ in 0..3 {
        let (decoded, next) =
            bacnet_encoding::primitives::decode_timestamp_choice(timestamps, offset).unwrap();
        assert_eq!(decoded, BACnetTimeStamp::SequenceNumber(0));
        offset = next;
    }
    assert_eq!(offset, timestamps.len());

    let messages = results
        .iter()
        .find(|result| result.property_identifier == PropertyIdentifier::EVENT_MESSAGE_TEXTS)
        .expect("EVENT_MESSAGE_TEXTS missing from RPM ALL")
        .property_value
        .as_ref()
        .expect("EVENT_MESSAGE_TEXTS must succeed");
    let mut offset = 0;
    for _ in 0..3 {
        let (value, next) =
            bacnet_encoding::primitives::decode_application_value(messages, offset).unwrap();
        assert_eq!(
            value,
            bacnet_types::primitives::PropertyValue::CharacterString(String::new())
        );
        offset = next;
    }
    assert_eq!(offset, messages.len());
}

#[test]
fn rpm_explicit_index_returns_one_multistate_event_message() {
    let db = make_db_with_msi();
    let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 1).unwrap();
    use bacnet_services::common::PropertyReference;
    use bacnet_services::rpm::ReadAccessSpecification;

    let request = bacnet_services::rpm::ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::EVENT_MESSAGE_TEXTS,
                property_array_index: Some(2),
            }],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    let mut ack_buf = BytesMut::new();
    handle_read_property_multiple(&db, &buf, &mut ack_buf).unwrap();
    let ack = bacnet_services::rpm::ReadPropertyMultipleACK::decode(&ack_buf.to_vec()).unwrap();
    let result = &ack.list_of_read_access_results[0].list_of_results[0];

    assert_eq!(result.property_array_index, Some(2));
    let value = result
        .property_value
        .as_ref()
        .expect("indexed EVENT_MESSAGE_TEXTS must succeed");
    let (decoded, end) = bacnet_encoding::primitives::decode_application_value(value, 0).unwrap();
    assert_eq!(
        decoded,
        bacnet_types::primitives::PropertyValue::CharacterString(String::new())
    );
    assert_eq!(end, value.len());
}

#[test]
fn rpm_handler_required_vs_optional() {
    let db = make_db_with_ai();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    assert!(
        db.get(&oid).unwrap().property_metadata().is_empty(),
        "Analog Input intentionally exercises the legacy RPM fallback"
    );

    use bacnet_services::common::PropertyReference;
    use bacnet_services::rpm::ReadAccessSpecification;

    // REQUIRED wildcard
    let req_required = bacnet_services::rpm::ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::REQUIRED,
                property_array_index: None,
            }],
        }],
    };
    let mut buf = BytesMut::new();
    req_required.encode(&mut buf);
    let mut ack_buf = BytesMut::new();
    handle_read_property_multiple(&db, &buf, &mut ack_buf).unwrap();
    let ack_bytes = ack_buf.to_vec();
    let ack = bacnet_services::rpm::ReadPropertyMultipleACK::decode(&ack_bytes).unwrap();
    let required_results = &ack.list_of_read_access_results[0].list_of_results;

    // OPTIONAL wildcard
    let req_optional = bacnet_services::rpm::ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::OPTIONAL,
                property_array_index: None,
            }],
        }],
    };
    let mut buf = BytesMut::new();
    req_optional.encode(&mut buf);
    let mut ack_buf = BytesMut::new();
    handle_read_property_multiple(&db, &buf, &mut ack_buf).unwrap();
    let ack_bytes = ack_buf.to_vec();
    let ack = bacnet_services::rpm::ReadPropertyMultipleACK::decode(&ack_bytes).unwrap();
    let optional_results = &ack.list_of_read_access_results[0].list_of_results;

    // REQUIRED must include the 4 universal properties
    let req_pids: Vec<_> = required_results
        .iter()
        .map(|r| r.property_identifier)
        .collect();
    assert!(req_pids.contains(&PropertyIdentifier::OBJECT_IDENTIFIER));
    assert!(req_pids.contains(&PropertyIdentifier::OBJECT_NAME));
    assert!(req_pids.contains(&PropertyIdentifier::OBJECT_TYPE));
    assert!(req_pids.contains(&PropertyIdentifier::PROPERTY_LIST));

    // OPTIONAL must NOT include any required properties
    let opt_pids: Vec<_> = optional_results
        .iter()
        .map(|r| r.property_identifier)
        .collect();
    for req_pid in &req_pids {
        assert!(
            !opt_pids.contains(req_pid),
            "OPTIONAL should not contain {req_pid:?}"
        );
    }

    // REQUIRED + OPTIONAL should cover ALL.
    // Note: REQUIRED may include PROPERTY_LIST (per Clause 12.11.12,
    // property_list() excludes itself, so REQUIRED can have 1 extra).
    let obj = db.get(&oid).unwrap();
    let all_pids = obj.property_list();
    let required_set: std::collections::HashSet<_> = req_pids.iter().collect();
    let optional_set: std::collections::HashSet<_> = opt_pids.iter().collect();
    for pid in all_pids.iter() {
        assert!(
            required_set.contains(pid) || optional_set.contains(pid),
            "ALL property {pid:?} missing from REQUIRED and OPTIONAL"
        );
    }
}

#[test]
fn read_property_serves_derived_services_supported() {
    // End-to-end pin for #192: the wire-level BitString a client receives for
    // Protocol_Services_Supported, through the real ReadProperty handler path.
    // Expected bytes derive from device::EXECUTED_SERVICES bits
    // {0,3-12,14-17,19,20,31-39,41,42} packed MSB-first over the full production
    // range (49 defined bits, 7 octets, 7 unused).
    let db = make_db_with_device_and_ai();
    let oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();

    let request = ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED,
        property_array_index: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    handle_read_property(&db, &buf, &mut ack_buf).unwrap();
    let ack = ReadPropertyACK::decode(&ack_buf.to_vec()).unwrap();

    let (val, _) =
        bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0).unwrap();
    assert_eq!(
        val,
        bacnet_types::primitives::PropertyValue::BitString {
            unused_bits: 7,
            data: vec![0x9F, 0xFB, 0xD8, 0x01, 0xFF, 0x44, 0x00],
        }
    );

    // The same bytes decode to the executed set by name.
    let bacnet_types::primitives::PropertyValue::BitString { data, .. } = val else {
        unreachable!()
    };
    let ss = bacnet_types::bitstring::ServicesSupported::from_bacnet(&data);
    assert!(ss.contains(bacnet_types::enums::ServiceSupported::WHO_IS));
    assert!(ss.contains(bacnet_types::enums::ServiceSupported::AUDIT_LOG_QUERY));
    assert!(!ss.contains(bacnet_types::enums::ServiceSupported::I_AM));
}
