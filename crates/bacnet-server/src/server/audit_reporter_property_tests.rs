//! Wire-visible optional property, metadata/PICS, and network-write denial.

use super::*;
use bacnet_services::{
    common::PropertyReference,
    read_property::{ReadPropertyACK, ReadPropertyRequest},
    rpm::{ReadAccessSpecification, ReadPropertyMultipleACK, ReadPropertyMultipleRequest},
};
use bacnet_types::constructed::BACnetObjectSelector as Selector;

async fn read_wire(
    server: &BACnetServer<CaptureTransport>,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> Result<Vec<u8>, (ErrorClass, ErrorCode)> {
    let mut bytes = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::AUDIT_REPORTER, 1),
        property_identifier: property,
        property_array_index: index,
    }
    .encode(&mut bytes);
    match dispatch(
        server,
        ConfirmedServiceChoice::READ_PROPERTY,
        bytes.freeze(),
    )
    .await
    {
        Apdu::ComplexAck(ack) => {
            let ack = ReadPropertyACK::decode(&ack.service_ack).unwrap();
            assert_eq!(ack.property_identifier, property);
            assert_eq!(ack.property_array_index, index);
            Ok(ack.property_value)
        }
        Apdu::Error(error) => Err((error.error_class, error.error_code)),
        other => panic!("unexpected read response {other:?}"),
    }
}

async fn rpm_wire(
    server: &BACnetServer<CaptureTransport>,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> Vec<bacnet_services::rpm::ReadResultElement> {
    let mut bytes = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid(ObjectType::AUDIT_REPORTER, 1),
            list_of_property_references: vec![PropertyReference {
                property_identifier: property,
                property_array_index: index,
            }],
        }],
    }
    .encode(&mut bytes)
    .unwrap();
    let response = dispatch(
        server,
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        bytes.freeze(),
    )
    .await;
    let Apdu::ComplexAck(ack) = response else {
        panic!("unexpected RPM response {response:?}")
    };
    let ack = ReadPropertyMultipleACK::decode(&ack.service_ack).unwrap();
    assert_eq!(ack.list_of_read_access_results.len(), 1);
    ack.list_of_read_access_results[0].list_of_results.clone()
}

#[tokio::test]
async fn audit_reporter_monitored_objects_rp_rpm_property_list_and_pics_agree() {
    use PropertyIdentifier as P;
    // Clause 21 application NULL / ObjectIdentifier (type 5, instance 1) /
    // Enumerated (type 0), with a duplicate retained in array order.
    let entries = [
        vec![0x00],
        vec![0xc4, 0x01, 0x40, 0, 1],
        vec![0x91, 0],
        vec![0xc4, 0x01, 0x40, 0, 1],
    ];
    let mixed = vec![
        Selector::None,
        Selector::Object(oid(ObjectType::BINARY_VALUE, 1)),
        Selector::ObjectType(ObjectType::ANALOG_INPUT),
        Selector::Object(oid(ObjectType::BINARY_VALUE, 1)),
    ];
    for (selection, wire_entries) in [
        (None, None),
        (Some(vec![]), Some(vec![])),
        (Some(vec![Selector::None]), Some(vec![vec![0x00]])),
        (Some(mixed), Some(entries.to_vec())),
    ] {
        let configured = selection.is_some();
        let mut reporter = reporter();
        reporter.set_monitored_objects(selection);
        let mut fixture = server(reporter).await;
        for index in [
            None,
            Some(0),
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            Some(5),
            Some(u32::MAX),
        ] {
            let expected = match (&wire_entries, index) {
                (None, _) => Err((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)),
                (Some(values), None) => Ok(values.concat()),
                (Some(values), Some(0)) => Ok(vec![0x21, values.len() as u8]),
                (Some(values), Some(index)) => values
                    .get((index - 1) as usize)
                    .cloned()
                    .ok_or((ErrorClass::PROPERTY, ErrorCode::INVALID_ARRAY_INDEX)),
            };
            assert_eq!(
                read_wire(&fixture.server, P::MONITORED_OBJECTS, index).await,
                expected
            );
            let results = rpm_wire(&fixture.server, P::MONITORED_OBJECTS, index).await;
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].property_identifier, P::MONITORED_OBJECTS);
            assert_eq!(results[0].property_array_index, index);
            assert_eq!(results[0].property_value, expected.clone().ok());
            assert_eq!(results[0].error, expected.err());
        }
        let required = vec![
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::STATUS_FLAGS,
            P::RELIABILITY,
            P::EVENT_STATE,
            P::AUDIT_LEVEL,
            P::AUDIT_SOURCE_REPORTER,
            P::AUDITABLE_OPERATIONS,
            P::AUDIT_PRIORITY_FILTER,
            P::ISSUE_CONFIRMED_NOTIFICATIONS,
        ];
        let mut all = required.clone();
        all.insert(3, P::DESCRIPTION);
        let mut optional = vec![P::DESCRIPTION];
        if configured {
            all.push(P::MONITORED_OBJECTS);
            optional.push(P::MONITORED_OBJECTS);
        }
        for (selector, expected) in [
            (P::ALL, &all),
            (P::REQUIRED, &required),
            (P::OPTIONAL, &optional),
        ] {
            let results = rpm_wire(&fixture.server, selector, None).await;
            assert_eq!(
                results
                    .iter()
                    .map(|row| row.property_identifier)
                    .collect::<Vec<_>>(),
                *expected
            );
            for row in results {
                assert_eq!(row.error, None);
                assert_eq!(
                    row.property_value,
                    Some(
                        read_wire(&fixture.server, row.property_identifier, None)
                            .await
                            .unwrap()
                    )
                );
            }
        }
        let property_list = &all[3..];
        let wire_list = read_wire(&fixture.server, P::PROPERTY_LIST, None)
            .await
            .unwrap();
        let mut offset = 0;
        for (index, property) in property_list.iter().enumerate() {
            let (value, end) =
                bacnet_encoding::primitives::decode_application_value(&wire_list, offset).unwrap();
            assert_eq!(value, PropertyValue::Enumerated(property.to_raw()));
            assert_eq!(
                read_wire(&fixture.server, P::PROPERTY_LIST, Some(index as u32 + 1))
                    .await
                    .unwrap(),
                wire_list[offset..end]
            );
            offset = end;
        }
        assert_eq!(offset, wire_list.len());
        assert_eq!(
            read_wire(&fixture.server, P::PROPERTY_LIST, Some(0))
                .await
                .unwrap(),
            vec![0x21, property_list.len() as u8]
        );
        assert_eq!(
            read_wire(
                &fixture.server,
                P::PROPERTY_LIST,
                Some(property_list.len() as u32 + 1)
            )
            .await,
            Err((ErrorClass::PROPERTY, ErrorCode::INVALID_ARRAY_INDEX))
        );
        {
            let db = fixture.server.db.read().await;
            let pics = crate::pics::generate_pics(
                &db,
                &fixture.server.config,
                &crate::pics::PicsConfig::default(),
            );
            let support = pics
                .supported_object_types
                .iter()
                .find(|row| row.object_type == ObjectType::AUDIT_REPORTER)
                .unwrap();
            let monitored = support
                .supported_properties
                .iter()
                .find(|row| row.property_id == P::MONITORED_OBJECTS);
            assert_eq!(monitored.is_some(), configured);
            if let Some(row) = monitored {
                assert!(row.access.optional && row.access.readable && !row.access.writable);
            }
        }
        assert!(notifications(&fixture.transport.sent).is_empty());
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_monitored_objects_network_writes_are_denied_without_mutation_or_recursion()
{
    for selection in [None, Some(vec![]), Some(vec![Selector::None])] {
        for multiple in [false, true] {
            for index in [None, Some(0), Some(1)] {
                let mut reporter = reporter();
                reporter.set_monitored_objects(selection.clone());
                let mut fixture = server(reporter).await;
                let target = oid(ObjectType::AUDIT_REPORTER, 1);
                let before = fixture
                    .server
                    .db
                    .read()
                    .await
                    .get(&target)
                    .unwrap()
                    .read_property(PropertyIdentifier::MONITORED_OBJECTS, None)
                    .ok();
                fixture.transport.fail.store(true, Ordering::Release);
                let value = if index == Some(0) {
                    vec![0x21, 0]
                } else {
                    vec![0x00]
                };
                let mut bytes = BytesMut::new();
                let service = if multiple {
                    WritePropertyMultipleRequest {
                        list_of_write_access_specs: vec![WriteAccessSpecification {
                            object_identifier: target,
                            list_of_properties: vec![
                                BACnetPropertyValue {
                                    property_identifier: PropertyIdentifier::MONITORED_OBJECTS,
                                    property_array_index: index,
                                    value: value.clone(),
                                    priority: None,
                                },
                                element(PropertyIdentifier::DESCRIPTION, vec![0x72, 0, b'x']),
                            ],
                        }],
                    }
                    .encode(&mut bytes);
                    ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE
                } else {
                    WritePropertyRequest {
                        object_identifier: target,
                        property_identifier: PropertyIdentifier::MONITORED_OBJECTS,
                        property_array_index: index,
                        property_value: value.clone(),
                        priority: None,
                    }
                    .encode(&mut bytes);
                    ConfirmedServiceChoice::WRITE_PROPERTY
                };
                let response = dispatch(&fixture.server, service, bytes.freeze()).await;
                let Apdu::Error(error) = response else {
                    panic!("expected denial")
                };
                assert_eq!(
                    (error.error_class, error.error_code),
                    (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
                );
                if multiple {
                    let error =
                        bacnet_services::wpm::WritePropertyMultipleError::from_error_pdu(&error)
                            .unwrap();
                    assert_eq!(error.first_failed_write_attempt.object_identifier, target);
                    assert_eq!(
                        error.first_failed_write_attempt.property_identifier,
                        PropertyIdentifier::MONITORED_OBJECTS.to_raw()
                    );
                    assert_eq!(error.first_failed_write_attempt.property_array_index, index);
                }
                settle().await;
                {
                    let db = fixture.server.db.read().await;
                    let object = db.get(&target).unwrap();
                    assert_eq!(
                        object
                            .read_property(PropertyIdentifier::MONITORED_OBJECTS, None)
                            .ok(),
                        before
                    );
                    assert_eq!(
                        object
                            .property_list()
                            .contains(&PropertyIdentifier::MONITORED_OBJECTS),
                        selection.is_some()
                    );
                    assert_eq!(
                        object
                            .read_property(PropertyIdentifier::DESCRIPTION, None)
                            .unwrap(),
                        PropertyValue::CharacterString(String::new())
                    );
                    assert!(!object.is_writable_property(PropertyIdentifier::MONITORED_OBJECTS));
                }
                // As for existing Reporter configuration, the denied external
                // execution is reported once. Delivery failure cannot recurse.
                let requests = notifications(&fixture.transport.sent);
                assert_eq!(requests.len(), 1);
                let mut expected = failures::expected_value_write(
                    1,
                    0,
                    0,
                    Some((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)),
                );
                expected.target_object = Some(target);
                expected.target_property = Some(AuditPropertyReference {
                    property_identifier: PropertyIdentifier::MONITORED_OBJECTS,
                    property_array_index: index.map(u64::from),
                });
                expected.target_priority = None;
                expected.target_value = Some(value);
                expected.current_value = match (&selection, index) {
                    (Some(values), Some(0)) => Some(vec![0x21, values.len() as u8]),
                    (Some(values), None | Some(1)) if !values.is_empty() => Some(vec![0x00]),
                    _ => None,
                };
                assert_eq!(requests[0].notifications, vec![expected]);
                assert_eq!(
                    health(&fixture.server).await,
                    Reliability::COMMUNICATION_FAILURE
                );
                assert_eq!(fixture.server.notification_transactions.active_count(), 0);
                assert!(fixture.server.notification_transactions.workers_empty());
                fixture.server.stop().await.unwrap();
            }
        }
    }
}
