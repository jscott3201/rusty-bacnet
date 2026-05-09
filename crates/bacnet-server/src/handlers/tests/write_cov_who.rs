use super::*;

#[test]
fn wpm_handler_success() {
    let mut db = ObjectDatabase::new();
    let bv = bacnet_objects::binary::BinaryValueObject::new(1, "BV-1").unwrap();
    db.add(Box::new(bv)).unwrap();

    let oid = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap();

    let mut value_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut value_buf, 1);

    use bacnet_services::common::BACnetPropertyValue;
    use bacnet_services::wpm::WriteAccessSpecification;

    let request = bacnet_services::wpm::WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: value_buf.to_vec(),
                priority: None,
            }],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    handle_write_property_multiple(&mut db, &buf).unwrap();

    let obj = db.get(&oid).unwrap();
    let val = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, bacnet_types::primitives::PropertyValue::Enumerated(1));
}

#[test]
fn subscribe_cov_handler_success() {
    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    let mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let request = SubscribeCOVRequest {
        subscriber_process_identifier: 1,
        monitored_object_identifier: oid,
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    handle_subscribe_cov(&mut table, &db, &mac, &buf).unwrap();
    assert_eq!(table.len(), 1);
}

#[test]
fn subscribe_cov_unknown_object_fails() {
    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    let mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 99).unwrap();

    let request = SubscribeCOVRequest {
        subscriber_process_identifier: 1,
        monitored_object_identifier: oid,
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    assert!(handle_subscribe_cov(&mut table, &db, &mac, &buf).is_err());
    assert!(table.is_empty());
}

#[test]
fn subscribe_cov_cancellation() {
    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    let mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    // First subscribe
    let request = SubscribeCOVRequest {
        subscriber_process_identifier: 1,
        monitored_object_identifier: oid,
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    handle_subscribe_cov(&mut table, &db, &mac, &buf).unwrap();
    assert_eq!(table.len(), 1);

    // Then cancel
    let cancel = SubscribeCOVRequest {
        subscriber_process_identifier: 1,
        monitored_object_identifier: oid,
        issue_confirmed_notifications: None,
        lifetime: None,
    };
    let mut buf = BytesMut::new();
    cancel.encode(&mut buf);
    handle_subscribe_cov(&mut table, &db, &mac, &buf).unwrap();
    assert!(table.is_empty());
}

#[test]
fn who_has_by_id_found() {
    let db = make_db_with_ai();
    let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let request = WhoHasRequest {
        low_limit: None,
        high_limit: None,
        object: WhoHasObject::Identifier(ai_oid),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let result = handle_who_has(&db, &buf, device_oid).unwrap();
    assert!(result.is_some());
    let i_have = result.unwrap();
    assert_eq!(i_have.object_identifier, ai_oid);
    assert_eq!(i_have.object_name, "AI-1");
}

#[test]
fn who_has_by_name_found() {
    let db = make_db_with_ai();
    let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();

    let request = WhoHasRequest {
        low_limit: None,
        high_limit: None,
        object: WhoHasObject::Name("AI-1".into()),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let result = handle_who_has(&db, &buf, device_oid).unwrap();
    assert!(result.is_some());
}

#[test]
fn who_has_not_found() {
    let db = make_db_with_ai();
    let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    let missing_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 99).unwrap();

    let request = WhoHasRequest {
        low_limit: None,
        high_limit: None,
        object: WhoHasObject::Identifier(missing_oid),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let result = handle_who_has(&db, &buf, device_oid).unwrap();
    assert!(result.is_none());
}

#[test]
fn who_has_out_of_range() {
    let db = make_db_with_ai();
    let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let request = WhoHasRequest {
        low_limit: Some(100),
        high_limit: Some(200),
        object: WhoHasObject::Identifier(ai_oid),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let result = handle_who_has(&db, &buf, device_oid).unwrap();
    assert!(result.is_none()); // device instance 1 not in [100, 200]
}

#[test]
fn delete_object_success() {
    let mut db = make_db_with_ai();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let request = bacnet_services::object_mgmt::DeleteObjectRequest {
        object_identifier: oid,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    handle_delete_object(&mut db, &buf).unwrap();
    assert!(db.get(&oid).is_none());
}

#[test]
fn delete_object_unknown_fails() {
    let mut db = make_db_with_ai();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 99).unwrap();

    let request = bacnet_services::object_mgmt::DeleteObjectRequest {
        object_identifier: oid,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    assert!(handle_delete_object(&mut db, &buf).is_err());
}

#[test]
fn delete_device_object_fails() {
    let mut db = ObjectDatabase::new();
    let device = bacnet_objects::device::DeviceObject::new(bacnet_objects::device::DeviceConfig {
        instance: 1,
        name: "Dev".into(),
        ..Default::default()
    })
    .unwrap();
    db.add(Box::new(device)).unwrap();

    let oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    let request = bacnet_services::object_mgmt::DeleteObjectRequest {
        object_identifier: oid,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    assert!(handle_delete_object(&mut db, &buf).is_err());
}
