use super::*;

#[test]
fn wpm_handler_unknown_object_fails() {
    let mut db = make_db_with_ai();
    let oid = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 99).unwrap();

    use bacnet_services::common::BACnetPropertyValue;
    use bacnet_services::wpm::WriteAccessSpecification;

    let request = bacnet_services::wpm::WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: vec![0x91, 0x01],
                priority: None,
            }],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    assert!(handle_write_property_multiple(&mut db, &buf).is_err());
}

#[test]
fn wpm_handler_atomicity_rollback() {
    // Write two properties: first succeeds (HIGH_LIMIT), second fails (read-only OBJECT_TYPE).
    // Verify HIGH_LIMIT is rolled back to its original value.
    let mut db = make_db_with_ai();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let original_hl = match db
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::HIGH_LIMIT, None)
        .unwrap()
    {
        PropertyValue::Real(v) => v,
        _ => panic!("expected Real"),
    };

    let mut hl_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut hl_buf, 999.0);
    let mut ot_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut ot_buf, 0);

    use bacnet_services::common::BACnetPropertyValue;
    use bacnet_services::wpm::WriteAccessSpecification;

    let request = bacnet_services::wpm::WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::HIGH_LIMIT,
                    property_array_index: None,
                    value: hl_buf.to_vec(),
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_TYPE,
                    property_array_index: None,
                    value: ot_buf.to_vec(),
                    priority: None,
                },
            ],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    // Should fail because OBJECT_TYPE is read-only
    assert!(handle_write_property_multiple(&mut db, &buf).is_err());

    // HIGH_LIMIT should be rolled back to original
    let after_hl = match db
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::HIGH_LIMIT, None)
        .unwrap()
    {
        PropertyValue::Real(v) => v,
        _ => panic!("expected Real"),
    };
    assert_eq!(
        original_hl, after_hl,
        "HIGH_LIMIT should be rolled back after failed WPM"
    );
}
#[test]
fn create_object_by_type_assigns_next_instance() {
    let mut db = make_db_with_device_and_ai();
    let req = CreateObjectRequest {
        object_specifier: ObjectSpecifier::Type(ObjectType::ANALOG_INPUT),
        list_of_initial_values: vec![],
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    let result = handle_create_object(&mut db, &buf, &mut ack_buf);
    assert!(result.is_ok());
    // Should now have 3 objects (Device + AI-1 + new AI)
    assert_eq!(db.len(), 3);
    // The new AI should have instance 2 (since 1 is taken)
    let ai2_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap();
    assert!(db.get(&ai2_oid).is_some());
}

#[test]
fn create_object_by_identifier() {
    let mut db = make_db_with_device_and_ai();
    let target_oid = ObjectIdentifier::new(ObjectType::BINARY_INPUT, 99).unwrap();
    let req = CreateObjectRequest {
        object_specifier: ObjectSpecifier::Identifier(target_oid),
        list_of_initial_values: vec![],
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    let result = handle_create_object(&mut db, &buf, &mut ack_buf);
    assert!(result.is_ok());
    assert!(db.get(&target_oid).is_some());
}

#[test]
fn create_object_duplicate_fails() {
    let mut db = make_db_with_device_and_ai();
    let existing_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let req = CreateObjectRequest {
        object_specifier: ObjectSpecifier::Identifier(existing_oid),
        list_of_initial_values: vec![],
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    let result = handle_create_object(&mut db, &buf, &mut ack_buf);
    assert!(result.is_err());
}

#[test]
fn create_unsupported_type_fails() {
    let mut db = make_db_with_device_and_ai();
    let req = CreateObjectRequest {
        object_specifier: ObjectSpecifier::Type(ObjectType::DEVICE),
        list_of_initial_values: vec![],
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    let result = handle_create_object(&mut db, &buf, &mut ack_buf);
    assert!(result.is_err());
}

#[test]
fn create_object_with_initial_values() {
    let mut db = make_db_with_device_and_ai();
    let mut desc_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut desc_buf, "Test AI").unwrap();

    let req = CreateObjectRequest {
        object_specifier: ObjectSpecifier::Type(ObjectType::ANALOG_INPUT),
        list_of_initial_values: vec![bacnet_services::common::BACnetPropertyValue {
            property_identifier: PropertyIdentifier::DESCRIPTION,
            property_array_index: None,
            value: desc_buf.to_vec(),
            priority: None,
        }],
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    handle_create_object(&mut db, &buf, &mut ack_buf).unwrap();
    let (pv, _) = bacnet_encoding::primitives::decode_application_value(&ack_buf, 0).unwrap();
    let created_oid = match pv {
        PropertyValue::ObjectIdentifier(oid) => oid,
        other => panic!("expected ObjectIdentifier, got {other:?}"),
    };

    let obj = db.get(&created_oid).unwrap();
    let desc = obj
        .read_property(PropertyIdentifier::DESCRIPTION, None)
        .unwrap();
    match desc {
        PropertyValue::CharacterString(s) => assert_eq!(s, "Test AI"),
        other => panic!("expected CharacterString, got {other:?}"),
    }
}

#[test]
fn create_object_bad_initial_value_rolls_back() {
    let mut db = make_db_with_device_and_ai();
    let before_count = db.len();

    // Try to write OBJECT_TYPE (read-only) as an initial value
    let mut ot_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut ot_buf, 99);

    let req = CreateObjectRequest {
        object_specifier: ObjectSpecifier::Type(ObjectType::BINARY_INPUT),
        list_of_initial_values: vec![bacnet_services::common::BACnetPropertyValue {
            property_identifier: PropertyIdentifier::OBJECT_TYPE,
            property_array_index: None,
            value: ot_buf.to_vec(),
            priority: None,
        }],
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf);

    assert!(handle_create_object(&mut db, &buf, &mut BytesMut::new()).is_err());
    assert_eq!(
        db.len(),
        before_count,
        "object should be removed on failure"
    );
}

// -----------------------------------------------------------------------
// AcknowledgeAlarm handler tests
// -----------------------------------------------------------------------

#[test]
fn acknowledge_alarm_success() {
    let mut db = make_db_with_ai();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let request = AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 1,
        event_object_identifier: oid,
        event_state_acknowledged: 3,
        timestamp: BACnetTimeStamp::SequenceNumber(42),
        acknowledgment_source: "operator".into(),
        time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(0),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    handle_acknowledge_alarm(&mut db, &buf).unwrap();
}

#[test]
fn acknowledge_alarm_unknown_object_fails() {
    let mut db = make_db_with_ai();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 99).unwrap();

    let request = AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 1,
        event_object_identifier: oid,
        event_state_acknowledged: 3,
        timestamp: BACnetTimeStamp::SequenceNumber(42),
        acknowledgment_source: "operator".into(),
        time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(0),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let err = handle_acknowledge_alarm(&mut db, &buf).unwrap_err();
    match err {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::OBJECT.to_raw() as u32);
            assert_eq!(code, ErrorCode::UNKNOWN_OBJECT.to_raw() as u32);
        }
        other => panic!("expected Protocol error, got: {other:?}"),
    }
}
