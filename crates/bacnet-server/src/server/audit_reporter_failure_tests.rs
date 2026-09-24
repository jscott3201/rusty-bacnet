//! Execution failures use the same bounded target-WRITE path as successes.

use super::*;

pub(super) fn expected_value_write(
    value: u8,
    current: u8,
    sequence: u16,
    result: Option<(ErrorClass, ErrorCode)>,
) -> BACnetAuditNotification {
    BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: Some(BACnetTimeStamp::SequenceNumber(sequence)),
        source_device: BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(SOURCE),
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
            property_array_index: None,
        }),
        target_priority: Some(16),
        target_value: Some(vec![0x91, value]),
        current_value: Some(vec![0x91, current]),
        result,
    }
}

async fn failed_value_write(server: &BACnetServer<CaptureTransport>, priority: Option<u8>) -> Apdu {
    dispatch(
        server,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        wp(
            oid(ObjectType::BINARY_VALUE, 1),
            PropertyIdentifier::PRESENT_VALUE,
            vec![0x91, 9],
            priority,
        ),
    )
    .await
}

fn assert_wp_error(response: Apdu, class: ErrorClass, code: ErrorCode) {
    let Apdu::Error(error) = response else {
        panic!("expected WP Error, got {response:?}");
    };
    assert_eq!(error.service_choice, ConfirmedServiceChoice::WRITE_PROPERTY);
    assert_eq!((error.error_class, error.error_code), (class, code));
    assert!(error.error_data.is_empty());
}

#[tokio::test]
async fn audit_reporter_wp_execution_error_has_exact_result_without_mutation() {
    let mut fixture = server(reporter()).await;
    let response = failed_value_write(&fixture.server, None).await;
    assert_wp_error(
        response,
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].notifications,
        vec![expected_value_write(
            9,
            0,
            0,
            Some((ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)),
        )]
    );
    assert_eq!(fixture.writes.load(Ordering::Acquire), 0);
    assert_eq!(fixture.attempts.load(Ordering::Acquire), 1);
    assert_eq!(
        fixture
            .server
            .db
            .read()
            .await
            .get(&oid(ObjectType::BINARY_VALUE, 1))
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(0)
    );
    assert_eq!(
        health(&fixture.server).await,
        Reliability::NO_FAULT_DETECTED
    );
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_wpm_reports_committed_prefix_one_execution_failure_and_halts() {
    let mut fixture = server(reporter()).await;
    let response = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        wpm(vec![
            element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 1]),
            element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 0]),
            element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 9]),
            element(PropertyIdentifier::DESCRIPTION, vec![0x72, 0, b'x']),
        ]),
    )
    .await;
    let Apdu::Error(error) = response else {
        panic!("expected WPM Error, got {response:?}");
    };
    assert_eq!(error.invoke_id, 77);
    let error = bacnet_services::wpm::WritePropertyMultipleError::from_error_pdu(&error).unwrap();
    assert_eq!(
        (error.error_class, error.error_code),
        (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)
    );
    assert_eq!(
        error.first_failed_write_attempt,
        bacnet_types::constructed::BACnetObjectPropertyReference {
            object_identifier: oid(ObjectType::BINARY_VALUE, 1),
            property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
            property_array_index: None,
        }
    );
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 3);
    for (request, expected) in records.iter().zip([
        expected_value_write(1, 0, 0, None),
        expected_value_write(0, 1, 1, None),
        expected_value_write(
            9,
            0,
            2,
            Some((ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)),
        ),
    ]) {
        assert_eq!(request.notifications, vec![expected]);
    }
    assert_eq!(fixture.writes.load(Ordering::Acquire), 2);
    assert_eq!(fixture.attempts.load(Ordering::Acquire), 3);
    {
        let db = fixture.server.db.read().await;
        let object = db.get(&oid(ObjectType::BINARY_VALUE, 1)).unwrap();
        assert_eq!(
            object
                .read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString(String::new())
        );
    }
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_execution_failures_obey_level_operation_and_priority_filters() {
    for multiple in [false, true] {
        for (level, write_bit, priority, expected) in [
            (AuditLevel::NONE, true, None, 0),
            (AuditLevel::AUDIT_CONFIG, true, None, 0),
            (AuditLevel::AUDIT_ALL, false, None, 0),
            (AuditLevel::AUDIT_ALL, true, Some(1), 1),
            (AuditLevel::AUDIT_ALL, true, Some(2), 0),
            (AuditLevel::AUDIT_ALL, true, Some(15), 0),
            (AuditLevel::AUDIT_ALL, true, Some(16), 1),
            (AuditLevel::AUDIT_ALL, true, None, 1),
            (AuditLevel::from_raw(128), true, None, 1),
        ] {
            let mut reporter = reporter();
            reporter.set_audit_level(level).unwrap();
            if !write_bit {
                reporter
                    .set_auditable_operations(AuditOperationFlags::empty())
                    .unwrap();
            }
            reporter
                .set_audit_priority_filter(BACnetPriorityFilter::from_bits(0x8001))
                .unwrap();
            let mut fixture = server(reporter).await;
            let response = if multiple {
                let mut property = element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 9]);
                property.priority = priority;
                dispatch(
                    &fixture.server,
                    ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
                    wpm(vec![
                        property,
                        element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 1]),
                    ]),
                )
                .await
            } else {
                failed_value_write(&fixture.server, priority).await
            };
            let Apdu::Error(error) = response else {
                panic!("expected Error, got {response:?}")
            };
            assert_eq!(
                (error.error_class, error.error_code),
                (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)
            );
            settle().await;
            let records = notifications(&fixture.transport.sent);
            assert_eq!(
                records.len(),
                expected,
                "{level:?}, {write_bit}, {priority:?}, {multiple}"
            );
            if expected == 1 {
                let mut expected = expected_value_write(
                    9,
                    0,
                    0,
                    Some((ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)),
                );
                expected.target_priority = Some(priority.unwrap_or(16));
                assert_eq!(records[0].notifications, vec![expected]);
            }
            assert_eq!(fixture.attempts.load(Ordering::Acquire), 1);
            assert_eq!(fixture.writes.load(Ordering::Acquire), 0);
            assert!(fixture.server.notification_transactions.workers_empty());
            fixture.server.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn audit_reporter_noncommandable_execution_failure_ignores_priority_filter() {
    let mut reporter = reporter();
    reporter.set_audit_level(AuditLevel::AUDIT_CONFIG).unwrap();
    reporter
        .set_audit_priority_filter(BACnetPriorityFilter::empty())
        .unwrap();
    let mut fixture = server(reporter).await;
    let response = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        wp(
            oid(ObjectType::BINARY_VALUE, 1),
            PropertyIdentifier::DESCRIPTION,
            vec![0x91, 9],
            Some(2),
        ),
    )
    .await;
    assert_wp_error(response, ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE);
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1);
    let mut expected = expected_value_write(
        9,
        0,
        0,
        Some((ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE)),
    );
    expected
        .target_property
        .as_mut()
        .unwrap()
        .property_identifier = PropertyIdentifier::DESCRIPTION;
    expected.target_priority = None;
    expected.current_value = Some(vec![0x71, 0]);
    assert_eq!(records[0].notifications, vec![expected]);
    assert_eq!(fixture.attempts.load(Ordering::Acquire), 1);
    assert_eq!(fixture.writes.load(Ordering::Acquire), 0);
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_policy_denial_of_invalid_values_never_enters_execution() {
    for deny_all in [false, true] {
        let mut fixture = server(reporter()).await;
        if deny_all {
            fixture.server.config.mutation_policy = crate::mutation::MutationPolicy::DenyAll;
        } else {
            fixture.server.config.mutation_authorizer = Some(Arc::new(|_| false));
        }
        assert!(matches!(
            failed_value_write(&fixture.server, None).await,
            Apdu::Error(_)
        ));
        assert!(matches!(
            dispatch(
                &fixture.server,
                ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
                wpm(vec![
                    element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 9]),
                    element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 1])
                ])
            )
            .await,
            Apdu::Error(_)
        ));
        settle().await;
        assert!(notifications(&fixture.transport.sent).is_empty());
        assert_eq!(fixture.attempts.load(Ordering::Acquire), 0);
        assert_eq!(fixture.writes.load(Ordering::Acquire), 0);
        assert_eq!(fixture.server.notification_transactions.active_count(), 0);
        assert!(fixture.server.notification_transactions.workers_empty());
        assert_eq!(
            health(&fixture.server).await,
            Reliability::NO_FAULT_DETECTED
        );
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_failed_self_write_and_delivery_do_not_recurse() {
    let mut reporter = reporter();
    reporter
        .set_auditable_operations(AuditOperationFlags::empty())
        .unwrap();
    let mut fixture = server(reporter).await;
    fixture.transport.fail.store(true, Ordering::Release);
    let response = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        wp(
            oid(ObjectType::AUDIT_REPORTER, 1),
            PropertyIdentifier::AUDIT_SOURCE_REPORTER,
            vec![0x11],
            None,
        ),
    )
    .await;
    assert_wp_error(
        response,
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    fixture
        .server
        .set_present_value_local(&oid(ObjectType::ANALOG_INPUT, 1), PropertyValue::Real(12.0))
        .await
        .unwrap();
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1);
    let mut expected = expected_value_write(
        0,
        0,
        0,
        Some((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)),
    );
    expected.target_object = Some(oid(ObjectType::AUDIT_REPORTER, 1));
    expected
        .target_property
        .as_mut()
        .unwrap()
        .property_identifier = PropertyIdentifier::AUDIT_SOURCE_REPORTER;
    expected.target_priority = None;
    expected.target_value = Some(vec![0x11]);
    expected.current_value = Some(vec![0x10]);
    assert_eq!(records[0].notifications, vec![expected]);
    assert_eq!(
        health(&fixture.server).await,
        Reliability::COMMUNICATION_FAILURE
    );
    assert_eq!(fixture.server.notification_transactions.active_count(), 0);
    assert!(fixture.server.notification_transactions.workers_empty());
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_execution_failures_without_recipient_do_not_accumulate() {
    for recipient in [BACnetRecipient::Device(oid(ObjectType::DEVICE, 999))] {
        let mut fixture = server_with_recipient(reporter(), recipient).await;
        for _ in 0..100 {
            assert_wp_error(
                failed_value_write(&fixture.server, None).await,
                ErrorClass::PROPERTY,
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
        }
        settle().await;
        assert!(notifications(&fixture.transport.sent).is_empty());
        assert_eq!(fixture.attempts.load(Ordering::Acquire), 100);
        assert_eq!(fixture.writes.load(Ordering::Acquire), 0);
        assert_eq!(
            health(&fixture.server).await,
            Reliability::CONFIGURATION_ERROR
        );
        assert_eq!(fixture.server.notification_transactions.active_count(), 0);
        assert!(fixture.server.notification_transactions.workers_empty());
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn audit_reporter_execution_failures_saturate_without_queue_retry_or_recursive_records() {
    for confirmed in [false, true] {
        let mut reporter = reporter();
        reporter
            .set_issue_confirmed_notifications(confirmed)
            .unwrap();
        let mut fixture = server(reporter).await;
        fixture.transport.block.store(true, Ordering::Release);
        for _ in 0..100 {
            assert_wp_error(
                failed_value_write(&fixture.server, None).await,
                ErrorClass::PROPERTY,
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
        }
        settle().await;
        assert_eq!(fixture.transport.sent.lock().unwrap().len(), 64);
        assert_eq!(
            fixture.server.notification_transactions.active_count(),
            if confirmed { 64 } else { 0 }
        );
        assert!(!fixture.server.notification_transactions.workers_empty());
        assert_eq!(
            health(&fixture.server).await,
            Reliability::COMMUNICATION_FAILURE
        );
        assert_eq!(fixture.attempts.load(Ordering::Acquire), 100);
        assert_eq!(fixture.writes.load(Ordering::Acquire), 0);
        // One deadline covers blocked transport and ACK wait. Nothing may be
        // queued to run when the admitted workers release their slots.
        tokio::time::advance(Duration::from_secs(3)).await;
        settle().await;
        fixture.transport.block.store(false, Ordering::Release);
        tokio::time::advance(Duration::from_secs(30)).await;
        settle().await;
        assert_eq!(fixture.transport.sent.lock().unwrap().len(), 64);
        assert_eq!(fixture.server.notification_transactions.active_count(), 0);
        assert!(fixture.server.notification_transactions.workers_empty());
        // Admission is reusable; earlier failure records are never replayed.
        assert_wp_error(
            failed_value_write(&fixture.server, None).await,
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        settle().await;
        assert_eq!(fixture.transport.sent.lock().unwrap().len(), 65);
        fixture.server.stop().await.unwrap();
        assert_eq!(fixture.server.notification_transactions.active_count(), 0);
        assert!(fixture.server.notification_transactions.workers_empty());
    }
}

#[tokio::test]
async fn audit_reporter_execution_error_mapping_matches_response_but_unknown_outcomes_are_silent() {
    for (error, expected) in [
        (
            Error::Protocol {
                class: 128,
                code: 512,
            },
            Some((ErrorClass::from_raw(128), ErrorCode::from_raw(512))),
        ),
        (
            Error::OutOfRange("injected execution failure".into()),
            Some((ErrorClass::SERVICES, ErrorCode::OTHER)),
        ),
        (Error::Timeout(Duration::from_secs(1)), None),
        (
            Error::Reject {
                reason: RejectReason::OTHER.to_raw(),
            },
            None,
        ),
        (
            Error::Abort {
                reason: AbortReason::OTHER.to_raw(),
            },
            None,
        ),
    ] {
        let mut fixture = server(reporter()).await;
        let rejected = matches!(error, Error::Reject { .. });
        *fixture.execution_error.lock().unwrap() = Some(error);
        let response = write_value(&fixture.server, None).await;
        if rejected {
            assert!(matches!(
                response,
                Apdu::Reject(RejectPdu {
                    reject_reason: RejectReason::OTHER,
                    ..
                })
            ));
        } else {
            let (class, code) = expected.unwrap_or((ErrorClass::SERVICES, ErrorCode::OTHER));
            assert_wp_error(response, class, code);
        }
        settle().await;
        let records = notifications(&fixture.transport.sent);
        assert_eq!(records.len(), usize::from(expected.is_some()));
        if expected.is_some() {
            assert_eq!(
                records[0].notifications,
                vec![expected_value_write(1, 0, 0, expected)]
            );
        }
        assert_eq!(fixture.attempts.load(Ordering::Acquire), 1);
        assert_eq!(fixture.writes.load(Ordering::Acquire), 0);
        assert_eq!(
            health(&fixture.server).await,
            Reliability::NO_FAULT_DETECTED
        );
        fixture.server.stop().await.unwrap();
    }
}
