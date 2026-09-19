use super::*;
use bacnet_services::{
    common::BACnetPropertyValue,
    object_mgmt::{CreateObjectRequest, DeleteObjectRequest, ObjectSpecifier},
};

fn lifecycle_reporter() -> bacnet_objects::audit::AuditReporterObject {
    let mut reporter = reporter();
    let mut operations = AuditOperationFlags::empty();
    for operation in [
        AuditOperation::CREATE,
        AuditOperation::DELETE,
        AuditOperation::WRITE,
    ] {
        operations.insert(operation);
    }
    reporter.set_auditable_operations(operations);
    reporter
}

fn create(specifier: ObjectSpecifier, values: Vec<BACnetPropertyValue>) -> Bytes {
    let mut bytes = BytesMut::new();
    CreateObjectRequest {
        object_specifier: specifier,
        list_of_initial_values: values,
    }
    .encode(&mut bytes);
    bytes.freeze()
}

fn delete(object: ObjectIdentifier) -> Bytes {
    let mut bytes = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: object,
    }
    .encode(&mut bytes);
    bytes.freeze()
}

fn assert_record(
    record: &BACnetAuditNotification,
    operation: AuditOperation,
    target: ObjectIdentifier,
    invoke: u8,
    sequence: u16,
    result: Option<(ErrorClass, ErrorCode)>,
) {
    assert_eq!(
        record,
        &BACnetAuditNotification {
            source_timestamp: None,
            target_timestamp: Some(BACnetTimeStamp::SequenceNumber(sequence)),
            source_device: BACnetRecipient::Address(BACnetAddress {
                network_number: 0,
                mac_address: MacAddr::from_slice(SOURCE),
            }),
            source_object: None,
            operation,
            source_comment: None,
            target_comment: None,
            invoke_id: Some(invoke),
            source_user_id: None,
            source_user_role: None,
            target_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 10)),
            target_object: Some(target),
            target_property: None,
            target_priority: None,
            target_value: None,
            current_value: None,
            result,
        }
    );
}

#[tokio::test]
async fn audit_reporter_create_delete_success_uses_final_identity_without_initial_writes() {
    let mut fixture = server(lifecycle_reporter()).await;
    let target = oid(ObjectType::BINARY_VALUE, 3);
    let response = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::CREATE_OBJECT,
        create(
            ObjectSpecifier::Type(ObjectType::BINARY_VALUE),
            vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: vec![0x91, 1],
                priority: Some(8),
            }],
        ),
    )
    .await;
    let Apdu::ComplexAck(ack) = response else {
        panic!("{response:?}")
    };
    assert_eq!(
        bacnet_encoding::primitives::decode_application_value(&ack.service_ack, 0)
            .unwrap()
            .0,
        PropertyValue::ObjectIdentifier(target)
    );
    assert_eq!(
        fixture
            .server
            .db
            .read()
            .await
            .get(&target)
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(1)
    );
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].notifications.len(), 1);
    assert_record(
        &records[0].notifications[0],
        AuditOperation::CREATE,
        target,
        77,
        0,
        None,
    );

    {
        let db = fixture.server.db.read().await;
        let mut table = fixture.server.cov_table.write().await;
        for object in [target, oid(ObjectType::BINARY_VALUE, 2)] {
            let mut bytes = BytesMut::new();
            bacnet_services::cov::SubscribeCOVRequest {
                subscriber_process_identifier: 1,
                monitored_object_identifier: object,
                issue_confirmed_notifications: Some(false),
                lifetime: Some(60),
            }
            .encode(&mut bytes);
            handlers::handle_subscribe_cov(&mut table, &db, SOURCE, &bytes).unwrap();
        }
        assert_eq!(table.len(), 2);
    }
    let response = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::DELETE_OBJECT,
        delete(target),
    )
    .await;
    assert!(matches!(response, Apdu::SimpleAck(_)), "{response:?}");
    assert!(fixture.server.db.read().await.get(&target).is_none());
    {
        let mut table = fixture.server.cov_table.write().await;
        assert_eq!(table.len(), 1);
        assert!(table.subscriptions_for(&target).is_empty());
        assert_eq!(
            table
                .subscriptions_for(&oid(ObjectType::BINARY_VALUE, 2))
                .len(),
            1
        );
    }
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].notifications.len(), 1);
    assert_record(
        &records[1].notifications[0],
        AuditOperation::DELETE,
        target,
        78,
        1,
        None,
    );
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_create_late_initial_value_decode_failure_is_silent_and_rolls_back() {
    let candidate = oid(ObjectType::BINARY_VALUE, 3);
    for specifier in [
        ObjectSpecifier::Type(ObjectType::BINARY_VALUE),
        ObjectSpecifier::Identifier(candidate),
    ] {
        let mut fixture = server(lifecycle_reporter()).await;
        let initial_count = fixture.server.db.read().await.len();
        let data = create(
            specifier,
            vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_NAME,
                    property_array_index: None,
                    value: vec![0x72, 0, b'x'],
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0xd1, 0x00], // Framed, but not a supported application tag.
                    priority: None,
                },
            ],
        );
        // The service parser retains these bytes; application-value decoding is
        // deferred until after creation and the preceding Object_Name write.
        let decoded = CreateObjectRequest::decode(&data).unwrap();
        assert!(matches!(
            bacnet_encoding::primitives::decode_application_value(
                &decoded.list_of_initial_values[1].value,
                0
            ),
            Err(Error::Decoding { .. })
        ));
        let response = dispatch(&fixture.server, ConfirmedServiceChoice::CREATE_OBJECT, data).await;
        let Apdu::Error(error) = response else {
            panic!("{response:?}")
        };
        assert_eq!(error.invoke_id, 77);
        assert_eq!(error.service_choice, ConfirmedServiceChoice::CREATE_OBJECT);
        assert_eq!(
            (error.error_class, error.error_code),
            (ErrorClass::SERVICES, ErrorCode::OTHER)
        );
        assert!(error.error_data.is_empty());
        {
            let db = fixture.server.db.read().await;
            assert_eq!(db.len(), initial_count);
            assert!(db.get(&candidate).is_none());
            assert!(!db
                .find_by_type(ObjectType::BINARY_VALUE)
                .contains(&candidate));
            // Use a different OID: a stale name-index entry for the rolled-back
            // candidate must not look like a successful same-owner name check.
            let other = oid(ObjectType::BINARY_VALUE, 4);
            assert!(db.check_name_available(&other, "x").is_ok());
            assert!(db
                .check_name_available(&other, &format!("{:?}-3", ObjectType::BINARY_VALUE))
                .is_ok());
        }
        settle().await;
        assert!(notifications(&fixture.transport.sent).is_empty());
        fixture.server.stop().await.unwrap();
        assert!(notifications(&fixture.transport.sent).is_empty());
    }
}

#[tokio::test]
async fn audit_reporter_lifecycle_execution_errors_preserve_results_and_create_rollback() {
    let mut fixture = server(lifecycle_reporter()).await;
    let candidate = oid(ObjectType::BINARY_VALUE, 3);
    let cases = [
        (
            ConfirmedServiceChoice::CREATE_OBJECT,
            AuditOperation::CREATE,
            create(
                ObjectSpecifier::Identifier(oid(ObjectType::BINARY_VALUE, 1)),
                vec![],
            ),
            oid(ObjectType::BINARY_VALUE, 1),
            ErrorClass::OBJECT,
            ErrorCode::OBJECT_IDENTIFIER_ALREADY_EXISTS,
        ),
        (
            ConfirmedServiceChoice::CREATE_OBJECT,
            AuditOperation::CREATE,
            create(ObjectSpecifier::Type(ObjectType::ANALOG_VALUE), vec![]),
            oid(ObjectType::ANALOG_VALUE, 1),
            ErrorClass::OBJECT,
            ErrorCode::UNSUPPORTED_OBJECT_TYPE,
        ),
        (
            ConfirmedServiceChoice::CREATE_OBJECT,
            AuditOperation::CREATE,
            create(
                ObjectSpecifier::Type(ObjectType::BINARY_VALUE),
                vec![
                    BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::OBJECT_NAME,
                        property_array_index: None,
                        value: vec![0x72, 0, b'x'],
                        priority: None,
                    },
                    BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                        value: vec![0x91, 9],
                        priority: None,
                    },
                ],
            ),
            candidate,
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            ConfirmedServiceChoice::DELETE_OBJECT,
            AuditOperation::DELETE,
            delete(candidate),
            candidate,
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        ),
        (
            ConfirmedServiceChoice::DELETE_OBJECT,
            AuditOperation::DELETE,
            delete(oid(ObjectType::DEVICE, 10)),
            oid(ObjectType::DEVICE, 10),
            ErrorClass::OBJECT,
            ErrorCode::OBJECT_DELETION_NOT_PERMITTED,
        ),
        (
            ConfirmedServiceChoice::DELETE_OBJECT,
            AuditOperation::DELETE,
            delete(oid(ObjectType::NETWORK_PORT, 1)),
            oid(ObjectType::NETWORK_PORT, 1),
            ErrorClass::OBJECT,
            ErrorCode::OBJECT_DELETION_NOT_PERMITTED,
        ),
    ];
    for (index, (service, operation, request, target, class, code)) in cases.into_iter().enumerate()
    {
        let response = dispatch(&fixture.server, service, request).await;
        let Apdu::Error(error) = response else {
            panic!("{response:?}")
        };
        assert_eq!(error.service_choice, service);
        assert_eq!((error.error_class, error.error_code), (class, code));
        assert!(error.error_data.is_empty());
        settle().await;
        let records = notifications(&fixture.transport.sent);
        assert_eq!(records.len(), index + 1);
        assert_eq!(records[index].notifications.len(), 1);
        assert_record(
            &records[index].notifications[0],
            operation,
            target,
            77 + index as u8,
            index as u16,
            Some((class, code)),
        );
        let db = fixture.server.db.read().await;
        assert!(db.get(&candidate).is_none());
        assert!(db.check_name_available(&candidate, "x").is_ok());
        assert!(db.get(&oid(ObjectType::DEVICE, 10)).is_some());
    }
    // The failed by-type creation did not consume the candidate instance.
    let response = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::CREATE_OBJECT,
        create(ObjectSpecifier::Type(ObjectType::BINARY_VALUE), vec![]),
    )
    .await;
    let Apdu::ComplexAck(ack) = response else {
        panic!("{response:?}")
    };
    assert_eq!(
        bacnet_encoding::primitives::decode_application_value(&ack.service_ack, 0)
            .unwrap()
            .0,
        PropertyValue::ObjectIdentifier(candidate)
    );
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_lifecycle_filters_final_identity_success_and_failure() {
    use bacnet_types::constructed::BACnetObjectSelector as Selector;
    let target = oid(ObjectType::BINARY_VALUE, 3);
    for (selectors, level, create_bit, delete_bit, expected) in [
        (None, AuditLevel::AUDIT_CONFIG, true, true, 4),
        (Some(vec![]), AuditLevel::AUDIT_ALL, true, true, 0),
        (
            Some(vec![Selector::None]),
            AuditLevel::AUDIT_CONFIG,
            true,
            true,
            0,
        ),
        (
            Some(vec![Selector::Object(target)]),
            AuditLevel::AUDIT_CONFIG,
            true,
            true,
            4,
        ),
        (
            Some(vec![Selector::Object(oid(ObjectType::BINARY_VALUE, 2))]),
            AuditLevel::AUDIT_ALL,
            true,
            true,
            0,
        ),
        (
            Some(vec![Selector::ObjectType(ObjectType::BINARY_VALUE)]),
            AuditLevel::AUDIT_ALL,
            true,
            true,
            4,
        ),
        (
            Some(vec![Selector::ObjectType(ObjectType::ANALOG_INPUT)]),
            AuditLevel::AUDIT_CONFIG,
            true,
            true,
            0,
        ),
        (
            Some(vec![
                Selector::None,
                Selector::Object(target),
                Selector::Object(target),
                Selector::ObjectType(ObjectType::BINARY_VALUE),
            ]),
            AuditLevel::AUDIT_CONFIG,
            true,
            true,
            4,
        ),
        (None, AuditLevel::NONE, true, true, 0),
        (None, AuditLevel::AUDIT_CONFIG, false, true, 2),
        (None, AuditLevel::AUDIT_ALL, true, false, 2),
        (None, AuditLevel::AUDIT_ALL, false, false, 0),
    ] {
        let mut reporter = lifecycle_reporter();
        reporter.set_monitored_objects(selectors);
        reporter.set_audit_level(level).unwrap();
        reporter.set_audit_priority_filter(BACnetPriorityFilter::from_bits(0));
        let mut operations = AuditOperationFlags::empty();
        if create_bit {
            operations.insert(AuditOperation::CREATE);
        }
        if delete_bit {
            operations.insert(AuditOperation::DELETE);
        }
        reporter.set_auditable_operations(operations);
        let mut fixture = server(reporter).await;
        let response = dispatch(
            &fixture.server,
            ConfirmedServiceChoice::CREATE_OBJECT,
            create(ObjectSpecifier::Type(ObjectType::BINARY_VALUE), vec![]),
        )
        .await;
        assert!(matches!(response, Apdu::ComplexAck(_)), "{response:?}");
        let response = dispatch(
            &fixture.server,
            ConfirmedServiceChoice::CREATE_OBJECT,
            create(ObjectSpecifier::Identifier(target), vec![]),
        )
        .await;
        assert!(matches!(response, Apdu::Error(_)), "{response:?}");
        let response = dispatch(
            &fixture.server,
            ConfirmedServiceChoice::DELETE_OBJECT,
            delete(target),
        )
        .await;
        assert!(matches!(response, Apdu::SimpleAck(_)), "{response:?}");
        let response = dispatch(
            &fixture.server,
            ConfirmedServiceChoice::DELETE_OBJECT,
            delete(target),
        )
        .await;
        assert!(matches!(response, Apdu::Error(_)), "{response:?}");
        assert!(fixture.server.db.read().await.get(&target).is_none());
        settle().await;
        let records = notifications(&fixture.transport.sent);
        assert_eq!(records.len(), expected);
        for (sequence, request) in records.iter().enumerate() {
            assert_eq!(request.notifications.len(), 1);
            let record = &request.notifications[0];
            let invoke = record.invoke_id.unwrap();
            let (operation, result) = match invoke {
                77 => (AuditOperation::CREATE, None),
                78 => (
                    AuditOperation::CREATE,
                    Some((
                        ErrorClass::OBJECT,
                        ErrorCode::OBJECT_IDENTIFIER_ALREADY_EXISTS,
                    )),
                ),
                79 => (AuditOperation::DELETE, None),
                80 => (
                    AuditOperation::DELETE,
                    Some((ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT)),
                ),
                _ => panic!("unexpected invoke"),
            };
            assert!(if operation == AuditOperation::CREATE {
                create_bit
            } else {
                delete_bit
            });
            assert_record(record, operation, target, invoke, sequence as u16, result);
        }
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_lifecycle_denials_and_decode_failures_are_silent() {
    use crate::mutation::MutationPolicy;
    for policy_denial in [true, false] {
        let mut fixture = server(lifecycle_reporter()).await;
        if policy_denial {
            fixture.server.config.mutation_policy = MutationPolicy::DenyAll;
        } else {
            fixture.server.config.mutation_authorizer = Some(Arc::new(|_| false));
        }
        for (service, request) in [
            (
                ConfirmedServiceChoice::CREATE_OBJECT,
                create(ObjectSpecifier::Type(ObjectType::BINARY_VALUE), vec![]),
            ),
            (
                ConfirmedServiceChoice::DELETE_OBJECT,
                delete(oid(ObjectType::BINARY_VALUE, 1)),
            ),
        ] {
            let response = dispatch(&fixture.server, service, request).await;
            let Apdu::Error(error) = response else {
                panic!("{response:?}")
            };
            assert_eq!(error.error_code, ErrorCode::SERVICE_REQUEST_DENIED);
        }
        fixture.server.config.mutation_policy = MutationPolicy::Permissive;
        fixture.server.config.mutation_authorizer = None;
        for service in [
            ConfirmedServiceChoice::CREATE_OBJECT,
            ConfirmedServiceChoice::DELETE_OBJECT,
        ] {
            let response = dispatch(&fixture.server, service, Bytes::new()).await;
            assert!(
                matches!(response, Apdu::Error(_) | Apdu::Reject(_)),
                "{response:?}"
            );
        }
        settle().await;
        assert!(notifications(&fixture.transport.sent).is_empty());
        let db = fixture.server.db.read().await;
        assert!(db.get(&oid(ObjectType::BINARY_VALUE, 1)).is_some());
        assert!(db.get(&oid(ObjectType::BINARY_VALUE, 3)).is_none());
        drop(db);
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn audit_reporter_selected_reporter_deletion_survives_removal_without_recursion() {
    for (level, delete_bit, expected) in [
        (AuditLevel::AUDIT_CONFIG, true, 1),
        (AuditLevel::NONE, true, 0),
        (AuditLevel::AUDIT_ALL, false, 0),
    ] {
        let mut reporter = lifecycle_reporter();
        reporter.set_monitored_objects(Some(vec![]));
        reporter.set_audit_level(level).unwrap();
        if !delete_bit {
            reporter.set_auditable_operations(AuditOperationFlags::empty());
        }
        let mut fixture = server(reporter).await;
        fixture.transport.block.store(true, Ordering::Release);
        let target = oid(ObjectType::AUDIT_REPORTER, 1);
        let response = dispatch(
            &fixture.server,
            ConfirmedServiceChoice::DELETE_OBJECT,
            delete(target),
        )
        .await;
        assert!(matches!(response, Apdu::SimpleAck(_)), "{response:?}");
        assert!(fixture.server.db.read().await.get(&target).is_none());
        settle().await;
        let records = notifications(&fixture.transport.sent);
        assert_eq!(records.len(), expected);
        if expected == 1 {
            assert_record(
                &records[0].notifications[0],
                AuditOperation::DELETE,
                target,
                77,
                0,
                None,
            );
        }
        // Configured profile now has no Reporter, but ordinary mutations still work.
        let response = dispatch(
            &fixture.server,
            ConfirmedServiceChoice::CREATE_OBJECT,
            create(
                ObjectSpecifier::Identifier(oid(ObjectType::BINARY_VALUE, 3)),
                vec![],
            ),
        )
        .await;
        assert!(matches!(response, Apdu::ComplexAck(_)), "{response:?}");
        settle().await;
        assert_eq!(notifications(&fixture.transport.sent).len(), expected);
        // A new object at the same OID must not inherit the old send's timeout.
        let replacement = lifecycle_reporter();
        replacement.status_internal().set_configured(true);
        fixture
            .server
            .db
            .write()
            .await
            .add(Box::new(replacement))
            .unwrap();
        tokio::time::advance(Duration::from_secs(4)).await;
        settle().await;
        assert_eq!(
            health(&fixture.server).await,
            Reliability::NO_FAULT_DETECTED
        );
        assert_eq!(notifications(&fixture.transport.sent).len(), expected);
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_unassigned_create_failures_never_invent_an_oid() {
    for capacity_failure in [false, true] {
        let mut fixture = server(lifecycle_reporter()).await;
        let (kind, class, code) = if capacity_failure {
            let mut db = fixture.server.db.write().await;
            let count = 10_000 - db.len();
            for instance in 3..3 + count as u32 {
                db.add(Box::new(
                    bacnet_objects::binary::BinaryValueObject::new(
                        instance,
                        format!("capacity-{instance}"),
                    )
                    .unwrap(),
                ))
                .unwrap();
            }
            assert_eq!(db.len(), 10_000);
            (
                ObjectType::BINARY_VALUE,
                ErrorClass::RESOURCES,
                ErrorCode::NO_SPACE_FOR_OBJECT,
            )
        } else {
            // The service's extensible ObjectType can exceed the OID's 10 bits.
            (
                ObjectType::from_raw(1024),
                ErrorClass::OBJECT,
                ErrorCode::UNSUPPORTED_OBJECT_TYPE,
            )
        };
        let response = dispatch(
            &fixture.server,
            ConfirmedServiceChoice::CREATE_OBJECT,
            create(ObjectSpecifier::Type(kind), vec![]),
        )
        .await;
        let Apdu::Error(error) = response else {
            panic!("{response:?}")
        };
        assert_eq!((error.error_class, error.error_code), (class, code));
        settle().await;
        let records = notifications(&fixture.transport.sent);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].notifications.len(), 1);
        let record = &records[0].notifications[0];
        assert_eq!(record.operation, AuditOperation::CREATE);
        assert_eq!(record.target_object, None);
        assert_eq!(record.result, Some((class, code)));
        assert!(record.target_property.is_none());
        assert!(record.target_priority.is_none());
        assert!(record.target_value.is_none());
        assert!(record.current_value.is_none());
        fixture.server.stop().await.unwrap();
    }
}
