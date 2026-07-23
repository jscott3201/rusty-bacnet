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

    let subscriptions = handle_subscribe_cov_with_initial(&mut table, &db, &mac, &buf).unwrap();
    assert_eq!(subscriptions.len(), 1);
    assert_eq!(subscriptions[0].monitored_object_identifier, oid);
    assert_eq!(subscriptions[0].monitored_property, None);
    assert_eq!(table.len(), 1);
}

#[test]
fn subscribe_cov_property_handler_returns_initial_subscription() {
    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    let mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let request = bacnet_services::cov::SubscribeCOVPropertyRequest {
        subscriber_process_identifier: 1,
        monitored_object_identifier: oid,
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
        monitored_property_identifier: PropertyIdentifier::PRESENT_VALUE,
        monitored_property_array_index: None,
        cov_increment: Some(0.5),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let subscriptions =
        handle_subscribe_cov_property_with_initial(&mut table, &db, &mac, &buf).unwrap();
    assert_eq!(subscriptions.len(), 1);
    assert_eq!(subscriptions[0].monitored_object_identifier, oid);
    assert_eq!(
        subscriptions[0].monitored_property,
        Some(PropertyIdentifier::PRESENT_VALUE)
    );
    assert_eq!(table.len(), 1);
}

#[test]
fn subscribe_cov_update_existing_entry_allowed_at_capacity() {
    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    let mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    for instance in 0..1023 {
        table.subscribe(CovSubscription {
            subscriber_mac: MacAddr::from_slice(&[10, 0, 0, (instance % 255) as u8, 0xBA, 0xC0]),
            subscriber_network: None,
            subscriber_process_identifier: instance,
            monitored_object_identifier: ObjectIdentifier::new(
                ObjectType::ANALOG_INPUT,
                1000 + instance,
            )
            .unwrap(),
            issue_confirmed_notifications: false,
            expires_at: None,
            last_notified_value: None,
            monitored_property: None,
            monitored_property_array_index: None,
            cov_increment: None,
            notification_kind: CovNotificationKind::Single,
            timestamped: false,
        });
    }

    let original = SubscribeCOVRequest {
        subscriber_process_identifier: 1,
        monitored_object_identifier: oid,
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
    };
    let mut buf = BytesMut::new();
    original.encode(&mut buf);
    handle_subscribe_cov(&mut table, &db, &mac, &buf).unwrap();
    assert_eq!(table.len(), 1024);

    let update = SubscribeCOVRequest {
        subscriber_process_identifier: 1,
        monitored_object_identifier: oid,
        issue_confirmed_notifications: Some(true),
        lifetime: Some(600),
    };
    let mut buf = BytesMut::new();
    update.encode(&mut buf);
    let subscriptions = handle_subscribe_cov_with_initial(&mut table, &db, &mac, &buf).unwrap();

    assert_eq!(subscriptions.len(), 1);
    assert!(subscriptions[0].issue_confirmed_notifications);
    assert_eq!(table.len(), 1024);
}

#[test]
fn subscribe_cov_property_multiple_handler_returns_initial_subscriptions() {
    use bacnet_services::common::PropertyReference;
    use bacnet_services::cov_multiple::{
        COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
    };

    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    let mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let request = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 1,
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
        max_notification_delay: Some(10),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: oid,
            list_of_cov_references: vec![
                COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    },
                    cov_increment: Some(0.5),
                    timestamped: false,
                },
                COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::STATUS_FLAGS,
                        property_array_index: None,
                    },
                    cov_increment: None,
                    timestamped: false,
                },
            ],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let subscriptions =
        handle_subscribe_cov_property_multiple_with_initial(&mut table, &db, &mac, &buf).unwrap();
    assert_eq!(subscriptions.len(), 2);
    assert_eq!(subscriptions[0].monitored_object_identifier, oid);
    assert_eq!(
        subscriptions[0].monitored_property,
        Some(PropertyIdentifier::PRESENT_VALUE)
    );
    assert_eq!(
        subscriptions[1].monitored_property,
        Some(PropertyIdentifier::STATUS_FLAGS)
    );
    assert!(subscriptions.iter().all(|sub| sub.expires_at.is_some()));
    assert_eq!(table.len(), 2);
}

#[test]
fn subscribe_cov_property_multiple_cancellation_removes_context_or_specs() {
    use bacnet_services::common::PropertyReference;
    use bacnet_services::cov_multiple::{
        COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
    };

    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    let mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let subscription_request = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 1,
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
        max_notification_delay: Some(10),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: oid,
            list_of_cov_references: vec![
                COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    },
                    cov_increment: Some(0.5),
                    timestamped: false,
                },
                COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::STATUS_FLAGS,
                        property_array_index: None,
                    },
                    cov_increment: None,
                    timestamped: false,
                },
            ],
        }],
    };
    let mut buf = BytesMut::new();
    subscription_request.encode(&mut buf);
    handle_subscribe_cov_property_multiple_with_initial(&mut table, &db, &mac, &buf).unwrap();
    assert_eq!(table.len(), 2);

    let cancel_present_value = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 1,
        issue_confirmed_notifications: Some(false),
        lifetime: None,
        max_notification_delay: None,
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: oid,
            list_of_cov_references: vec![COVReference {
                monitored_property: PropertyReference {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                },
                cov_increment: None,
                timestamped: false,
            }],
        }],
    };
    let mut buf = BytesMut::new();
    cancel_present_value.encode(&mut buf);
    let initial =
        handle_subscribe_cov_property_multiple_with_initial(&mut table, &db, &mac, &buf).unwrap();
    assert!(initial.is_empty());
    let remaining: Vec<_> = table
        .subscriptions_for(&oid)
        .into_iter()
        .map(|sub| sub.monitored_property)
        .collect();
    assert_eq!(remaining, vec![Some(PropertyIdentifier::STATUS_FLAGS)]);

    let cancel_context = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 1,
        issue_confirmed_notifications: Some(false),
        lifetime: None,
        max_notification_delay: None,
        list_of_cov_subscription_specifications: Vec::new(),
    };
    let mut buf = BytesMut::new();
    cancel_context.encode(&mut buf);
    let initial =
        handle_subscribe_cov_property_multiple_with_initial(&mut table, &db, &mac, &buf).unwrap();
    assert!(initial.is_empty());
    assert!(table.is_empty());
}

#[test]
fn subscribe_cov_property_multiple_rejects_invalid_service_parameters() {
    use bacnet_services::common::PropertyReference;
    use bacnet_services::cov_multiple::{
        COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
    };

    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    let mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let specs = vec![COVSubscriptionSpecification {
        monitored_object_identifier: oid,
        list_of_cov_references: vec![COVReference {
            monitored_property: PropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
            },
            cov_increment: Some(0.5),
            timestamped: false,
        }],
    }];

    for (issue_confirmed_notifications, lifetime, max_notification_delay, expected_code) in [
        (
            None,
            Some(300),
            Some(10),
            ErrorCode::MISSING_REQUIRED_PARAMETER,
        ),
        (
            Some(false),
            Some(300),
            None,
            ErrorCode::INCONSISTENT_PARAMETERS,
        ),
        (
            Some(false),
            None,
            Some(10),
            ErrorCode::INCONSISTENT_PARAMETERS,
        ),
        (Some(false), Some(0), Some(0), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            Some(false),
            Some(300),
            Some(300),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            Some(false),
            Some(4000),
            Some(3601),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
    ] {
        let request = SubscribeCOVPropertyMultipleRequest {
            subscriber_process_identifier: 1,
            issue_confirmed_notifications,
            lifetime,
            max_notification_delay,
            list_of_cov_subscription_specifications: specs.clone(),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let err = handle_subscribe_cov_property_multiple_with_initial(&mut table, &db, &mac, &buf)
            .unwrap_err();
        match err {
            Error::Protocol { class, code } => {
                assert_eq!(class, ErrorClass::SERVICES.to_raw() as u32);
                assert_eq!(code, expected_code.to_raw() as u32);
            }
            other => panic!("expected service parameter protocol error, got {other:?}"),
        }
        assert!(table.is_empty());
    }
}

#[test]
fn subscribe_cov_property_multiple_invalid_property_is_atomic() {
    use bacnet_services::common::PropertyReference;
    use bacnet_services::cov_multiple::{
        COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
    };

    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    let mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let request = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 1,
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
        max_notification_delay: Some(10),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: oid,
            list_of_cov_references: vec![
                COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    },
                    cov_increment: Some(0.5),
                    timestamped: false,
                },
                COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
                        property_array_index: None,
                    },
                    cov_increment: None,
                    timestamped: false,
                },
            ],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let err = handle_subscribe_cov_property_multiple_with_initial(&mut table, &db, &mac, &buf)
        .unwrap_err();
    match err {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
            assert_eq!(code, ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32);
        }
        other => panic!("expected UNKNOWN_PROPERTY protocol error, got {other:?}"),
    }
    assert!(table.is_empty());
}

#[test]
fn subscribe_cov_property_multiple_capacity_failure_is_atomic() {
    use bacnet_services::common::PropertyReference;
    use bacnet_services::cov_multiple::{
        COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
    };

    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    let mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    for instance in 1000..2023 {
        table.subscribe(CovSubscription {
            subscriber_mac: MacAddr::from_slice(&mac),
            subscriber_network: None,
            subscriber_process_identifier: 99,
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance)
                .unwrap(),
            issue_confirmed_notifications: false,
            expires_at: None,
            last_notified_value: None,
            monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
            monitored_property_array_index: None,
            cov_increment: None,
            notification_kind: CovNotificationKind::Single,
            timestamped: false,
        });
    }
    assert_eq!(table.len(), 1023);

    let request = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 1,
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
        max_notification_delay: Some(10),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: oid,
            list_of_cov_references: vec![
                COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    },
                    cov_increment: Some(0.5),
                    timestamped: false,
                },
                COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: PropertyIdentifier::STATUS_FLAGS,
                        property_array_index: None,
                    },
                    cov_increment: None,
                    timestamped: false,
                },
            ],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let err = handle_subscribe_cov_property_multiple_with_initial(&mut table, &db, &mac, &buf)
        .unwrap_err();
    match err {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::RESOURCES.to_raw() as u32);
            assert_eq!(
                code,
                ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32
            );
        }
        other => panic!("expected NO_SPACE_TO_ADD_LIST_ELEMENT protocol error, got {other:?}"),
    }
    assert_eq!(table.len(), 1023);
}

#[test]
fn subscribe_cov_records_routed_subscriber_endpoint() {
    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    let router_mac = vec![192, 168, 1, 1, 0xBA, 0xC0];
    let remote = NpduAddress {
        network: 100,
        mac_address: MacAddr::from_slice(&[0x0A, 0x14, 0x1E]),
    };
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let request = SubscribeCOVRequest {
        subscriber_process_identifier: 1,
        monitored_object_identifier: oid,
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    let subscriptions = handle_subscribe_cov_with_initial_endpoint(
        &mut table,
        &db,
        &router_mac,
        Some(&remote),
        &buf,
    )
    .unwrap();
    assert_eq!(subscriptions.len(), 1);
    assert_eq!(subscriptions[0].subscriber_mac.as_slice(), &router_mac[..]);
    assert_eq!(subscriptions[0].subscriber_network.as_ref(), Some(&remote));
    assert_eq!(
        table.subscriptions_for(&oid)[0].subscriber_network.as_ref(),
        Some(&remote)
    );
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
    let subscriptions = handle_subscribe_cov_with_initial(&mut table, &db, &mac, &buf).unwrap();
    assert!(subscriptions.is_empty());
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

#[test]
fn delete_network_port_object_fails() {
    // NetworkPort models a running node's port and is not deleteable at
    // runtime, mirroring `NetworkPortObject::is_deleteable` so PICS and the
    // runtime DeleteObject handler share one truth source.
    let mut db = ObjectDatabase::new();
    let np = bacnet_objects::network_port::NetworkPortObject::new(1, "NP-1", 0).unwrap();
    db.add(Box::new(np)).unwrap();

    let oid = ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap();
    let request = bacnet_services::object_mgmt::DeleteObjectRequest {
        object_identifier: oid,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    assert!(
        handle_delete_object(&mut db, &buf).is_err(),
        "DeleteObject must reject NETWORK_PORT (non-deleteable)"
    );
    assert!(
        db.get(&oid).is_some(),
        "NetworkPort must still be present after a rejected delete"
    );
}
