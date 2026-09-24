use super::*;
use bacnet_objects::{
    audit::AuditReporterObject,
    binary::BinaryValueObject,
    device::{DeviceConfig, DeviceObject},
};

#[tokio::test]
async fn target_reporter_live_enable_emits_one_local_change() {
    let mut object = reporter();
    object.set_audit_level(AuditLevel::NONE).unwrap();
    let mut fixture = server(object).await;
    let reporter = oid(ObjectType::AUDIT_REPORTER, 1);
    let mut operations = AuditOperationFlags::empty();
    operations.insert(AuditOperation::WRITE);
    fixture
        .server
        .db
        .write()
        .await
        .get_mut(&reporter)
        .unwrap()
        .configure_audit_reporter_internal(
            AuditLevel::AUDIT_ALL,
            operations,
            false,
            None,
            BACnetPriorityFilter::all(),
        )
        .unwrap();
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(
        records.len(),
        1,
        "live local NONE-to-enabled must report the actual change"
    );
    assert_eq!(records[0].notifications[0].target_object, Some(reporter));
    assert_eq!(
        records[0].notifications[0]
            .target_property
            .as_ref()
            .unwrap()
            .property_identifier,
        PropertyIdentifier::AUDIT_LEVEL
    );
    assert_eq!(
        records[0].notifications[0].source_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 10))
    );
    assert_eq!(records[0].notifications[0].invoke_id, None);
    fixture.server.stop().await.unwrap();
}

use bacnet_types::constructed::BACnetObjectSelector as Selector;

fn configured(instance: u32, selectors: Option<Vec<Selector>>, write: bool) -> AuditReporterObject {
    let mut object = AuditReporterObject::new(instance, format!("Reporter {instance}")).unwrap();
    object.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
    let mut flags = AuditOperationFlags::empty();
    if write {
        flags.insert(AuditOperation::WRITE);
    }
    flags.insert(AuditOperation::AUDITING_FAILURE);
    object.set_auditable_operations(flags).unwrap();
    object.set_monitored_objects(selectors).unwrap();
    object
}

async fn plural(reporters: Vec<AuditReporterObject>) -> Fixture {
    try_servers(
        reporters,
        &[10],
        Some(BACnetRecipient::Device(oid(ObjectType::DEVICE, 20))),
        vec![DeviceBinding::local(oid(ObjectType::DEVICE, 20), LOGGER).unwrap()],
    )
    .await
    .unwrap()
}

fn records(fixture: &Fixture) -> Vec<BACnetAuditNotification> {
    fixture
        .transport
        .sent
        .lock()
        .unwrap()
        .iter()
        .flat_map(|bytes| {
            let payload = match decode_apdu(decode_npdu(bytes.clone()).unwrap().payload).unwrap() {
                Apdu::ConfirmedRequest(request) => request.service_request,
                Apdu::UnconfirmedRequest(request) => request.service_request,
                other => panic!("unexpected {other:?}"),
            };
            bacnet_services::audit::AuditNotificationRequest::decode(&payload)
                .unwrap()
                .notifications
        })
        .collect()
}

async fn reliability(fixture: &Fixture, instance: u32) -> Reliability {
    let db = fixture.server.db.read().await;
    let value = db
        .get(&oid(ObjectType::AUDIT_REPORTER, instance))
        .unwrap()
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    let PropertyValue::Enumerated(raw) = value else {
        panic!("expected Reliability")
    };
    Reliability::from_raw(raw)
}

#[tokio::test]
async fn target_reporter_nominal_overlap_elects_lowest_before_write_filters_and_updates_live() {
    let target = oid(ObjectType::BINARY_VALUE, 1);
    // Input order does not influence election. The lower Reporter suppresses
    // WRITE, but its nominal membership still wins and faults both Reporters.
    let mut fixture = plural(vec![
        configured(9, Some(vec![Selector::Object(target)]), true),
        configured(
            2,
            Some(vec![Selector::ObjectType(ObjectType::BINARY_VALUE)]),
            false,
        ),
    ])
    .await;
    for instance in [2, 9] {
        assert_eq!(
            reliability(&fixture, instance).await,
            Reliability::CONFIGURATION_ERROR
        );
    }
    assert!(matches!(
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            wp(
                target,
                PropertyIdentifier::DESCRIPTION,
                vec![0x72, 0, b'x'],
                None
            )
        )
        .await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    assert!(records(&fixture).is_empty());
    fixture
        .server
        .db
        .write()
        .await
        .get_mut(&oid(ObjectType::AUDIT_REPORTER, 2))
        .unwrap()
        .configure_audit_reporter_internal(
            AuditLevel::NONE,
            AuditOperationFlags::empty(),
            false,
            Some(vec![Selector::ObjectType(ObjectType::BINARY_VALUE)]),
            BACnetPriorityFilter::all(),
        )
        .unwrap();
    settle().await;
    // The disabling Reporter self-reports mandatory changed properties, even
    // though its nominal selectors exclude Reporter objects and WRITE was off.
    assert_eq!(records(&fixture).len(), 2);
    fixture.transport.sent.lock().unwrap().clear();
    assert_eq!(
        reliability(&fixture, 9).await,
        Reliability::NO_FAULT_DETECTED
    );
    assert!(matches!(
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            wp(
                target,
                PropertyIdentifier::DESCRIPTION,
                vec![0x72, 0, b'y'],
                None
            )
        )
        .await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    assert_eq!(records(&fixture).len(), 1);
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn target_reporter_overlap_follows_actual_subject_membership_and_protects_every_owner() {
    let target = oid(ObjectType::BINARY_VALUE, 99);
    let mut fixture = plural(vec![
        configured(8, Some(vec![Selector::Object(target)]), true),
        configured(3, Some(vec![Selector::Object(target)]), true),
    ])
    .await;
    for instance in [3, 8] {
        assert_eq!(
            reliability(&fixture, instance).await,
            Reliability::NO_FAULT_DETECTED
        );
    }
    fixture
        .server
        .db
        .write()
        .await
        .add(Box::new(BinaryValueObject::new(99, "dynamic").unwrap()))
        .unwrap();
    for instance in [3, 8] {
        assert_eq!(
            reliability(&fixture, instance).await,
            Reliability::CONFIGURATION_ERROR
        );
    }
    fixture.server.db.write().await.remove(&target).unwrap();
    for instance in [3, 8] {
        assert_eq!(
            reliability(&fixture, instance).await,
            Reliability::NO_FAULT_DETECTED
        );
    }
    for instance in [3, 8] {
        assert!(fixture
            .server
            .db
            .write()
            .await
            .remove(&oid(ObjectType::AUDIT_REPORTER, instance))
            .is_err());
    }
    assert!(fixture
        .server
        .db
        .write()
        .await
        .add(Box::new(
            DeviceObject::new(DeviceConfig {
                instance: 11,
                ..Default::default()
            })
            .unwrap()
        ))
        .is_err());
    let retained = Arc::clone(fixture.server.database());
    fixture.server.stop().await.unwrap();
    for instance in [3, 8] {
        assert!(retained
            .write()
            .await
            .remove(&oid(ObjectType::AUDIT_REPORTER, instance))
            .unwrap()
            .is_some());
    }
}

#[tokio::test]
async fn target_reporter_aggregate_late_admission_failure_is_atomic_and_releases_prefix() {
    let mut fixture = plural(vec![configured(1, Some(vec![]), false)]).await;
    let target = oid(ObjectType::AUDIT_REPORTER, 1);
    let before = fixture
        .server
        .db
        .read()
        .await
        .get(&target)
        .unwrap()
        .audit_reporter_internal()
        .unwrap()
        .configuration_internal();
    let permits: Vec<_> = (0..63)
        .map(|_| {
            fixture
                .server
                .notification_transactions
                .try_admit_audit()
                .unwrap()
        })
        .collect();
    let mut flags = AuditOperationFlags::empty();
    flags.insert(AuditOperation::WRITE);
    let result = fixture
        .server
        .db
        .write()
        .await
        .get_mut(&target)
        .unwrap()
        .configure_audit_reporter_internal(
            AuditLevel::AUDIT_CONFIG,
            flags,
            true,
            None,
            BACnetPriorityFilter::empty(),
        );
    assert!(
        result.is_err(),
        "the second prepared record must fail the shared admission bound"
    );
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (false, 0, 1)
    );
    assert_eq!(fixture.server.notification_transactions.active_count(), 0);
    assert_eq!(
        fixture
            .server
            .db
            .read()
            .await
            .get(&target)
            .unwrap()
            .audit_reporter_internal()
            .unwrap()
            .configuration_internal(),
        before
    );
    assert!(records(&fixture).is_empty());
    drop(permits);
    // Retry uses sequence zero: failed preparation did not consume timestamps.
    fixture
        .server
        .db
        .write()
        .await
        .get_mut(&target)
        .unwrap()
        .configure_audit_reporter_internal(
            AuditLevel::AUDIT_CONFIG,
            flags,
            false,
            None,
            BACnetPriorityFilter::empty(),
        )
        .unwrap();
    settle().await;
    let records = records(&fixture);
    assert_eq!(records.len(), 4);
    let properties: Vec<_> = records
        .iter()
        .map(|record| {
            record
                .target_property
                .as_ref()
                .unwrap()
                .property_identifier
                .to_raw()
        })
        .collect();
    assert!(properties.windows(2).all(|pair| pair[0] < pair[1]));
    for (index, record) in records.iter().enumerate() {
        assert_eq!(
            record.target_timestamp,
            Some(BACnetTimeStamp::SequenceNumber(index as u16))
        );
        assert_eq!(
            record.source_device,
            BACnetRecipient::Device(oid(ObjectType::DEVICE, 10))
        );
        assert_eq!(record.invoke_id, None);
    }
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn target_reporter_description_actual_change_and_noop_keep_remote_identity() {
    let mut fixture = plural(vec![configured(1, Some(vec![]), false)]).await;
    let target = oid(ObjectType::AUDIT_REPORTER, 1);
    for _ in 0..2 {
        assert!(matches!(
            dispatch(
                &fixture.server,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                wp(
                    target,
                    PropertyIdentifier::DESCRIPTION,
                    vec![0x72, 0, b'x'],
                    None
                )
            )
            .await,
            Apdu::SimpleAck(_)
        ));
    }
    settle().await;
    let remote = records(&fixture);
    assert_eq!(
        remote.len(),
        2,
        "one record per network attempt, without duplicate actual-change capture"
    );
    assert!(matches!(
        remote[0].source_device,
        BACnetRecipient::Address(_)
    ));
    assert!(remote[0].invoke_id.is_some());
    fixture
        .server
        .db
        .write()
        .await
        .get_mut(&target)
        .unwrap()
        .write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("local".into()),
            None,
        )
        .unwrap();
    settle().await;
    let local = records(&fixture);
    assert_eq!(local.len(), 3);
    assert_eq!(
        local[2].source_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 10))
    );
    assert_eq!(local[2].invoke_id, None);
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn target_reporter_recipient_pair_has_one_elected_owner_and_fences_every_context() {
    for all_disabled in [false, true] {
        let mut low = configured(
            1,
            Some(vec![Selector::ObjectType(ObjectType::BINARY_VALUE)]),
            true,
        );
        let mut high = configured(
            2,
            Some(vec![Selector::ObjectType(ObjectType::DEVICE)]),
            true,
        );
        high.set_issue_confirmed_notifications(true).unwrap();
        if all_disabled {
            low.set_audit_level(AuditLevel::NONE).unwrap();
            high.set_audit_level(AuditLevel::NONE).unwrap();
        }
        let mut fixture = try_servers(
            vec![high, low],
            &[10],
            Some(BACnetRecipient::Device(oid(ObjectType::DEVICE, 20))),
            vec![
                DeviceBinding::local(oid(ObjectType::DEVICE, 20), LOGGER).unwrap(),
                DeviceBinding::local(oid(ObjectType::DEVICE, 21), NEW_LOGGER).unwrap(),
            ],
        )
        .await
        .unwrap();
        let statuses: Vec<_> = {
            let db = fixture.server.db.read().await;
            db.find_by_type(ObjectType::AUDIT_REPORTER)
                .iter()
                .map(|id| {
                    db.get(id)
                        .unwrap()
                        .audit_reporter_internal()
                        .unwrap()
                        .status_internal()
                })
                .collect()
        };
        let before: Vec<_> = statuses
            .iter()
            .map(|status| status.begin_delivery())
            .collect();
        let mut value = BytesMut::new();
        bacnet_encoding::constructed::encode_recipient(
            &mut value,
            &BACnetRecipient::Device(oid(ObjectType::DEVICE, 21)),
        );
        fixture
            .server
            .write_local(
                &oid(ObjectType::DEVICE, 10),
                PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                None,
                PropertyValue::ApplicationData(value.to_vec()),
                None,
            )
            .await
            .unwrap();
        settle().await;
        assert_eq!(
            records(&fixture).len(),
            2,
            "one pair independent of configured Reporter count"
        );
        assert_eq!(
            *fixture.transport.destinations.lock().unwrap(),
            vec![LOGGER.to_vec(), NEW_LOGGER.to_vec()]
        );
        for bytes in fixture.transport.sent.lock().unwrap().iter() {
            assert_eq!(
                matches!(
                    decode_apdu(decode_npdu(bytes.clone()).unwrap().payload).unwrap(),
                    Apdu::ConfirmedRequest(_)
                ),
                !all_disabled
            );
        }
        for (status, previous) in statuses.iter().zip(before) {
            assert_ne!(status.begin_delivery(), previous);
        }
        fixture.server.stop().await.unwrap();
        assert_eq!(fixture.server.notification_transactions.active_count(), 0);
    }
}

#[tokio::test]
async fn target_reporter_plural_drop_keeps_all_memberships_until_task_frames_end() {
    let fixture = plural(vec![configured(1, None, true), configured(2, None, true)]).await;
    let db = Arc::clone(fixture.server.database());
    let mut held = db.write().await;
    drop(fixture);
    for instance in [1, 2] {
        assert!(held
            .remove(&oid(ObjectType::AUDIT_REPORTER, instance))
            .is_err());
    }
    drop(held);
    settle().await;
    let mut held = db.write().await;
    for instance in [1, 2] {
        assert!(held
            .remove(&oid(ObjectType::AUDIT_REPORTER, instance))
            .unwrap()
            .is_some());
    }
}

#[tokio::test(start_paused = true)]
async fn target_reporter_plural_cancelled_stop_rejects_changes_until_joined_uninstall() {
    let mut fixture = plural(vec![
        configured(1, Some(vec![]), true),
        configured(2, Some(vec![]), true),
    ])
    .await;
    fixture.transport.block.store(true, Ordering::Release);
    fixture
        .server
        .write_local(
            &oid(ObjectType::AUDIT_REPORTER, 2),
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("pending".into()),
            None,
        )
        .await
        .unwrap();
    settle().await;
    let database = Arc::clone(fixture.server.database());
    let mut held = database.write().await;
    assert!(
        tokio::time::timeout(Duration::from_millis(10), fixture.server.stop())
            .await
            .is_err()
    );
    for instance in [1, 2] {
        let target = oid(ObjectType::AUDIT_REPORTER, instance);
        assert!(held.remove(&target).is_err());
        assert!(held
            .get_mut(&target)
            .unwrap()
            .write_property(
                PropertyIdentifier::DESCRIPTION,
                None,
                PropertyValue::CharacterString("after seal".into()),
                None
            )
            .is_err());
    }
    drop(held);
    fixture.server.stop().await.unwrap();
    fixture.server.stop().await.unwrap();
    assert!(fixture.server.notification_transactions.workers_empty());
    let mut held = database.write().await;
    for instance in [1, 2] {
        let target = oid(ObjectType::AUDIT_REPORTER, instance);
        held.get_mut(&target)
            .unwrap()
            .write_property(
                PropertyIdentifier::DESCRIPTION,
                None,
                PropertyValue::CharacterString("inactive authoring".into()),
                None,
            )
            .unwrap();
        assert!(held.remove(&target).unwrap().is_some());
    }
}

#[tokio::test]
async fn target_reporter_none_property_change_can_use_other_nominal_reporter_but_not_self() {
    for other_write in [false, true] {
        let mut target = configured(1, Some(vec![]), true);
        target.set_audit_level(AuditLevel::NONE).unwrap();
        let mut fixture = plural(vec![
            target,
            configured(
                2,
                Some(vec![Selector::Object(oid(ObjectType::AUDIT_REPORTER, 1))]),
                other_write,
            ),
        ])
        .await;
        fixture
            .server
            .db
            .write()
            .await
            .get_mut(&oid(ObjectType::AUDIT_REPORTER, 1))
            .unwrap()
            .write_property(
                PropertyIdentifier::DESCRIPTION,
                None,
                PropertyValue::CharacterString("changed".into()),
                None,
            )
            .unwrap();
        settle().await;
        assert_eq!(records(&fixture).len(), usize::from(other_write));
        for instance in [1, 2] {
            assert_eq!(
                reliability(&fixture, instance).await,
                Reliability::NO_FAULT_DETECTED
            );
        }
        fixture.server.stop().await.unwrap();
    }
}

#[path = "audit_reporter_description_tests.rs"]
mod description;
