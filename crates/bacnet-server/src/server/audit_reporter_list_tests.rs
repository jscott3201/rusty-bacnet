use super::*;
use bacnet_objects::multistate::MultiStateInputObject;
use bacnet_services::list_manipulation::ListElementRequest;

fn list_request(
    target: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    delta: Vec<u8>,
) -> Bytes {
    let mut bytes = BytesMut::new();
    // Preserve independently authored malformed inbound fixtures.
    bacnet_encoding::primitives::encode_ctx_object_id(&mut bytes, 0, &target);
    bacnet_encoding::primitives::encode_ctx_enumerated(&mut bytes, 1, property.to_raw());
    if let Some(index) = index {
        bacnet_encoding::primitives::encode_ctx_unsigned(&mut bytes, 2, u64::from(index));
    }
    bytes.extend_from_slice(&[0x3e]);
    bytes.extend_from_slice(&delta);
    bytes.extend_from_slice(&[0x3f]);
    bytes.freeze()
}

async fn list_server(initial: Vec<u32>) -> Fixture {
    let fixture = server(reporter()).await;
    let mut object = MultiStateInputObject::new(1, "list", 3).unwrap();
    object.set_alarm_values(initial);
    fixture
        .server
        .db
        .write()
        .await
        .add(Box::new(object))
        .unwrap();
    fixture
}

async fn values(fixture: &Fixture) -> PropertyValue {
    fixture
        .server
        .db
        .read()
        .await
        .get(&oid(ObjectType::MULTI_STATE_INPUT, 1))
        .unwrap()
        .read_property(PropertyIdentifier::ALARM_VALUES, None)
        .unwrap()
}

#[tokio::test]
async fn audit_reporter_list_success_duplicates_and_noop_preserve_delta_and_preimage() {
    let mut fixture = list_server(vec![1]).await;
    let target = oid(ObjectType::MULTI_STATE_INPUT, 1);
    for (step, (service, delta, before, after)) in [
        (
            ConfirmedServiceChoice::ADD_LIST_ELEMENT,
            vec![0x21, 2, 0x21, 2],
            vec![1],
            vec![1, 2, 2],
        ),
        (
            ConfirmedServiceChoice::REMOVE_LIST_ELEMENT,
            vec![0x21, 2],
            vec![1, 2, 2],
            vec![1],
        ),
        (
            ConfirmedServiceChoice::REMOVE_LIST_ELEMENT,
            vec![0x21, 3],
            vec![1],
            vec![1],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let response = dispatch(
            &fixture.server,
            service,
            list_request(
                target,
                PropertyIdentifier::ALARM_VALUES,
                None,
                delta.clone(),
            ),
        )
        .await;
        assert!(matches!(response, Apdu::SimpleAck(_)), "{response:?}");
        assert_eq!(
            values(&fixture).await,
            PropertyValue::List(after.into_iter().map(PropertyValue::Unsigned).collect())
        );
        settle().await;
        let records = notifications(&fixture.transport.sent);
        assert_eq!(records.len(), step + 1);
        assert_eq!(records[step].notifications.len(), 1);
        assert_eq!(
            records[step].notifications[0],
            BACnetAuditNotification {
                source_timestamp: None,
                target_timestamp: Some(BACnetTimeStamp::SequenceNumber(step as u16)),
                source_device: BACnetRecipient::Address(BACnetAddress {
                    network_number: 0,
                    mac_address: MacAddr::from_slice(SOURCE)
                }),
                source_object: None,
                operation: AuditOperation::WRITE,
                source_comment: None,
                target_comment: None,
                invoke_id: Some(77 + step as u8),
                source_user_id: None,
                source_user_role: None,
                target_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 10)),
                target_object: Some(target),
                target_property: Some(AuditPropertyReference {
                    property_identifier: PropertyIdentifier::ALARM_VALUES,
                    property_array_index: None
                }),
                target_priority: None,
                target_value: Some(delta),
                current_value: Some(before.into_iter().flat_map(|value| [0x21, value]).collect()),
                result: None,
            }
        );
    }
    fixture.server.stop().await.unwrap();
    assert_eq!(
        notifications(&fixture.transport.sent).len(),
        3,
        "no recursive records"
    );
}

const SERVICES: [ConfirmedServiceChoice; 2] = [
    ConfirmedServiceChoice::ADD_LIST_ELEMENT,
    ConfirmedServiceChoice::REMOVE_LIST_ELEMENT,
];

fn error_fields(response: Apdu, service: ConfirmedServiceChoice) -> (ErrorClass, ErrorCode) {
    let Apdu::Error(error) = response else {
        panic!("{response:?}")
    };
    assert_eq!(error.service_choice, service);
    assert!(error.error_data.is_empty());
    (error.error_class, error.error_code)
}

#[tokio::test]
async fn audit_reporter_list_execution_failures_keep_response_state_and_known_fields() {
    let target = oid(ObjectType::MULTI_STATE_INPUT, 1);
    for service in SERVICES {
        for (object, property, index, delta, expected, current) in [
            (
                oid(ObjectType::MULTI_STATE_INPUT, 999),
                PropertyIdentifier::ALARM_VALUES,
                None,
                vec![0x21, 2],
                (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
                None,
            ),
            (
                target,
                PropertyIdentifier::ALARM_VALUES,
                Some(1),
                vec![0x21, 2],
                (ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
                Some(vec![0x21, 1]),
            ),
            (
                target,
                PropertyIdentifier::from_raw(9999),
                None,
                vec![0x21, 2],
                (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
                None,
            ),
            (
                target,
                PropertyIdentifier::OBJECT_IDENTIFIER,
                None,
                vec![0x21, 2],
                (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED),
                Some(vec![0xc4, 0x03, 0x40, 0, 1]),
            ),
            (
                oid(ObjectType::BINARY_VALUE, 1),
                PropertyIdentifier::PRESENT_VALUE,
                None,
                vec![0x21, 2],
                (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE),
                Some(vec![0x91, 0]),
            ),
        ] {
            let mut fixture = list_server(vec![1]).await;
            let before = values(&fixture).await;
            let response = dispatch(
                &fixture.server,
                service,
                list_request(object, property, index, delta.clone()),
            )
            .await;
            assert_eq!(error_fields(response, service), expected);
            assert_eq!(values(&fixture).await, before);
            settle().await;
            let records = notifications(&fixture.transport.sent);
            assert_eq!(records.len(), 1, "{service:?} {property:?}");
            let record = &records[0].notifications[0];
            assert_eq!(record.result, Some(expected));
            assert_eq!(record.operation, AuditOperation::WRITE);
            assert_eq!(record.target_object, Some(object));
            assert_eq!(
                record.target_property,
                Some(AuditPropertyReference {
                    property_identifier: property,
                    property_array_index: index.map(u64::from)
                })
            );
            assert_eq!(record.target_priority, None);
            assert_eq!(record.target_value, Some(delta));
            assert_eq!(record.current_value, current);
            fixture.server.stop().await.unwrap();
        }
    }
    let mut fixture = list_server((0..1024).collect()).await;
    let before = values(&fixture).await;
    let response = dispatch(
        &fixture.server,
        SERVICES[0],
        list_request(
            target,
            PropertyIdentifier::ALARM_VALUES,
            None,
            vec![0x21, 2],
        ),
    )
    .await;
    let expected = (
        ErrorClass::RESOURCES,
        ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT,
    );
    assert_eq!(error_fields(response, SERVICES[0]), expected);
    assert_eq!(values(&fixture).await, before);
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].notifications[0].result, Some(expected));
    assert_eq!(records[0].notifications[0].current_value, None);
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_list_malformed_elements_and_services_are_silent() {
    for service in SERVICES {
        let mut fixture = list_server(vec![1]).await;
        for target in [
            oid(ObjectType::MULTI_STATE_INPUT, 1),
            oid(ObjectType::MULTI_STATE_INPUT, 999),
        ] {
            for delta in [vec![0x21, 2, 0xd1, 0], vec![0x21, 2, 0x44, 0]] {
                let response = dispatch(
                    &fixture.server,
                    service,
                    list_request(target, PropertyIdentifier::ALARM_VALUES, None, delta),
                )
                .await;
                assert!(matches!(response, Apdu::Error(_)), "{response:?}");
                assert_eq!(
                    values(&fixture).await,
                    PropertyValue::List(vec![PropertyValue::Unsigned(1)])
                );
            }
        }
        for data in [Bytes::new(), {
            let mut data = list_request(
                oid(ObjectType::MULTI_STATE_INPUT, 1),
                PropertyIdentifier::ALARM_VALUES,
                None,
                vec![0x21, 2],
            )
            .to_vec();
            data.push(0);
            Bytes::from(data)
        }] {
            assert!(matches!(
                dispatch(&fixture.server, service, data).await,
                Apdu::Error(_)
            ));
        }
        settle().await;
        assert!(notifications(&fixture.transport.sent).is_empty());
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_list_filters_selection_and_self_target_do_not_change_execution() {
    use bacnet_types::constructed::BACnetObjectSelector as Selector;
    let target = oid(ObjectType::MULTI_STATE_INPUT, 1);
    for service in SERVICES {
        for (level, write_bit, selection, selected, self_target) in [
            (AuditLevel::NONE, true, None, false, false),
            (AuditLevel::AUDIT_ALL, false, None, false, false),
            (AuditLevel::AUDIT_CONFIG, true, None, true, false),
            (AuditLevel::AUDIT_ALL, true, Some(vec![]), false, false),
            (
                AuditLevel::AUDIT_ALL,
                true,
                Some(vec![Selector::None]),
                false,
                false,
            ),
            (
                AuditLevel::AUDIT_ALL,
                true,
                Some(vec![Selector::Object(target), Selector::Object(target)]),
                true,
                false,
            ),
            (
                AuditLevel::AUDIT_CONFIG,
                true,
                Some(vec![
                    Selector::None,
                    Selector::ObjectType(ObjectType::MULTI_STATE_INPUT),
                ]),
                true,
                false,
            ),
            (
                AuditLevel::AUDIT_ALL,
                true,
                Some(vec![Selector::Object(oid(
                    ObjectType::MULTI_STATE_INPUT,
                    2,
                ))]),
                false,
                false,
            ),
            (
                AuditLevel::AUDIT_ALL,
                true,
                Some(vec![Selector::ObjectType(ObjectType::ANALOG_INPUT)]),
                false,
                false,
            ),
            (AuditLevel::AUDIT_CONFIG, false, Some(vec![]), true, true),
            (AuditLevel::NONE, false, Some(vec![]), false, true),
        ] {
            let mut reporter = reporter();
            reporter.set_audit_level(level).unwrap();
            if !write_bit {
                reporter
                    .set_auditable_operations(AuditOperationFlags::empty())
                    .unwrap();
            }
            reporter
                .set_audit_priority_filter(BACnetPriorityFilter::empty())
                .unwrap();
            reporter.set_monitored_objects(selection).unwrap();
            let mut fixture = server(reporter).await;
            let mut object = MultiStateInputObject::new(1, "list", 3).unwrap();
            object.set_alarm_values(vec![1]);
            fixture
                .server
                .db
                .write()
                .await
                .add(Box::new(object))
                .unwrap();
            let object = if self_target {
                oid(ObjectType::AUDIT_REPORTER, 1)
            } else {
                target
            };
            let property = if self_target {
                PropertyIdentifier::MONITORED_OBJECTS
            } else {
                PropertyIdentifier::ALARM_VALUES
            };
            // Ordinary target: committed success, then indexed execution failure.
            // Reporter target: network read-only list failure bypasses selection/WRITE.
            for (step, index) in [None, Some(1)].into_iter().enumerate() {
                let response = dispatch(
                    &fixture.server,
                    service,
                    list_request(object, property, index, vec![0x21, 2]),
                )
                .await;
                let result = if step == 0 && !self_target {
                    assert!(matches!(response, Apdu::SimpleAck(_)), "{response:?}");
                    None
                } else {
                    Some(error_fields(response, service))
                };
                settle().await;
                let records = notifications(&fixture.transport.sent);
                assert_eq!(records.len(), if selected { step + 1 } else { 0 });
                if selected {
                    assert_eq!(records[step].notifications.len(), 1);
                    assert_eq!(records[step].notifications[0].result, result);
                    assert_eq!(records[step].notifications[0].target_priority, None);
                }
            }
            assert_eq!(
                values(&fixture).await,
                PropertyValue::List(if service == SERVICES[0] && !self_target {
                    vec![PropertyValue::Unsigned(1), PropertyValue::Unsigned(2)]
                } else {
                    vec![PropertyValue::Unsigned(1)]
                })
            );
            fixture.server.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn audit_reporter_list_value_bounds_and_empty_delta_are_omitted_not_wrapped() {
    for service in SERVICES {
        for (current_count, delta_count) in [
            (0, 0),
            (16, 16),
            (17, 17),
            (0, 16),
            (16, 0),
            (17, 16),
            (16, 17),
        ] {
            let mut fixture = list_server(vec![1; current_count]).await;
            let delta = [0x21, 2].repeat(delta_count);
            let response = dispatch(
                &fixture.server,
                service,
                list_request(
                    oid(ObjectType::MULTI_STATE_INPUT, 1),
                    PropertyIdentifier::ALARM_VALUES,
                    None,
                    delta.clone(),
                ),
            )
            .await;
            assert!(matches!(response, Apdu::SimpleAck(_)), "{response:?}");
            settle().await;
            let records = notifications(&fixture.transport.sent);
            assert_eq!(records.len(), 1);
            assert_eq!(
                records[0].notifications[0].target_value,
                (delta_count == 16).then_some(delta)
            );
            assert_eq!(
                records[0].notifications[0].current_value,
                (current_count == 16).then(|| [0x21, 1].repeat(current_count))
            );
            assert_eq!(records[0].notifications[0].result, None);
            fixture.server.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn audit_reporter_list_policy_and_authorizer_denials_are_silent() {
    for service in SERVICES {
        for policy_denial in [false, true] {
            let mut fixture = list_server(vec![1]).await;
            if policy_denial {
                fixture.server.config.mutation_policy = crate::mutation::MutationPolicy::DenyAll;
            } else {
                fixture.server.config.mutation_authorizer = Some(Arc::new(|_| false));
            }
            let response = dispatch(
                &fixture.server,
                service,
                list_request(
                    oid(ObjectType::MULTI_STATE_INPUT, 1),
                    PropertyIdentifier::ALARM_VALUES,
                    None,
                    vec![0x21, 2],
                ),
            )
            .await;
            assert_eq!(
                error_fields(response, service),
                (ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED)
            );
            assert_eq!(
                values(&fixture).await,
                PropertyValue::List(vec![PropertyValue::Unsigned(1)])
            );
            settle().await;
            assert!(notifications(&fixture.transport.sent).is_empty());
            fixture.server.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn audit_reporter_list_unknown_outcomes_are_silent_and_errors_match_response() {
    for service in SERVICES {
        for (error, report) in [
            (
                Error::Protocol {
                    class: 128,
                    code: 512,
                },
                true,
            ),
            (Error::OutOfRange("execution failure".into()), true),
            (Error::Timeout(Duration::from_secs(1)), false),
            (
                Error::Reject {
                    reason: RejectReason::OTHER.to_raw(),
                },
                false,
            ),
            (
                Error::Abort {
                    reason: AbortReason::OTHER.to_raw(),
                },
                false,
            ),
        ] {
            let mut fixture = list_server(vec![1]).await;
            *fixture.execution_error.lock().unwrap() = Some(error);
            let response = dispatch(
                &fixture.server,
                service,
                list_request(
                    oid(ObjectType::BINARY_VALUE, 1),
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                    vec![0x21, 2],
                ),
            )
            .await;
            settle().await;
            let records = notifications(&fixture.transport.sent);
            assert_eq!(records.len(), usize::from(report));
            if report {
                assert_eq!(
                    records[0].notifications[0].result,
                    Some(error_fields(response, service))
                );
            } else {
                assert!(matches!(response, Apdu::Error(_) | Apdu::Reject(_)));
            }
            assert_eq!(fixture.attempts.load(Ordering::Acquire), 1);
            assert_eq!(fixture.writes.load(Ordering::Acquire), 0);
            fixture.server.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn audit_reporter_list_framed_destinations_decode_before_observation() {
    use bacnet_objects::notification_class::NotificationClass;
    use bacnet_types::{constructed::BACnetDestination, primitives::Time};
    let target = oid(ObjectType::NOTIFICATION_CLASS, 1);
    let destination = BACnetDestination {
        valid_days: 0x7f,
        from_time: Time {
            hour: 0,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        to_time: Time {
            hour: 23,
            minute: 59,
            second: 0,
            hundredths: 0,
        },
        recipient: BACnetRecipient::Device(oid(ObjectType::DEVICE, 20)),
        process_identifier: 1,
        issue_confirmed_notifications: false,
        transitions: 7,
    };
    let mut delta = BytesMut::new();
    bacnet_encoding::constructed::encode_destination_list(
        &mut delta,
        std::slice::from_ref(&destination),
    );
    assert!(delta.len() <= 32);
    for service in SERVICES {
        let mut fixture = list_server(vec![1]).await;
        let mut object = NotificationClass::new(1, "destinations").unwrap();
        object.add_destination(destination.clone());
        fixture
            .server
            .db
            .write()
            .await
            .add(Box::new(object))
            .unwrap();
        // A valid frame followed by a valid TLV that is NOT a destination
        // distinguishes service decoding from the late framed decoder.
        for object in [target, oid(ObjectType::NOTIFICATION_CLASS, 999)] {
            let mut malformed = delta.to_vec();
            malformed.extend_from_slice(&[0x21, 2]);
            let request = list_request(object, PropertyIdentifier::RECIPIENT_LIST, None, malformed);
            assert!(ListElementRequest::decode(&request).is_ok());
            let response = dispatch(&fixture.server, service, request).await;
            let expected = if object == target {
                (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE)
            } else {
                (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT)
            };
            assert_eq!(error_fields(response, service), expected);
        }
        settle().await;
        assert!(notifications(&fixture.transport.sent).is_empty());
        assert_eq!(
            fixture
                .server
                .db
                .read()
                .await
                .get(&target)
                .unwrap()
                .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
                .unwrap(),
            PropertyValue::ApplicationData(delta.to_vec())
        );
        let response = dispatch(
            &fixture.server,
            service,
            list_request(
                target,
                PropertyIdentifier::RECIPIENT_LIST,
                None,
                delta.to_vec(),
            ),
        )
        .await;
        assert!(matches!(response, Apdu::SimpleAck(_)));
        settle().await;
        let records = notifications(&fixture.transport.sent);
        assert_eq!(records.len(), 1);
        let record = &records[0].notifications[0];
        assert_eq!(record.target_value, Some(delta.to_vec()));
        assert_eq!(record.current_value, Some(delta.to_vec()));
        assert_eq!(record.result, None);
        let after = if service == SERVICES[0] {
            [delta.as_ref(), delta.as_ref()].concat()
        } else {
            vec![]
        };
        assert_eq!(
            fixture
                .server
                .db
                .read()
                .await
                .get(&target)
                .unwrap()
                .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
                .unwrap(),
            PropertyValue::ApplicationData(after)
        );
        // Valid framed content with an unknown object is an execution failure.
        let response = dispatch(
            &fixture.server,
            service,
            list_request(
                oid(ObjectType::NOTIFICATION_CLASS, 999),
                PropertyIdentifier::RECIPIENT_LIST,
                None,
                delta.to_vec(),
            ),
        )
        .await;
        assert_eq!(
            error_fields(response, service),
            (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT)
        );
        settle().await;
        let records = notifications(&fixture.transport.sent);
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].notifications[0].current_value, None);
        assert_eq!(
            records[1].notifications[0].result,
            Some((ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT))
        );
        fixture.server.stop().await.unwrap();
    }
}
