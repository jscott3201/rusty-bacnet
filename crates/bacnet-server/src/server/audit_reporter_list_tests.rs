use super::*;
use bacnet_objects::multistate::MultiStateInputObject;
use bacnet_services::list_manipulation::ListElementRequest;

#[path = "audit_reporter_list_destination_tests.rs"]
mod destinations;

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
    // A list edit runs the object's event evaluation (#1305). Present_Value 3
    // stays out of the alarm values these tests edit, so no transition takes
    // a sequence number from the source the audit time stamps share.
    object.set_present_value(3);
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
async fn audit_reporter_list_duplicates_and_absent_elements_preserve_delta_and_preimage() {
    let mut fixture = list_server(vec![1]).await;
    let target = oid(ObjectType::MULTI_STATE_INPUT, 1);
    // The list gains one 2 from a doubled 2, and removing an absent 3 is an
    // execution failure the log records with the unchanged pre-image (#1027).
    let not_found = (ErrorClass::SERVICES, ErrorCode::LIST_ELEMENT_NOT_FOUND);
    for (step, (service, delta, before, after, result)) in [
        (
            ConfirmedServiceChoice::ADD_LIST_ELEMENT,
            vec![0x21, 2, 0x21, 2],
            vec![1],
            vec![1, 2],
            None,
        ),
        (
            ConfirmedServiceChoice::REMOVE_LIST_ELEMENT,
            vec![0x21, 2],
            vec![1, 2],
            vec![1],
            None,
        ),
        (
            ConfirmedServiceChoice::REMOVE_LIST_ELEMENT,
            vec![0x21, 3],
            vec![1],
            vec![1],
            Some(not_found),
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
        match result {
            None => assert!(matches!(response, Apdu::SimpleAck(_)), "{response:?}"),
            Some((class, code)) => {
                assert_eq!(change_list_error(response, service), (class, code, 1))
            }
        }
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
                result,
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

/// The ChangeList-Error both list services answer with (#1026): the class,
/// code and First Failed Element Number.
fn change_list_error(
    response: Apdu,
    service: ConfirmedServiceChoice,
) -> (ErrorClass, ErrorCode, u32) {
    let Apdu::Error(error) = response else {
        panic!("{response:?}")
    };
    assert_eq!(error.service_choice, service);
    let body = bacnet_services::list_manipulation::ChangeListError::try_from(&error).unwrap();
    (
        body.error_class,
        body.error_code,
        body.first_failed_element_number,
    )
}

/// A ChangeList-Error that names no element: a refusal of the request or its
/// target.
fn error_fields(response: Apdu, service: ConfirmedServiceChoice) -> (ErrorClass, ErrorCode) {
    let (class, code, element) = change_list_error(response, service);
    assert_eq!(element, 0, "{class:?}/{code:?}");
    (class, code)
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
                (ErrorClass::SERVICES, ErrorCode::PROPERTY_IS_NOT_A_LIST),
                Some(vec![0xc4, 0x03, 0x40, 0, 1]),
            ),
            (
                oid(ObjectType::BINARY_VALUE, 1),
                PropertyIdentifier::PRESENT_VALUE,
                None,
                vec![0x21, 2],
                // A scalar target is refused before any write is attempted.
                (ErrorClass::SERVICES, ErrorCode::PROPERTY_IS_NOT_A_LIST),
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
    // Unsigned 2000 is new, so it is the element that does not fit.
    let response = dispatch(
        &fixture.server,
        SERVICES[0],
        list_request(
            target,
            PropertyIdentifier::ALARM_VALUES,
            None,
            vec![0x22, 0x07, 0xD0],
        ),
    )
    .await;
    let expected = (
        ErrorClass::RESOURCES,
        ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT,
    );
    assert_eq!(
        change_list_error(response, SERVICES[0]),
        (expected.0, expected.1, 1)
    );
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
                // A REAL whose contents run past the list frame is a syntax
                // fault of the request, rejected (#1446).
                assert!(
                    matches!(response, Apdu::Error(_) | Apdu::Reject(_)),
                    "{response:?}"
                );
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
                Apdu::Reject(_)
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
            // RemoveListElement names the present 1, AddListElement a new 2.
            let element = if service == SERVICES[0] { 2 } else { 1 };
            for (step, index) in [None, Some(1)].into_iter().enumerate() {
                let response = dispatch(
                    &fixture.server,
                    service,
                    list_request(object, property, index, vec![0x21, element]),
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
                PropertyValue::List(match (service == SERVICES[0], self_target) {
                    (_, true) => vec![PropertyValue::Unsigned(1)],
                    (true, false) => vec![PropertyValue::Unsigned(1), PropertyValue::Unsigned(2)],
                    (false, false) => vec![],
                })
            );
            fixture.server.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn audit_reporter_list_value_bounds_preserve_empty_and_omit_large() {
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
            // AddListElement adds a 2 (once); RemoveListElement removes the 1s,
            // which an empty list does not have (#1027).
            let (element, result) = if service == SERVICES[0] {
                (2, None)
            } else {
                (
                    1,
                    (current_count == 0 && delta_count > 0)
                        .then_some((ErrorClass::SERVICES, ErrorCode::LIST_ELEMENT_NOT_FOUND)),
                )
            };
            let delta = [0x21, element].repeat(delta_count);
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
            match result {
                None => assert!(matches!(response, Apdu::SimpleAck(_)), "{response:?}"),
                Some((class, code)) => {
                    assert_eq!(change_list_error(response, service), (class, code, 1))
                }
            }
            settle().await;
            let records = notifications(&fixture.transport.sent);
            assert_eq!(records.len(), 1);
            assert_eq!(
                records[0].notifications[0].target_value,
                (delta_count <= 16).then_some(delta)
            );
            assert_eq!(
                records[0].notifications[0].current_value,
                (current_count <= 16).then(|| [0x21, 1].repeat(current_count))
            );
            assert_eq!(records[0].notifications[0].result, result);
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
                fixture.server.config_mut().mutation_policy =
                    crate::mutation::MutationPolicy::DenyAll;
            } else {
                fixture.server.config_mut().mutation_authorizer = Some(Arc::new(|_| false));
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
            // The list itself must reach its write for the scripted error to fire.
            let mut fixture = server(reporter()).await;
            let mut list = MultiStateInputObject::new(1, "list", 3).unwrap();
            list.set_alarm_values(vec![1]);
            let list = fixture.counting(list);
            fixture.server.db.write().await.add(list).unwrap();
            *fixture.execution_error.lock().unwrap() = Some(error);
            // A present element for RemoveListElement, so both reach the write.
            let element = if service == SERVICES[0] { 2 } else { 1 };
            let response = dispatch(
                &fixture.server,
                service,
                list_request(
                    oid(ObjectType::MULTI_STATE_INPUT, 1),
                    PropertyIdentifier::ALARM_VALUES,
                    None,
                    vec![0x21, element],
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
