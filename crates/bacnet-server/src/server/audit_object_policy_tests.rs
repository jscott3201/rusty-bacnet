use super::*;

#[tokio::test]
async fn supported_local_write_has_local_device_provenance_and_preimage() {
    let mut fixture = server(reporter()).await;
    fixture
        .server
        .write_local(
            &oid(ObjectType::BINARY_VALUE, 1),
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            Some(8),
        )
        .await
        .unwrap();
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1);
    let record = &records[0].notifications[0];
    assert_eq!(
        record.source_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 10))
    );
    assert_eq!(record.invoke_id, None);
    assert_eq!(record.current_value.as_deref(), Some(&[0x91, 0][..]));
    assert_eq!(record.target_value.as_deref(), Some(&[0x91, 1][..]));
    fixture.server.stop().await.unwrap();
}

use bacnet_objects::{
    analog::AnalogValueObject,
    audit::{AuditPriorityPolicy, ObjectAuditPolicy},
    binary::BinaryValueObject,
};
use bacnet_services::common::BACnetPropertyValue;

fn flags(operations: &[AuditOperation]) -> AuditOperationFlags {
    let mut flags = AuditOperationFlags::empty();
    for operation in operations {
        flags.insert(*operation);
    }
    flags
}
fn encoded(value: &PropertyValue) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut bytes, value).unwrap();
    bytes.to_vec()
}
async fn install(f: &Fixture, policy: ObjectAuditPolicy) {
    let mut db = f.server.db.write().await;
    let mut av = AnalogValueObject::new(11, "policy av", 62).unwrap();
    av.set_audit_policy(policy);
    let mut bv = BinaryValueObject::new(11, "policy bv").unwrap();
    bv.set_audit_policy(policy);
    db.add(Box::new(av)).unwrap();
    db.add(Box::new(bv)).unwrap();
}
async fn write(
    f: &Fixture,
    kind: ObjectType,
    property: PropertyIdentifier,
    value: PropertyValue,
    priority: Option<u8>,
) -> Apdu {
    dispatch(
        &f.server,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        wp(oid(kind, 11), property, encoded(&value), priority),
    )
    .await
}
fn count(f: &Fixture) -> usize {
    notifications(&f.transport.sent).len()
}

#[tokio::test]
async fn object_audit_policy_actual_changes_noops_failures_and_local_paths() {
    for kind in [ObjectType::ANALOG_VALUE, ObjectType::BINARY_VALUE] {
        let mut r = reporter();
        r.set_auditable_operations(AuditOperationFlags::empty());
        let mut f = server(r).await;
        install(
            &f,
            ObjectAuditPolicy {
                level: Some(AuditLevel::NONE),
                operations: Some(AuditOperationFlags::empty()),
                ..Default::default()
            },
        )
        .await;
        f.server
            .write_local(
                &oid(kind, 11),
                PropertyIdentifier::AUDIT_LEVEL,
                None,
                PropertyValue::Enumerated(AuditLevel::NONE.to_raw()),
                None,
            )
            .await
            .unwrap();
        assert!(f
            .server
            .write_local(
                &oid(kind, 11),
                PropertyIdentifier::AUDIT_LEVEL,
                None,
                PropertyValue::Boolean(true),
                None
            )
            .await
            .is_err());
        // No-op and failed operations do not acquire mandatory-change status.
        assert!(matches!(
            write(
                &f,
                kind,
                PropertyIdentifier::AUDIT_LEVEL,
                PropertyValue::Enumerated(AuditLevel::NONE.to_raw()),
                None
            )
            .await,
            Apdu::SimpleAck(_)
        ));
        assert!(matches!(
            write(
                &f,
                kind,
                PropertyIdentifier::AUDIT_LEVEL,
                PropertyValue::Boolean(true),
                None
            )
            .await,
            Apdu::Error(_)
        ));
        settle().await;
        assert_eq!(count(&f), 0);
        f.server
            .write_local(
                &oid(kind, 11),
                PropertyIdentifier::AUDIT_LEVEL,
                None,
                PropertyValue::Enumerated(AuditLevel::AUDIT_ALL.to_raw()),
                None,
            )
            .await
            .unwrap();
        settle().await;
        assert_eq!(count(&f), 1);
        let records = notifications(&f.transport.sent);
        let record = &records[0].notifications[0];
        assert_eq!(record.invoke_id, None);
        assert_eq!(
            record.current_value,
            Some(encoded(&PropertyValue::Enumerated(
                AuditLevel::NONE.to_raw()
            )))
        );
        let (unused_bits, data) = flags(&[AuditOperation::READ]).to_bacnet();
        assert!(matches!(
            write(
                &f,
                kind,
                PropertyIdentifier::AUDITABLE_OPERATIONS,
                PropertyValue::BitString { unused_bits, data },
                None
            )
            .await,
            Apdu::SimpleAck(_)
        ));
        settle().await;
        assert_eq!(count(&f), 2); // WRITE is clear both before and after.
        assert!(matches!(
            write(
                &f,
                kind,
                PropertyIdentifier::AUDIT_LEVEL,
                PropertyValue::Enumerated(AuditLevel::NONE.to_raw()),
                None
            )
            .await,
            Apdu::SimpleAck(_)
        ));
        settle().await;
        assert_eq!(count(&f), 3);
        let (unused_bits, data) = flags(&[AuditOperation::WRITE]).to_bacnet();
        write(
            &f,
            kind,
            PropertyIdentifier::AUDITABLE_OPERATIONS,
            PropertyValue::BitString { unused_bits, data },
            None,
        )
        .await;
        settle().await;
        assert_eq!(count(&f), 3); // Effective NONE suppresses operation changes.
        f.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn object_audit_policy_reporter_none_unselected_and_input_sampling_silent() {
    for none in [false, true] {
        let mut r = reporter();
        if none {
            r.set_audit_level(AuditLevel::NONE).unwrap();
        } else {
            r.set_monitored_objects(Some(vec![]));
        }
        let mut f = server(r).await;
        install(
            &f,
            ObjectAuditPolicy {
                level: Some(AuditLevel::NONE),
                operations: Some(flags(&[AuditOperation::WRITE])),
                ..Default::default()
            },
        )
        .await;
        f.server
            .write_local(
                &oid(ObjectType::ANALOG_VALUE, 11),
                PropertyIdentifier::AUDIT_LEVEL,
                None,
                PropertyValue::Enumerated(AuditLevel::AUDIT_ALL.to_raw()),
                None,
            )
            .await
            .unwrap();
        settle().await;
        assert_eq!(count(&f), 0);
        f.server.stop().await.unwrap();
    }
    let mut f = server(reporter()).await;
    f.server
        .set_present_value_local(&oid(ObjectType::ANALOG_INPUT, 1), PropertyValue::Real(2.0))
        .await
        .unwrap();
    settle().await;
    assert_eq!(count(&f), 0);
    f.server.stop().await.unwrap();
}

#[tokio::test]
async fn object_audit_policy_priorities_config_read_and_failed_write() {
    for filter in [
        None,
        Some(AuditPriorityPolicy::Inherit),
        Some(AuditPriorityPolicy::Filter(
            BACnetPriorityFilter::from_bits(1 << 7),
        )),
    ] {
        let mut r = reporter();
        r.set_audit_priority_filter(BACnetPriorityFilter::from_bits(1 << 7));
        let mut f = server(r).await;
        install(
            &f,
            ObjectAuditPolicy {
                level: Some(AuditLevel::DEFAULT),
                operations: Some(flags(&[AuditOperation::WRITE, AuditOperation::READ])),
                priority_filter: filter,
            },
        )
        .await;
        for priority in [Some(8), Some(16), None] {
            write(
                &f,
                ObjectType::BINARY_VALUE,
                PropertyIdentifier::PRESENT_VALUE,
                PropertyValue::Enumerated(1),
                priority,
            )
            .await;
        }
        write(
            &f,
            ObjectType::BINARY_VALUE,
            PropertyIdentifier::DESCRIPTION,
            PropertyValue::CharacterString("desc".into()),
            Some(16),
        )
        .await;
        write(
            &f,
            ObjectType::BINARY_VALUE,
            PropertyIdentifier::DESCRIPTION,
            PropertyValue::Boolean(true),
            None,
        )
        .await;
        settle().await;
        assert_eq!(count(&f), 3);
        assert!(notifications(&f.transport.sent)[2].notifications[0]
            .result
            .is_some());
        write(
            &f,
            ObjectType::BINARY_VALUE,
            PropertyIdentifier::AUDIT_LEVEL,
            PropertyValue::Enumerated(AuditLevel::AUDIT_CONFIG.to_raw()),
            None,
        )
        .await;
        let mut request = BytesMut::new();
        bacnet_services::read_property::ReadPropertyRequest {
            object_identifier: oid(ObjectType::BINARY_VALUE, 11),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        }
        .encode(&mut request);
        dispatch(
            &f.server,
            ConfirmedServiceChoice::READ_PROPERTY,
            request.freeze(),
        )
        .await;
        write(
            &f,
            ObjectType::BINARY_VALUE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyValue::Enumerated(0),
            Some(8),
        )
        .await;
        settle().await;
        assert_eq!(count(&f), 4);
        let mut request = BytesMut::new();
        bacnet_services::read_property::ReadPropertyRequest {
            object_identifier: oid(ObjectType::BINARY_VALUE, 11),
            property_identifier: PropertyIdentifier::DESCRIPTION,
            property_array_index: None,
        }
        .encode(&mut request);
        dispatch(
            &f.server,
            ConfirmedServiceChoice::READ_PROPERTY,
            request.freeze(),
        )
        .await;
        settle().await;
        assert_eq!(count(&f), 5);
        assert_eq!(
            notifications(&f.transport.sent)[4].notifications[0].operation,
            AuditOperation::READ
        );
        f.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn object_audit_policy_wpm_each_element_captures_committed_prestate() {
    let mut f = server(reporter()).await;
    install(
        &f,
        ObjectAuditPolicy {
            level: Some(AuditLevel::NONE),
            operations: Some(flags(&[AuditOperation::WRITE])),
            ..Default::default()
        },
    )
    .await;
    let properties = [
        (
            PropertyIdentifier::AUDIT_LEVEL,
            PropertyValue::Enumerated(AuditLevel::AUDIT_ALL.to_raw()),
        ),
        (
            PropertyIdentifier::DESCRIPTION,
            PropertyValue::CharacterString("first".into()),
        ),
        (
            PropertyIdentifier::AUDIT_LEVEL,
            PropertyValue::Enumerated(AuditLevel::NONE.to_raw()),
        ),
        (
            PropertyIdentifier::DESCRIPTION,
            PropertyValue::CharacterString("silent".into()),
        ),
        (
            PropertyIdentifier::AUDIT_LEVEL,
            PropertyValue::Boolean(true),
        ),
        (
            PropertyIdentifier::DESCRIPTION,
            PropertyValue::CharacterString("suffix".into()),
        ),
    ];
    let mut bytes = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid(ObjectType::BINARY_VALUE, 11),
            list_of_properties: properties
                .into_iter()
                .map(|(property_identifier, value)| BACnetPropertyValue {
                    property_identifier,
                    property_array_index: None,
                    value: encoded(&value),
                    priority: None,
                })
                .collect(),
        }],
    }
    .encode(&mut bytes);
    assert!(matches!(
        dispatch(
            &f.server,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            bytes.freeze()
        )
        .await,
        Apdu::Error(_)
    ));
    settle().await;
    assert_eq!(count(&f), 3);
    assert_eq!(
        f.server
            .db
            .read()
            .await
            .get(&oid(ObjectType::BINARY_VALUE, 11))
            .unwrap()
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("silent".into())
    );
    f.server.stop().await.unwrap();
}

#[tokio::test]
async fn object_audit_policy_bv_create_delete_and_failed_av_create_fallback() {
    use bacnet_services::object_mgmt::{CreateObjectRequest, DeleteObjectRequest, ObjectSpecifier};
    let mut r = reporter();
    r.set_auditable_operations(flags(&[AuditOperation::CREATE, AuditOperation::DELETE]));
    let mut f = server(r).await;
    // NONE created policy suppresses CREATE and its deletion. The next object
    // at the same OID has no policy state from the deleted instance.
    for level in [Some(AuditLevel::NONE), None] {
        let mut bytes = BytesMut::new();
        CreateObjectRequest {
            object_specifier: ObjectSpecifier::Identifier(oid(ObjectType::BINARY_VALUE, 11)),
            list_of_initial_values: level
                .into_iter()
                .map(|level| BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::AUDIT_LEVEL,
                    property_array_index: None,
                    value: encoded(&PropertyValue::Enumerated(level.to_raw())),
                    priority: None,
                })
                .collect(),
        }
        .encode(&mut bytes);
        assert!(matches!(
            dispatch(
                &f.server,
                ConfirmedServiceChoice::CREATE_OBJECT,
                bytes.freeze()
            )
            .await,
            Apdu::ComplexAck(_)
        ));
        let mut bytes = BytesMut::new();
        DeleteObjectRequest {
            object_identifier: oid(ObjectType::BINARY_VALUE, 11),
        }
        .encode(&mut bytes);
        assert!(matches!(
            dispatch(
                &f.server,
                ConfirmedServiceChoice::DELETE_OBJECT,
                bytes.freeze()
            )
            .await,
            Apdu::SimpleAck(_)
        ));
        settle().await;
        assert_eq!(count(&f), if level.is_some() { 0 } else { 2 });
    }
    let mut bytes = BytesMut::new();
    CreateObjectRequest {
        object_specifier: ObjectSpecifier::Type(ObjectType::ANALOG_VALUE),
        list_of_initial_values: vec![],
    }
    .encode(&mut bytes);
    assert!(matches!(
        dispatch(
            &f.server,
            ConfirmedServiceChoice::CREATE_OBJECT,
            bytes.freeze()
        )
        .await,
        Apdu::Error(_)
    ));
    settle().await;
    assert_eq!(count(&f), 3);
    assert_eq!(
        notifications(&f.transport.sent)[2].notifications[0].result,
        Some((ErrorClass::OBJECT, ErrorCode::UNSUPPORTED_OBJECT_TYPE))
    );
    f.server.stop().await.unwrap();
}

#[tokio::test]
async fn object_audit_policy_also_filters_list_and_file_execution_failures() {
    use bacnet_services::list_manipulation::ListElementRequest;
    for level in [AuditLevel::NONE, AuditLevel::AUDIT_ALL] {
        let mut f = server(reporter()).await;
        install(
            &f,
            ObjectAuditPolicy {
                level: Some(level),
                ..Default::default()
            },
        )
        .await;
        let target = oid(ObjectType::BINARY_VALUE, 11);
        let mut bytes = BytesMut::new();
        ListElementRequest {
            object_identifier: target,
            property_identifier: PropertyIdentifier::DESCRIPTION,
            property_array_index: None,
            list_of_elements: encoded(&PropertyValue::CharacterString("not a list".into())),
        }
        .encode(&mut bytes);
        assert!(matches!(
            dispatch(
                &f.server,
                ConfirmedServiceChoice::ADD_LIST_ELEMENT,
                bytes.freeze()
            )
            .await,
            Apdu::Error(_)
        ));
        assert!(matches!(
            dispatch(
                &f.server,
                file::SERVICE,
                file::request(target, file::access(false, 0))
            )
            .await,
            Apdu::Error(_)
        ));
        settle().await;
        assert_eq!(count(&f), if level == AuditLevel::NONE { 0 } else { 2 });
        assert!(notifications(&f.transport.sent)
            .iter()
            .all(|n| n.notifications[0].result.is_some()));
        f.server.stop().await.unwrap();
    }
}
