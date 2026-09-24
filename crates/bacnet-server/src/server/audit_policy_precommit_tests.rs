use super::*;

#[tokio::test]
async fn mandatory_policy_send_capacity_denial_is_atomic() {
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
    f.server.db.write().await.set_clock_reader(None);
    let held: Vec<_> = (0..64)
        .map(|_| {
            f.server
                .notification_transactions
                .try_admit_audit()
                .unwrap()
        })
        .collect();
    let sequence = f
        .server
        .db
        .read()
        .await
        .reserve_event_sequence_number()
        .number();
    let target = oid(ObjectType::ANALOG_VALUE, 11);
    let result = f
        .server
        .write_local(
            &target,
            PropertyIdentifier::AUDIT_LEVEL,
            None,
            PropertyValue::Enumerated(AuditLevel::AUDIT_ALL.to_raw()),
            None,
        )
        .await;
    assert!(
        matches!(result, Err(Error::Protocol { class, code })
        if class == ErrorClass::SERVICES.to_raw() as u32 && code == ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32),
        "{result:?}"
    );
    let db = f.server.db.read().await;
    assert_eq!(
        db.get(&target)
            .unwrap()
            .audit_object_policy_internal()
            .level,
        Some(AuditLevel::NONE)
    );
    assert_eq!(db.reserve_event_sequence_number().number(), sequence);
    drop(db);
    assert_eq!(count(&f), 0);
    drop(held);
    f.server.stop().await.unwrap();
}

fn denied(result: Result<(), Error>) {
    assert!(
        matches!(result, Err(Error::Protocol { class, code })
        if class == ErrorClass::SERVICES.to_raw() as u32 && code == ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32),
        "{result:?}"
    );
}
fn policy() -> ObjectAuditPolicy {
    ObjectAuditPolicy {
        level: Some(AuditLevel::AUDIT_ALL),
        operations: Some(flags(&[AuditOperation::WRITE])),
        ..Default::default()
    }
}
fn change_value(property: PropertyIdentifier) -> PropertyValue {
    if property == PropertyIdentifier::AUDIT_LEVEL {
        PropertyValue::Enumerated(AuditLevel::AUDIT_CONFIG.to_raw())
    } else {
        let (unused_bits, data) = flags(&[AuditOperation::READ]).to_bacnet();
        PropertyValue::BitString { unused_bits, data }
    }
}
async fn assert_unchanged(f: &Fixture, kind: ObjectType, before: ObjectAuditPolicy, sequence: u16) {
    let db = f.server.db.read().await;
    assert_eq!(
        db.get(&oid(kind, 11))
            .unwrap()
            .audit_object_policy_internal(),
        before
    );
    assert_eq!(db.reserve_event_sequence_number().number(), sequence);
    assert_eq!(count(f), 0);
}

#[tokio::test]
async fn mandatory_policy_wire_matrix_capacity_retry_provenance_and_sequence() {
    for kind in [ObjectType::ANALOG_VALUE, ObjectType::BINARY_VALUE] {
        for property in [
            PropertyIdentifier::AUDIT_LEVEL,
            PropertyIdentifier::AUDITABLE_OPERATIONS,
        ] {
            let mut f = server(reporter()).await;
            install(&f, policy()).await;
            f.server.db.write().await.set_clock_reader(None);
            let sequence = f
                .server
                .db
                .read()
                .await
                .reserve_event_sequence_number()
                .number();
            let held: Vec<_> = (0..64)
                .map(|_| {
                    f.server
                        .notification_transactions
                        .try_admit_audit()
                        .unwrap()
                })
                .collect();
            let response = write(&f, kind, property, change_value(property), None).await;
            assert!(
                matches!(response, Apdu::Error(ref e) if e.error_class == ErrorClass::SERVICES && e.error_code == ErrorCode::SERVICE_REQUEST_DENIED),
                "{response:?}"
            );
            assert_unchanged(&f, kind, policy(), sequence).await;
            assert_eq!(health(&f.server).await, Reliability::NO_FAULT_DETECTED);
            assert_eq!(
                f.server.notification_transactions.audit_resources(),
                (false, 0, 0)
            );
            assert_eq!(f.server.notification_transactions.active_count(), 0);
            drop(held);
            assert!(matches!(
                write(&f, kind, property, change_value(property), None).await,
                Apdu::SimpleAck(_)
            ));
            settle().await;
            let records = notifications(&f.transport.sent);
            assert_eq!(records.len(), 1);
            let record = &records[0].notifications[0];
            assert_eq!(record.target_object, Some(oid(kind, 11)));
            assert_eq!(
                record.target_property.as_ref().unwrap().property_identifier,
                property
            );
            assert_eq!(
                record.target_timestamp,
                Some(BACnetTimeStamp::SequenceNumber(sequence))
            );
            assert_eq!(record.invoke_id, Some(78));
            assert_eq!(
                record.source_device,
                BACnetRecipient::Address(BACnetAddress {
                    network_number: 0,
                    mac_address: MacAddr::from_slice(SOURCE)
                })
            );
            assert_eq!(
                f.server
                    .db
                    .read()
                    .await
                    .reserve_event_sequence_number()
                    .number(),
                sequence.wrapping_add(1)
            );
            f.server.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn mandatory_policy_confirmed_lease_route_and_closed_owner_denials() {
    for reason in ["lease", "route", "sealed", "closed"] {
        let mut r = reporter();
        r.set_issue_confirmed_notifications(true).unwrap();
        let mut f = if reason == "route" {
            try_servers(
                vec![r],
                &[10],
                Some(BACnetRecipient::Device(oid(ObjectType::DEVICE, 999))),
                vec![],
            )
            .await
            .unwrap()
        } else {
            server(r).await
        };
        install(&f, policy()).await;
        f.server.db.write().await.set_clock_reader(None);
        let sequence = f
            .server
            .db
            .read()
            .await
            .reserve_event_sequence_number()
            .number();
        let mut held = vec![];
        if reason == "lease" {
            for _ in 0..256 {
                held.push(
                    f.server
                        .notification_transactions
                        .reserve(
                            canonical_direct_peer(LOGGER),
                            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
                        )
                        .unwrap(),
                );
            }
        } else if reason == "sealed" {
            f.server.target_audit.as_ref().unwrap().seal();
        } else if reason == "closed" {
            f.server.notification_transactions.close();
        }
        denied(
            f.server
                .write_local(
                    &oid(ObjectType::BINARY_VALUE, 11),
                    PropertyIdentifier::AUDIT_LEVEL,
                    None,
                    change_value(PropertyIdentifier::AUDIT_LEVEL),
                    None,
                )
                .await,
        );
        assert_unchanged(&f, ObjectType::BINARY_VALUE, policy(), sequence).await;
        assert_eq!(f.server.notification_transactions.audit_resources().2, 64);
        drop(held);
        f.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn mandatory_policy_no_runtime_refuses_without_consuming_sequence() {
    let mut f = server(reporter()).await;
    install(&f, policy()).await;
    f.server.db.write().await.set_clock_reader(None);
    let result = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let mut db = f.server.db.blocking_write();
                let mut audit = audit_reporter::WriteAudit::local(
                    &f.server.config,
                    &f.server.network,
                    &f.server.notification_transactions,
                    &f.server.comm_state,
                    &db,
                )
                .unwrap();
                crate::handlers::handle_write_property_observed(
                    &mut db,
                    &wp(
                        oid(ObjectType::ANALOG_VALUE, 11),
                        PropertyIdentifier::AUDIT_LEVEL,
                        encoded(&change_value(PropertyIdentifier::AUDIT_LEVEL)),
                        None,
                    ),
                    Some(&mut audit),
                    None,
                )
                .map(|_| ())
            })
            .join()
            .unwrap()
    });
    denied(result);
    assert_unchanged(&f, ObjectType::ANALOG_VALUE, policy(), 0).await;
    f.server.stop().await.unwrap();
}

#[tokio::test]
async fn mandatory_policy_invalid_noop_null_and_ordinary_writes_keep_their_contract() {
    let mut r = reporter();
    r.set_auditable_operations(AuditOperationFlags::empty())
        .unwrap();
    let mut f = server(r).await;
    install(
        &f,
        ObjectAuditPolicy {
            operations: Some(AuditOperationFlags::empty()),
            ..policy()
        },
    )
    .await;
    let held: Vec<_> = (0..64)
        .map(|_| {
            f.server
                .notification_transactions
                .try_admit_audit()
                .unwrap()
        })
        .collect();
    let target = oid(ObjectType::BINARY_VALUE, 11);
    for value in [
        PropertyValue::Null,
        PropertyValue::Enumerated(AuditLevel::AUDIT_ALL.to_raw()),
    ] {
        f.server
            .write_local(&target, PropertyIdentifier::AUDIT_LEVEL, None, value, None)
            .await
            .unwrap();
    }
    for (index, value, priority, class, code) in [
        (
            Some(0),
            PropertyValue::Null,
            None,
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
        (
            None,
            PropertyValue::Boolean(true),
            None,
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            None,
            change_value(PropertyIdentifier::AUDIT_LEVEL),
            Some(17),
            ErrorClass::SERVICES,
            ErrorCode::PARAMETER_OUT_OF_RANGE,
        ),
    ] {
        let error = f
            .server
            .write_local(
                &target,
                PropertyIdentifier::AUDIT_LEVEL,
                index,
                value,
                priority,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(error, Error::Protocol { class: c, code: e } if c == class.to_raw() as u32 && e == code.to_raw() as u32),
            "{error:?}"
        );
    }
    f.server
        .write_local(
            &target,
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("ordinary".into()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(count(&f), 0);
    drop(held);
    f.server.stop().await.unwrap();
}

#[tokio::test]
async fn mandatory_policy_wpm_denial_keeps_prefix_and_exact_coordinate_then_retries() {
    for kind in [ObjectType::ANALOG_VALUE, ObjectType::BINARY_VALUE] {
        let mut f = server(reporter()).await;
        let initial = ObjectAuditPolicy {
            level: Some(AuditLevel::NONE),
            ..policy()
        };
        install(&f, initial).await;
        f.server.db.write().await.set_clock_reader(None);
        let held: Vec<_> = (0..64)
            .map(|_| {
                f.server
                    .notification_transactions
                    .try_admit_audit()
                    .unwrap()
            })
            .collect();
        let target = oid(kind, 11);
        let mut request = BytesMut::new();
        WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: target,
                list_of_properties: [
                    (
                        PropertyIdentifier::DESCRIPTION,
                        PropertyValue::CharacterString("prefix".into()),
                    ),
                    (
                        PropertyIdentifier::AUDIT_LEVEL,
                        PropertyValue::Enumerated(AuditLevel::AUDIT_ALL.to_raw()),
                    ),
                    (
                        PropertyIdentifier::DESCRIPTION,
                        PropertyValue::CharacterString("suffix".into()),
                    ),
                ]
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
        .encode(&mut request)
        .unwrap();
        let request = request.freeze();
        let response = dispatch(
            &f.server,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            request.clone(),
        )
        .await;
        let Apdu::Error(error) = response else {
            panic!("expected WPM Error: {response:?}")
        };
        let error =
            bacnet_services::wpm::WritePropertyMultipleError::from_error_pdu(&error).unwrap();
        assert_eq!(
            (error.error_class, error.error_code),
            (ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED)
        );
        assert_eq!(
            error.first_failed_write_attempt,
            bacnet_types::constructed::BACnetObjectPropertyReference {
                object_identifier: target,
                property_identifier: PropertyIdentifier::AUDIT_LEVEL.to_raw(),
                property_array_index: None,
            }
        );
        assert_unchanged(&f, kind, initial, 0).await;
        assert_eq!(
            f.server
                .db
                .read()
                .await
                .get(&target)
                .unwrap()
                .read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString("prefix".into())
        );
        drop(held);
        assert!(matches!(
            dispatch(
                &f.server,
                ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
                request
            )
            .await,
            Apdu::SimpleAck(_)
        ));
        settle().await;
        let records = notifications(&f.transport.sent);
        assert_eq!(records.len(), 2);
        // Following elements observe the just-committed policy; records carry
        // distinct ordered acquisition sequences even if delivery is concurrent.
        for (property, number) in [
            (PropertyIdentifier::AUDIT_LEVEL, 0),
            (PropertyIdentifier::DESCRIPTION, 1),
        ] {
            let record = records
                .iter()
                .flat_map(|r| &r.notifications)
                .find(|r| r.target_property.as_ref().unwrap().property_identifier == property)
                .unwrap();
            assert_eq!(
                record.target_timestamp,
                Some(BACnetTimeStamp::SequenceNumber(number))
            );
            assert_eq!(record.invoke_id, Some(78));
        }
        assert_eq!(
            f.server
                .db
                .read()
                .await
                .get(&target)
                .unwrap()
                .read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString("suffix".into())
        );
        f.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn mandatory_policy_stop_between_capture_and_commit_denies_without_assignment() {
    use crate::handlers::{WriteCommitObserver, WriteTarget};
    let mut f = server(reporter()).await;
    install(&f, policy()).await;
    let database = Arc::clone(&f.server.db);
    let mut db = database.write().await;
    db.set_clock_reader(None);
    let value = change_value(PropertyIdentifier::AUDIT_LEVEL);
    let encoded = encoded(&value);
    let target = WriteTarget {
        oid: oid(ObjectType::ANALOG_VALUE, 11),
        property: PropertyIdentifier::AUDIT_LEVEL,
        array_index: None,
        priority: None,
        value: &encoded,
    };
    let mut audit = audit_reporter::WriteAudit::local(
        &f.server.config,
        &f.server.network,
        &f.server.notification_transactions,
        &f.server.comm_state,
        &db,
    )
    .unwrap();
    audit.before(&db, target);
    // This is the exact precommit boundary; no arbitrary callback is inserted
    // inside the assignment/admission critical section to manufacture the race.
    f.server.target_audit.as_ref().unwrap().seal();
    denied(audit.commit_policy(&mut db, target, &value).unwrap());
    audit.failed(
        &mut db,
        &Error::Protocol {
            class: ErrorClass::SERVICES.to_raw() as u32,
            code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
        },
    );
    assert_eq!(
        db.get(&target.oid).unwrap().audit_object_policy_internal(),
        policy()
    );
    assert_eq!(db.reserve_event_sequence_number().number(), 0);
    assert_eq!(
        f.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
    assert_eq!(count(&f), 0);
    drop(audit);
    drop(db);
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn mandatory_policy_admitted_worker_is_joined_after_cancelled_stop() {
    let mut r = reporter();
    r.set_issue_confirmed_notifications(true).unwrap();
    let mut f = server(r).await;
    install(&f, policy()).await;
    f.server.db.write().await.set_clock_reader(None);
    f.transport.block.store(true, Ordering::Release);
    f.server
        .write_local(
            &oid(ObjectType::BINARY_VALUE, 11),
            PropertyIdentifier::AUDIT_LEVEL,
            None,
            change_value(PropertyIdentifier::AUDIT_LEVEL),
            None,
        )
        .await
        .unwrap();
    settle().await;
    assert_eq!(f.server.notification_transactions.active_count(), 1);
    assert_eq!(f.server.notification_transactions.audit_resources().2, 63);
    let database = Arc::clone(&f.server.db);
    let mut db = database.write().await;
    assert!(
        tokio::time::timeout(Duration::from_millis(10), f.server.stop())
            .await
            .is_err()
    );
    assert!(db.remove(&oid(ObjectType::DEVICE, 10)).is_err());
    assert_eq!(
        db.get(&oid(ObjectType::BINARY_VALUE, 11))
            .unwrap()
            .audit_object_policy_internal()
            .level,
        Some(AuditLevel::AUDIT_CONFIG)
    );
    assert_eq!(db.reserve_event_sequence_number().number(), 1);
    assert_eq!(f.server.notification_transactions.active_count(), 1);
    assert_eq!(f.server.notification_transactions.audit_resources().2, 63);
    // Cancellation retains the active attempt until the original drain deadline.
    tokio::time::advance(Duration::from_secs(3)).await;
    settle().await;
    assert_eq!(f.server.notification_transactions.active_count(), 0);
    assert_eq!(
        f.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
    drop(db);
    f.server.stop().await.unwrap();
    f.server.stop().await.unwrap();
    assert!(f.server.notification_transactions.workers_empty());
    assert!(database
        .write()
        .await
        .remove(&oid(ObjectType::DEVICE, 10))
        .unwrap()
        .is_some());
}

#[path = "audit_policy_precommit_edge_tests.rs"]
mod edges;
