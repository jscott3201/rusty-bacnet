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
fn rpm_handler_required_vs_optional() {
    let db = make_db_with_ai();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

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
