use super::*;
use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::{
    bitstring::{AuditOperationFlags, BACnetPriorityFilter},
    constructed::{
        AuditPropertyReference, BACnetAddress, BACnetAuditNotification, BACnetRecipient,
    },
    enums::{AuditLevel, AuditOperation, Reliability},
    primitives::BACnetTimeStamp,
};
use std::sync::Mutex as StdMutex;

#[path = "audit_reporter_test_support.rs"]
mod support;
use support::*;

#[path = "audit_recipient_routes_tests.rs"]
mod recipient_routes;

#[path = "audit_recipient_tests.rs"]
mod recipient_changes;

#[path = "audit_reporter_identity_tests.rs"]
mod identity;

#[path = "audit_reporter_startup_tests.rs"]
mod startup;

#[path = "audit_reporter_failure_tests.rs"]
mod failures;

#[path = "audit_reporter_selection_tests.rs"]
mod selection;

#[path = "audit_reporter_property_tests.rs"]
mod selection_properties;

#[path = "audit_reporter_lifecycle_tests.rs"]
mod lifecycle;

#[path = "audit_reporter_list_tests.rs"]
mod list;

#[path = "audit_reporter_file_tests.rs"]
mod file;

#[path = "audit_reporter_resource_tests.rs"]
mod resources;

#[path = "audit_reporter_read_tests.rs"]
mod read;

#[path = "audit_reporter_live_tests.rs"]
mod live;

#[tokio::test(start_paused = true)]
async fn audit_reporter_atomic_write_file_delivery_saturation_deadline_and_no_recursion() {
    use file::{access, file_server, request, SERVICE};
    let mut fixture = file_server(false).await;
    fixture.transport.block.store(true, Ordering::Release);
    for _ in 0..65 {
        assert!(matches!(
            dispatch(
                &fixture.server,
                SERVICE,
                request(oid(ObjectType::FILE, 1), access(false, 0))
            )
            .await,
            Apdu::ComplexAck(_)
        ));
    }
    settle().await;
    assert_eq!(notifications(&fixture.transport.sent).len(), 64);
    assert_eq!(
        health(&fixture.server).await,
        Reliability::COMMUNICATION_FAILURE
    );
    tokio::time::advance(Duration::from_secs(3)).await;
    settle().await;
    assert_eq!(
        notifications(&fixture.transport.sent).len(),
        64,
        "no queue or retries"
    );
    fixture.transport.block.store(false, Ordering::Release);
    assert!(matches!(
        dispatch(
            &fixture.server,
            SERVICE,
            request(oid(ObjectType::FILE, 1), access(false, 0))
        )
        .await,
        Apdu::ComplexAck(_)
    ));
    settle().await;
    assert_eq!(notifications(&fixture.transport.sent).len(), 65);
    assert_eq!(
        health(&fixture.server).await,
        Reliability::NO_FAULT_DETECTED
    );
    fixture.server.stop().await.unwrap();
    assert_eq!(notifications(&fixture.transport.sent).len(), 65);
}

#[tokio::test]
async fn audit_reporter_list_optional_values_are_validated_independently() {
    use bacnet_services::list_manipulation::ListElementRequest;

    // Exercise the representation boundary directly: malformed wire requests
    // are separately tested through dispatch and must never reach this hook.
    // Application NULL is one encoded octet, so these cover exactly 32 and 33.
    let mut fixture = server(reporter()).await;
    let mut audit = audit_reporter::WriteAudit::new(
        &fixture.server.config,
        &fixture.server.network,
        &fixture.server.notification_transactions,
        &fixture.server.device_bindings,
        &fixture.server.comm_state,
        SOURCE,
        None,
        77,
    )
    .await;
    let accepted = vec![0; 32];
    let mut count = 0;
    for omitted in [vec![], vec![0; 33], vec![0x0e, 0xd1, 0, 0x0f]] {
        for omit_target in [false, true] {
            let (delta, current) = if omit_target {
                (omitted.clone(), accepted.clone())
            } else {
                (accepted.clone(), omitted.clone())
            };
            let request = ListElementRequest {
                object_identifier: oid(ObjectType::BINARY_VALUE, 1),
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: Some(7),
                list_of_elements: delta,
            };
            {
                let mut db = fixture.server.db.write().await;
                audit.before_list(
                    &db,
                    &request,
                    Some(&PropertyValue::ApplicationData(current)),
                );
                audit.lifecycle_completed(&mut db, &Ok(()));
            }
            settle().await;
            count += 1;
            let records = notifications(&fixture.transport.sent);
            assert_eq!(records.len(), count, "omission must not drop the record");
            assert_eq!(records[count - 1].notifications.len(), 1);
            let record = &records[count - 1].notifications[0];
            assert_eq!(
                record.target_value,
                (!omit_target).then(|| accepted.clone())
            );
            assert_eq!(record.current_value, omit_target.then(|| accepted.clone()));
            assert_eq!(
                record
                    .target_property
                    .as_ref()
                    .unwrap()
                    .property_array_index,
                Some(7)
            );
            assert_eq!(record.target_priority, None);
            assert_eq!(record.result, None);
        }
    }
    drop(audit);
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_wp_emits_one_success_after_commit() {
    let mut fixture = server(reporter()).await;
    let response = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        wp(
            oid(ObjectType::BINARY_VALUE, 1),
            PropertyIdentifier::PRESENT_VALUE,
            vec![0x91, 1],
            None,
        ),
    )
    .await;
    assert!(matches!(response, Apdu::SimpleAck(_)), "{response:?}");
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].notifications.len(), 1);
    assert_eq!(
        records[0].notifications[0],
        BACnetAuditNotification {
            source_timestamp: None,
            target_timestamp: Some(BACnetTimeStamp::SequenceNumber(0)),
            source_device: BACnetRecipient::Address(BACnetAddress {
                network_number: 0,
                mac_address: MacAddr::from_slice(SOURCE)
            }),
            source_object: None,
            operation: AuditOperation::WRITE,
            source_comment: None,
            target_comment: None,
            invoke_id: Some(77),
            source_user_id: None,
            source_user_role: None,
            target_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 10)),
            target_object: Some(oid(ObjectType::BINARY_VALUE, 1)),
            target_property: Some(AuditPropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None
            }),
            target_priority: Some(16),
            target_value: Some(vec![0x91, 1]),
            current_value: Some(vec![0x91, 0]),
            result: None,
        }
    );
    assert_eq!(fixture.writes.load(Ordering::Acquire), 1);
    assert_eq!(
        health(&fixture.server).await,
        Reliability::NO_FAULT_DETECTED
    );
    fixture.server.stop().await.unwrap();
}

async fn settle() {
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
}

async fn health(server: &BACnetServer<CaptureTransport>) -> Reliability {
    let db = server.db.read().await;
    let value = db
        .get(&oid(ObjectType::AUDIT_REPORTER, 1))
        .unwrap()
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    let PropertyValue::Enumerated(value) = value else {
        panic!("unexpected reliability")
    };
    Reliability::from_raw(value)
}

async fn write_value(server: &BACnetServer<CaptureTransport>, priority: Option<u8>) -> Apdu {
    dispatch(
        server,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        wp(
            oid(ObjectType::BINARY_VALUE, 1),
            PropertyIdentifier::PRESENT_VALUE,
            vec![0x91, 1],
            priority,
        ),
    )
    .await
}

#[tokio::test]
async fn audit_reporter_filters_disabled_write_bit_and_priority_without_filtering_description() {
    for (enabled, write_bit, priority, expected) in [
        (false, true, None, 0),
        (true, false, None, 0),
        (true, true, Some(1), 1),
        (true, true, Some(2), 0),
        (true, true, Some(15), 0),
        (true, true, Some(16), 1),
        (true, true, None, 1),
    ] {
        let mut reporter = reporter();
        if !enabled {
            reporter.set_audit_level(AuditLevel::NONE).unwrap();
        }
        if !write_bit {
            reporter
                .set_auditable_operations(AuditOperationFlags::empty())
                .unwrap();
        }
        reporter
            .set_audit_priority_filter(BACnetPriorityFilter::from_bits(0x8001))
            .unwrap();
        let mut fixture = server(reporter).await;
        assert!(matches!(
            write_value(&fixture.server, priority).await,
            Apdu::SimpleAck(_)
        ));
        settle().await;
        assert_eq!(notifications(&fixture.transport.sent).len(), expected);
        assert_eq!(fixture.writes.load(Ordering::Acquire), 1);
        fixture.server.stop().await.unwrap();
    }
    let mut reporter = reporter();
    reporter
        .set_audit_priority_filter(BACnetPriorityFilter::empty())
        .unwrap();
    let mut fixture = server(reporter).await;
    assert!(matches!(
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            wp(
                oid(ObjectType::BINARY_VALUE, 1),
                PropertyIdentifier::DESCRIPTION,
                vec![0x72, 0, b'x'],
                Some(2)
            )
        )
        .await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].notifications[0].target_priority, None);
    fixture.server.stop().await.unwrap();
}

fn wpm(properties: Vec<BACnetPropertyValue>) -> Bytes {
    let mut bytes = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid(ObjectType::BINARY_VALUE, 1),
            list_of_properties: properties,
        }],
    }
    .encode(&mut bytes)
    .unwrap();
    bytes.freeze()
}

fn element(property: PropertyIdentifier, value: Vec<u8>) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: property,
        property_array_index: None,
        value,
        priority: None,
    }
}

#[tokio::test]
async fn audit_reporter_denied_wp_and_wpm_suffix_have_no_audit_side_effects() {
    let mut fixture = server(reporter()).await;
    fixture.server.config.mutation_policy = crate::mutation::MutationPolicy::DenyAll;
    assert!(matches!(
        write_value(&fixture.server, None).await,
        Apdu::Error(_)
    ));
    settle().await;
    assert!(fixture.transport.sent.lock().unwrap().is_empty());
    assert_eq!(fixture.writes.load(Ordering::Acquire), 0);
    assert_eq!(fixture.attempts.load(Ordering::Acquire), 0);
    fixture.server.config.mutation_policy = crate::mutation::MutationPolicy::Permissive;
    fixture.server.config.mutation_authorizer = Some(Arc::new(|context| match &context.target {
        crate::mutation::MutationTarget::WritePropertyMultiple(attempt) => {
            attempt.reference.property_identifier == PropertyIdentifier::PRESENT_VALUE.to_raw()
        }
        _ => false,
    }));
    assert!(matches!(
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            wpm(vec![
                element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 1]),
                element(PropertyIdentifier::DESCRIPTION, vec![0x72, 0, b'x']),
                element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 0]),
            ])
        )
        .await,
        Apdu::Error(_)
    ));
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].notifications.len(), 1);
    assert_eq!(records[0].notifications[0].result, None);
    assert_eq!(
        records[0].notifications[0].target_value,
        Some(vec![0x91, 1])
    );
    assert_eq!(fixture.writes.load(Ordering::Acquire), 1);
    assert_eq!(fixture.attempts.load(Ordering::Acquire), 1);
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_missing_recipient_is_configuration_failure_without_growth() {
    for recipient in [BACnetRecipient::Device(oid(ObjectType::DEVICE, 999))] {
        let mut fixture = server_with_recipient(reporter(), recipient).await;
        for _ in 0..100 {
            assert!(matches!(
                write_value(&fixture.server, None).await,
                Apdu::SimpleAck(_)
            ));
        }
        settle().await;
        assert_eq!(
            health(&fixture.server).await,
            Reliability::CONFIGURATION_ERROR
        );
        assert!(fixture.transport.sent.lock().unwrap().is_empty());
        assert_eq!(fixture.server.notification_transactions.active_count(), 0);
        assert!(fixture.server.notification_transactions.workers_empty());
        assert_eq!(fixture.writes.load(Ordering::Acquire), 100);
        fixture.server.stop().await.unwrap();
    }
}

fn confirmed_notification(sent: &StdMutex<Vec<Bytes>>, index: usize) -> ConfirmedRequestPdu {
    match decode_apdu(
        decode_npdu(sent.lock().unwrap()[index].clone())
            .unwrap()
            .payload,
    )
    .unwrap()
    {
        Apdu::ConfirmedRequest(request) => {
            assert_eq!(
                request.service_choice,
                ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION
            );
            request
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn audit_reporter_missing_ack_and_failed_delivery_are_communication_failures_and_recover() {
    let mut reporter = reporter();
    reporter.set_issue_confirmed_notifications(true).unwrap();
    let mut fixture = server(reporter).await;
    write_value(&fixture.server, None).await;
    settle().await;
    let first = confirmed_notification(&fixture.transport.sent, 0);
    let ack = Apdu::SimpleAck(SimpleAck {
        invoke_id: first.invoke_id,
        service_choice: first.service_choice,
    });
    assert!(!fixture
        .server
        .notification_transactions
        .admit_terminal(SOURCE, None, &ack));
    tokio::time::advance(Duration::from_secs(3)).await;
    settle().await;
    assert_eq!(
        health(&fixture.server).await,
        Reliability::COMMUNICATION_FAILURE
    );
    assert_eq!(fixture.server.notification_transactions.active_count(), 0);
    fixture.transport.fail.store(true, Ordering::Release);
    write_value(&fixture.server, None).await;
    settle().await;
    tokio::time::advance(Duration::from_secs(3)).await;
    settle().await;
    assert_eq!(
        health(&fixture.server).await,
        Reliability::COMMUNICATION_FAILURE
    );
    fixture.transport.fail.store(false, Ordering::Release);
    write_value(&fixture.server, None).await;
    settle().await;
    let request = confirmed_notification(&fixture.transport.sent, 2);
    assert!(fixture.server.notification_transactions.admit_terminal(
        LOGGER,
        None,
        &Apdu::SimpleAck(SimpleAck {
            invoke_id: request.invoke_id,
            service_choice: request.service_choice
        })
    ));
    settle().await;
    assert_eq!(
        health(&fixture.server).await,
        Reliability::NO_FAULT_DETECTED
    );
    assert_eq!(fixture.writes.load(Ordering::Acquire), 3);
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_self_write_is_one_notification_and_sensor_sampling_is_excluded() {
    let mut reporter = reporter();
    reporter
        .set_auditable_operations(AuditOperationFlags::empty())
        .unwrap();
    let mut fixture = server(reporter).await;
    assert!(matches!(
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            wp(
                oid(ObjectType::AUDIT_REPORTER, 1),
                PropertyIdentifier::DESCRIPTION,
                vec![0x72, 0, b'x'],
                None
            )
        )
        .await,
        Apdu::SimpleAck(_)
    ));
    fixture
        .server
        .set_present_value_local(&oid(ObjectType::ANALOG_INPUT, 1), PropertyValue::Real(12.0))
        .await
        .unwrap();
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].notifications[0].target_object,
        Some(oid(ObjectType::AUDIT_REPORTER, 1))
    );
    assert_eq!(
        health(&fixture.server).await,
        Reliability::NO_FAULT_DETECTED
    );
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_overflow_and_shutdown_with_inflight_send_are_bounded() {
    let mut reporter = reporter();
    reporter.set_issue_confirmed_notifications(true).unwrap();
    let mut fixture = server(reporter).await;
    fixture.transport.block.store(true, Ordering::Release);
    let response = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        wpm((0..65)
            .map(|_| element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 1]))
            .collect()),
    )
    .await;
    assert!(matches!(response, Apdu::SimpleAck(_)));
    settle().await;
    assert_eq!(fixture.writes.load(Ordering::Acquire), 65);
    assert_eq!(fixture.transport.sent.lock().unwrap().len(), 64);
    assert_eq!(fixture.server.notification_transactions.active_count(), 64);
    assert_eq!(
        health(&fixture.server).await,
        Reliability::COMMUNICATION_FAILURE
    );
    tokio::time::timeout(Duration::from_secs(1), fixture.server.stop())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fixture.server.notification_transactions.active_count(), 0);
    assert!(fixture.server.notification_transactions.workers_empty());
}

#[tokio::test(start_paused = true)]
async fn audit_reporter_blocked_transport_has_a_total_deadline_even_without_ack_wait() {
    for confirmed in [false, true] {
        let mut reporter = reporter();
        reporter
            .set_issue_confirmed_notifications(confirmed)
            .unwrap();
        let mut fixture = server(reporter).await;
        fixture.transport.block.store(true, Ordering::Release);
        write_value(&fixture.server, None).await;
        settle().await;
        assert_eq!(fixture.transport.sent.lock().unwrap().len(), 1);
        tokio::time::advance(Duration::from_secs(3)).await;
        settle().await;
        assert_eq!(
            health(&fixture.server).await,
            Reliability::COMMUNICATION_FAILURE
        );
        assert_eq!(fixture.server.notification_transactions.active_count(), 0);
        assert!(fixture.server.notification_transactions.workers_empty());
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_known_source_array_coordinate_and_large_value_policy() {
    let mut fixture = server(reporter()).await;
    fixture
        .server
        .device_bindings
        .write()
        .await
        .insert_configured(
            DeviceBinding::local(oid(ObjectType::DEVICE, 30), SOURCE).unwrap(),
            |_| false,
        )
        .unwrap();
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid(ObjectType::BINARY_VALUE, 1),
        property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
        property_array_index: Some(8),
        property_value: vec![0x91, 1],
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    assert!(matches!(
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            request.freeze()
        )
        .await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1);
    let record = &records[0].notifications[0];
    assert_eq!(
        record.source_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 30))
    );
    assert_eq!(
        record
            .target_property
            .as_ref()
            .unwrap()
            .property_array_index,
        Some(8)
    );
    assert_eq!(record.current_value, Some(vec![0]));
    assert_eq!(record.target_priority, None);
    for size in [29, 30, 31, 1000] {
        let mut encoded = BytesMut::new();
        encode_property_value(
            &mut encoded,
            &PropertyValue::CharacterString("x".repeat(size)),
        )
        .unwrap();
        let included = (encoded.len() <= 32).then(|| encoded.to_vec());
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            wp(
                oid(ObjectType::BINARY_VALUE, 1),
                PropertyIdentifier::DESCRIPTION,
                encoded.to_vec(),
                None,
            ),
        )
        .await;
        settle().await;
        assert_eq!(
            notifications(&fixture.transport.sent)
                .last()
                .unwrap()
                .notifications[0]
                .target_value,
            included
        );
    }
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_noncommandable_present_value_ignores_priority_filter() {
    let mut reporter = reporter();
    reporter
        .set_audit_priority_filter(BACnetPriorityFilter::empty())
        .unwrap();
    let mut fixture = server(reporter).await;
    fixture
        .server
        .write_local(
            &oid(ObjectType::ANALOG_INPUT, 1),
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .await
        .unwrap();
    let response = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        wp(
            oid(ObjectType::ANALOG_INPUT, 1),
            PropertyIdentifier::PRESENT_VALUE,
            vec![0x44, 0x41, 0x20, 0, 0],
            Some(8),
        ),
    )
    .await;
    assert!(matches!(response, Apdu::SimpleAck(_)));
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].notifications[0].invoke_id, None);
    assert_eq!(
        records[0].notifications[0]
            .target_property
            .as_ref()
            .unwrap()
            .property_identifier,
        PropertyIdentifier::OUT_OF_SERVICE
    );
    assert_eq!(records[1].notifications[0].target_priority, None);
    assert_eq!(
        records[1].notifications[0].target_value,
        Some(vec![0x44, 0x41, 0x20, 0, 0])
    );
    fixture.server.stop().await.unwrap();
}

#[path = "audit_object_policy_tests.rs"]
mod object_policy;
