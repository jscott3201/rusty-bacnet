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
fn wpm_handler_commits_successful_prefix() {
    // HIGH_LIMIT succeeds before the read-only OBJECT_TYPE failure.
    let mut db = make_db_with_ai();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

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

    let after_hl = match db
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::HIGH_LIMIT, None)
        .unwrap()
    {
        PropertyValue::Real(v) => v,
        _ => panic!("expected Real"),
    };
    assert_eq!(after_hl, 999.0, "the successful prefix stays committed");
}

#[test]
fn wpm_prefix_commit_keeps_out_of_service_transition() {
    use bacnet_services::common::BACnetPropertyValue;
    use bacnet_services::wpm::WriteAccessSpecification;
    use bacnet_types::enums::Reliability;

    let mut db = make_db_with_ai();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let obj = db.get_mut(&oid).unwrap();
    obj.set_reliability_internal(Reliability::OVER_RANGE.to_raw())
        .unwrap();
    obj.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    obj.write_property(
        PropertyIdentifier::RELIABILITY,
        None,
        PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw()),
        None,
    )
    .unwrap();

    let mut oos_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_boolean(&mut oos_buf, false);
    let mut object_type_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut object_type_buf, 0);
    let request = bacnet_services::wpm::WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OUT_OF_SERVICE,
                    property_array_index: None,
                    value: oos_buf.to_vec(),
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_TYPE,
                    property_array_index: None,
                    value: object_type_buf.to_vec(),
                    priority: None,
                },
            ],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    assert!(handle_write_property_multiple(&mut db, &buf).is_err());
    let obj = db.get_mut(&oid).unwrap();
    assert_eq!(
        obj.read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
            .unwrap(),
        PropertyValue::Boolean(false)
    );
    assert_eq!(
        obj.read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::OVER_RANGE.to_raw()),
        "the successful OOS exit restores evaluated Reliability"
    );

    obj.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    assert_eq!(
        obj.read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::OVER_RANGE.to_raw()),
        "a repeated in-service write keeps evaluated Reliability"
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

/// Build a database with one commandable AnalogOutput whose priority-8 slot
/// holds an active command, so a `PRESENT_VALUE` write at priority 8 has a real
/// slot to overwrite. Returns the db and the AO's object identifier.
fn make_db_with_commandable_ao() -> (ObjectDatabase, ObjectIdentifier) {
    use bacnet_objects::analog::AnalogOutputObject;
    let mut db = ObjectDatabase::new();
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    // Establish an active command at priority 8.
    ao.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(50.0),
        Some(8),
    )
    .unwrap();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
    db.add(Box::new(ao)).unwrap();
    (db, oid)
}

/// Read a single `PRIORITY_ARRAY` slot as `Option<f32>` (`None` if relinquished).
/// Multi-state objects store `Unsigned` slots, so coerce those to `f32` for
/// uniform comparison across commandable object kinds.
fn priority_slot(db: &ObjectDatabase, oid: &ObjectIdentifier, slot: u32) -> Option<f32> {
    match db
        .get(oid)
        .unwrap()
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(slot))
        .unwrap()
    {
        PropertyValue::Real(v) => Some(v),
        PropertyValue::Unsigned(v) => Some(v as f32),
        PropertyValue::Null => None,
        other => panic!("expected Real, Unsigned, or Null for priority slot {slot}, got {other:?}"),
    }
}

#[test]
fn wpm_prefix_commit_keeps_commandable_priority_slot() {
    let (mut db, oid) = make_db_with_commandable_ao();

    assert_eq!(priority_slot(&db, &oid, 8), Some(50.0));
    assert_eq!(priority_slot(&db, &oid, 16), None);

    // First spec: overwrite the priority-8 command. Second spec: OBJECT_TYPE is
    // read-only and fails after the first write commits.
    let mut pv_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut pv_buf, 99.0);
    let mut ot_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut ot_buf, 0);

    use bacnet_services::common::BACnetPropertyValue;
    use bacnet_services::wpm::WriteAccessSpecification;
    let request = bacnet_services::wpm::WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: pv_buf.to_vec(),
                    priority: Some(8),
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

    assert!(handle_write_property_multiple(&mut db, &buf).is_err());

    assert_eq!(priority_slot(&db, &oid, 8), Some(99.0));
    assert_eq!(
        priority_slot(&db, &oid, 16),
        None,
        "the failed attempt does not create another command"
    );
    // Effective present value and command priority are unchanged.
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(99.0)
    );
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::CURRENT_COMMAND_PRIORITY, None)
            .unwrap(),
        PropertyValue::Unsigned(8)
    );
}

#[test]
fn wpm_prefix_commit_keeps_relinquished_priority_slot() {
    let (mut db, oid) = make_db_with_commandable_ao();
    assert_eq!(priority_slot(&db, &oid, 8), Some(50.0));

    // First spec: relinquish priority 8 (write Null). Second spec: OBJECT_TYPE
    // fails after the relinquish commits.
    let mut null_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_null(&mut null_buf);
    let mut ot_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut ot_buf, 0);

    use bacnet_services::common::BACnetPropertyValue;
    use bacnet_services::wpm::WriteAccessSpecification;
    let request = bacnet_services::wpm::WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: null_buf.to_vec(),
                    priority: Some(8),
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

    assert!(handle_write_property_multiple(&mut db, &buf).is_err());

    assert_eq!(
        priority_slot(&db, &oid, 8),
        None,
        "the successful relinquish stays committed"
    );
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(0.0)
    );
}

#[test]
fn wpm_prefix_commit_keeps_noncommandable_present_value_analoginput() {
    use bacnet_objects::analog::AnalogInputObject;
    let mut db = ObjectDatabase::new();
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    ai.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(10.0),
        None,
    )
    .unwrap();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    db.add(Box::new(ai)).unwrap();
    let pre_pv = db
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pre_pv, PropertyValue::Real(10.0));

    // First spec: write PRESENT_VALUE (succeeds, AI is OOS). Second spec:
    // OBJECT_TYPE is read-only and fails after the first write commits.
    let mut pv_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut pv_buf, 77.0);
    let mut ot_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut ot_buf, 0);

    use bacnet_services::common::BACnetPropertyValue;
    use bacnet_services::wpm::WriteAccessSpecification;
    let request = bacnet_services::wpm::WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: pv_buf.to_vec(),
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

    assert!(handle_write_property_multiple(&mut db, &buf).is_err());

    let post_pv = db
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(post_pv, PropertyValue::Real(77.0));
}

#[test]
fn wpm_prefix_commit_keeps_commandable_priority_slot_multistate_output() {
    use bacnet_objects::multistate::MultiStateOutputObject;
    let mut db = ObjectDatabase::new();
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
    // Active command at priority 8 = state 2.
    mso.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Unsigned(2),
        Some(8),
    )
    .unwrap();
    let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_OUTPUT, 1).unwrap();
    db.add(Box::new(mso)).unwrap();
    assert_eq!(priority_slot(&db, &oid, 8), Some(2.0));

    // First spec: overwrite priority 8 with state 3. Second spec: OBJECT_TYPE fails.
    let mut pv_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut pv_buf, 3);
    let mut ot_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut ot_buf, 0);

    use bacnet_services::common::BACnetPropertyValue;
    use bacnet_services::wpm::WriteAccessSpecification;
    let request = bacnet_services::wpm::WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: pv_buf.to_vec(),
                    priority: Some(8),
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

    assert!(handle_write_property_multiple(&mut db, &buf).is_err());

    assert_eq!(priority_slot(&db, &oid, 8), Some(3.0));
    assert_eq!(priority_slot(&db, &oid, 16), None);
}

#[test]
fn wpm_prefix_commit_keeps_commandable_priority_16_slot() {
    let (mut db, oid) = make_db_with_commandable_ao();
    // priority 8 holds 50.0 (from the helper); priority 16 is empty.
    assert_eq!(priority_slot(&db, &oid, 8), Some(50.0));
    assert_eq!(priority_slot(&db, &oid, 16), None);

    // First spec: write PRESENT_VALUE with no priority → slot 16 = 99.0.
    // Second spec: OBJECT_TYPE fails after the slot-16 write commits.
    let mut pv_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut pv_buf, 99.0);
    let mut ot_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut ot_buf, 0);

    use bacnet_services::common::BACnetPropertyValue;
    use bacnet_services::wpm::WriteAccessSpecification;
    let request = bacnet_services::wpm::WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: pv_buf.to_vec(),
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

    assert!(handle_write_property_multiple(&mut db, &buf).is_err());

    assert_eq!(priority_slot(&db, &oid, 16), Some(99.0));
    assert_eq!(priority_slot(&db, &oid, 8), Some(50.0));
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(50.0)
    );
}
