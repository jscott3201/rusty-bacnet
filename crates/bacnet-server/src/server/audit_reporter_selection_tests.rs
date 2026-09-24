//! Monitored_Objects filters records, never execution or completion outcomes.

use super::*;
use bacnet_types::constructed::BACnetObjectSelector as Selector;

fn expected_description(
    target: ObjectIdentifier,
    invoke: u8,
    sequence: u16,
    failed: bool,
) -> BACnetAuditNotification {
    let mut record = failures::expected_value_write(1, 0, sequence, None);
    record.target_object = Some(target);
    record.target_property.as_mut().unwrap().property_identifier = PropertyIdentifier::DESCRIPTION;
    record.target_priority = None;
    record.invoke_id = Some(invoke);
    record.current_value = Some(if failed {
        vec![0x72, 0, b'x']
    } else {
        vec![0x71, 0]
    });
    record.target_value = Some(if failed {
        vec![0x91, 9]
    } else {
        vec![0x72, 0, b'x']
    });
    record.result = failed.then_some((ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE));
    record
}

#[tokio::test]
async fn audit_reporter_selection_absent_exact_type_mixed_and_duplicates_have_exact_wp_records() {
    let first = oid(ObjectType::BINARY_VALUE, 1);
    let second = oid(ObjectType::BINARY_VALUE, 2);
    let input = oid(ObjectType::ANALOG_INPUT, 1);
    for (selection, selected) in [
        (None, vec![first, second, input]),
        (Some(vec![Selector::Object(first)]), vec![first]),
        (
            Some(vec![Selector::ObjectType(ObjectType::BINARY_VALUE)]),
            vec![first, second],
        ),
        (
            Some(vec![Selector::ObjectType(ObjectType::ANALOG_INPUT)]),
            vec![input],
        ),
        (
            Some(vec![
                Selector::None,
                Selector::Object(first),
                Selector::ObjectType(ObjectType::ANALOG_INPUT),
            ]),
            vec![first, input],
        ),
        (
            Some(vec![
                Selector::Object(first),
                Selector::Object(first),
                Selector::ObjectType(ObjectType::BINARY_VALUE),
                Selector::ObjectType(ObjectType::BINARY_VALUE),
            ]),
            vec![first, second],
        ),
        (
            Some(vec![Selector::Object(oid(ObjectType::BINARY_VALUE, 99))]),
            vec![],
        ),
    ] {
        let mut reporter = reporter();
        reporter.set_monitored_objects(selection).unwrap();
        let mut fixture = server(reporter).await;
        let mut expected = Vec::new();
        let mut invoke = 77;
        for target in [first, second, input] {
            for failed in [false, true] {
                let response = dispatch(
                    &fixture.server,
                    ConfirmedServiceChoice::WRITE_PROPERTY,
                    wp(
                        target,
                        PropertyIdentifier::DESCRIPTION,
                        if failed {
                            vec![0x91, 9]
                        } else {
                            vec![0x72, 0, b'x']
                        },
                        None,
                    ),
                )
                .await;
                if failed {
                    let Apdu::Error(error) = response else {
                        panic!("expected execution error")
                    };
                    assert_eq!(
                        (error.error_class, error.error_code),
                        (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE)
                    );
                } else {
                    assert!(matches!(response, Apdu::SimpleAck(_)));
                }
                if selected.contains(&target) {
                    expected.push(expected_description(
                        target,
                        invoke,
                        expected.len() as u16,
                        failed,
                    ));
                }
                invoke += 1;
                // Observe each completed delivery; no timing-dependent ordering assumption.
                settle().await;
            }
            assert_eq!(
                fixture
                    .server
                    .db
                    .read()
                    .await
                    .get(&target)
                    .unwrap()
                    .read_property(PropertyIdentifier::DESCRIPTION, None)
                    .unwrap(),
                PropertyValue::CharacterString("x".into())
            );
        }
        let requests = notifications(&fixture.transport.sent);
        assert_eq!(requests.len(), expected.len());
        for (request, record) in requests.iter().zip(expected) {
            assert_eq!(request.notifications, vec![record]);
        }
        assert_eq!(
            health(&fixture.server).await,
            Reliability::NO_FAULT_DETECTED
        );
        assert_eq!(fixture.writes.load(Ordering::Acquire), 1);
        assert_eq!(fixture.attempts.load(Ordering::Acquire), 2);
        fixture.server.stop().await.unwrap();
    }
}

fn multi_target_wpm(specs: Vec<(ObjectIdentifier, Vec<BACnetPropertyValue>)>) -> Bytes {
    let mut bytes = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: specs
            .into_iter()
            .map(
                |(object_identifier, list_of_properties)| WriteAccessSpecification {
                    object_identifier,
                    list_of_properties,
                },
            )
            .collect(),
    }
    .encode(&mut bytes)
    .unwrap();
    bytes.freeze()
}

#[tokio::test]
async fn audit_reporter_selection_wpm_keeps_only_matching_ordered_outcomes() {
    let value = oid(ObjectType::BINARY_VALUE, 1);
    let input = oid(ObjectType::ANALOG_INPUT, 1);
    for selection in [
        vec![Selector::Object(value)],
        vec![Selector::ObjectType(ObjectType::BINARY_VALUE)],
        vec![
            Selector::None,
            Selector::Object(value),
            Selector::Object(value),
            Selector::ObjectType(ObjectType::BINARY_VALUE),
        ],
    ] {
        let mut reporter = reporter();
        reporter.set_monitored_objects(Some(selection)).unwrap();
        let mut fixture = server(reporter).await;
        let response = dispatch(
            &fixture.server,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            multi_target_wpm(vec![
                (
                    input,
                    vec![element(
                        PropertyIdentifier::DESCRIPTION,
                        vec![0x72, 0, b'x'],
                    )],
                ),
                (
                    value,
                    vec![element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 1])],
                ),
                (
                    input,
                    vec![element(
                        PropertyIdentifier::DESCRIPTION,
                        vec![0x72, 0, b'y'],
                    )],
                ),
                (
                    value,
                    vec![
                        element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 0]),
                        element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 9]),
                        element(PropertyIdentifier::DESCRIPTION, vec![0x72, 0, b'z']),
                    ],
                ),
            ]),
        )
        .await;
        let Apdu::Error(error) = response else {
            panic!("expected WPM error")
        };
        let error =
            bacnet_services::wpm::WritePropertyMultipleError::from_error_pdu(&error).unwrap();
        assert_eq!(
            (error.error_class, error.error_code),
            (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)
        );
        assert_eq!(error.first_failed_write_attempt.object_identifier, value);
        assert_eq!(
            error.first_failed_write_attempt.property_identifier,
            PropertyIdentifier::PRESENT_VALUE.to_raw()
        );
        settle().await;
        let requests = notifications(&fixture.transport.sent);
        assert_eq!(requests.len(), 3);
        for (request, expected) in requests.iter().zip([
            failures::expected_value_write(1, 0, 0, None),
            failures::expected_value_write(0, 1, 1, None),
            failures::expected_value_write(
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
            assert_eq!(
                db.get(&input)
                    .unwrap()
                    .read_property(PropertyIdentifier::DESCRIPTION, None)
                    .unwrap(),
                PropertyValue::CharacterString("y".into())
            );
            assert_eq!(
                db.get(&value)
                    .unwrap()
                    .read_property(PropertyIdentifier::DESCRIPTION, None)
                    .unwrap(),
                PropertyValue::CharacterString(String::new())
            );
        }
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_selection_unmatched_wpm_failure_is_silent_and_still_halts() {
    let input = oid(ObjectType::ANALOG_INPUT, 1);
    let mut reporter = reporter();
    reporter
        .set_monitored_objects(Some(vec![Selector::Object(input)]))
        .unwrap();
    let mut fixture = server(reporter).await;
    let response = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        multi_target_wpm(vec![
            (
                input,
                vec![element(
                    PropertyIdentifier::DESCRIPTION,
                    vec![0x72, 0, b'x'],
                )],
            ),
            (
                oid(ObjectType::BINARY_VALUE, 1),
                vec![element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 9])],
            ),
            (
                input,
                vec![element(
                    PropertyIdentifier::DESCRIPTION,
                    vec![0x72, 0, b'y'],
                )],
            ),
        ]),
    )
    .await;
    let Apdu::Error(error) = response else {
        panic!("expected error")
    };
    assert_eq!(
        (error.error_class, error.error_code),
        (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)
    );
    settle().await;
    let requests = notifications(&fixture.transport.sent);
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].notifications,
        vec![expected_description(input, 77, 0, false)]
    );
    assert_eq!(fixture.attempts.load(Ordering::Acquire), 1);
    assert_eq!(fixture.writes.load(Ordering::Acquire), 0);
    assert_eq!(
        fixture
            .server
            .db
            .read()
            .await
            .get(&input)
            .unwrap()
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("x".into())
    );
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_selection_self_write_bypasses_selection_but_not_none_level() {
    for level in [
        AuditLevel::NONE,
        AuditLevel::AUDIT_CONFIG,
        AuditLevel::AUDIT_ALL,
    ] {
        let mut reporter = reporter();
        reporter.set_audit_level(level).unwrap();
        reporter.set_monitored_objects(Some(vec![])).unwrap();
        reporter
            .set_auditable_operations(AuditOperationFlags::empty())
            .unwrap();
        reporter
            .set_audit_priority_filter(BACnetPriorityFilter::empty())
            .unwrap();
        let mut fixture = server(reporter).await;
        let target = oid(ObjectType::AUDIT_REPORTER, 1);
        for failed in [false, true] {
            let response = dispatch(
                &fixture.server,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                wp(
                    target,
                    PropertyIdentifier::DESCRIPTION,
                    if failed {
                        vec![0x91, 9]
                    } else {
                        vec![0x72, 0, b'x']
                    },
                    Some(2),
                ),
            )
            .await;
            assert_eq!(matches!(response, Apdu::Error(_)), failed);
            settle().await;
        }
        let requests = notifications(&fixture.transport.sent);
        if level == AuditLevel::NONE {
            assert!(requests.is_empty());
        } else {
            assert_eq!(requests.len(), 2);
            assert_eq!(
                requests[0].notifications,
                vec![expected_description(target, 77, 0, false)]
            );
            assert_eq!(
                requests[1].notifications,
                vec![expected_description(target, 78, 1, true)]
            );
        }
        assert!(fixture
            .server
            .notification_transactions
            .delivery_workers_idle());
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_selection_empty_and_null_suppress_without_state_or_execution_changes() {
    for selection in [vec![], vec![Selector::None, Selector::None]] {
        let mut reporter = reporter();
        reporter.set_monitored_objects(Some(selection)).unwrap();
        let mut fixture = server(reporter).await;
        // Suppression must not even attempt a delivery that could fault health.
        fixture.transport.fail.store(true, Ordering::Release);
        assert!(matches!(
            write_value(&fixture.server, None).await,
            Apdu::SimpleAck(_)
        ));
        let response = dispatch(
            &fixture.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            wp(
                oid(ObjectType::BINARY_VALUE, 1),
                PropertyIdentifier::PRESENT_VALUE,
                vec![0x91, 9],
                None,
            ),
        )
        .await;
        let Apdu::Error(error) = response else {
            panic!("expected execution error")
        };
        assert_eq!(
            (error.error_class, error.error_code),
            (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)
        );
        settle().await;
        assert!(notifications(&fixture.transport.sent).is_empty());
        assert_eq!(fixture.writes.load(Ordering::Acquire), 1);
        assert_eq!(fixture.attempts.load(Ordering::Acquire), 2);
        assert_eq!(
            health(&fixture.server).await,
            Reliability::NO_FAULT_DETECTED
        );
        assert_eq!(fixture.server.notification_transactions.active_count(), 0);
        assert!(fixture
            .server
            .notification_transactions
            .delivery_workers_idle());
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
            PropertyValue::Enumerated(1)
        );
        fixture.server.stop().await.unwrap();
    }
}
