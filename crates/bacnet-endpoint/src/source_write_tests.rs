//! Direct source writes use the read family's existing ownership and wire fixtures.
use super::*;
use crate::roles::Commandability;
use bacnet_services::write_property::WritePropertyRequest;

fn write_database(confirmed: bool) -> ObjectDatabase {
    let mut db = database(confirmed);
    db.get_mut(&selected())
        .unwrap()
        .configure_audit_reporter_internal(
            AuditLevel::AUDIT_ALL,
            AuditOperationFlags::from_bits(2).unwrap(),
            confirmed,
            None,
            BACnetPriorityFilter::all(),
            None,
        )
        .unwrap();
    db
}
fn write_request(envelope: &ReceivedApdu) -> (u8, WritePropertyRequest) {
    let Apdu::ConfirmedRequest(request) = decode_apdu(envelope.apdu.clone()).unwrap() else {
        panic!("WP request")
    };
    assert_eq!(
        request.service_choice,
        ConfirmedServiceChoice::WRITE_PROPERTY
    );
    (
        request.invoke_id,
        WritePropertyRequest::decode(&request.service_request).unwrap(),
    )
}
fn write_ack(invoke_id: u8, _: &WritePropertyRequest) -> Apdu {
    Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
    })
}
fn start_write(
    session: &EndpointSession<BipTransport>,
    mac: &[u8],
    property: PropertyIdentifier,
    index: Option<u32>,
) -> tokio::task::JoinHandle<Result<(), Error>> {
    write_value(
        session,
        mac,
        property,
        index,
        vec![0],
        None,
        Commandability::Commandable,
    )
}
fn write_value(
    session: &EndpointSession<BipTransport>,
    mac: &[u8],
    property: PropertyIdentifier,
    index: Option<u32>,
    value: Vec<u8>,
    priority: Option<u8>,
    commandability: Commandability,
) -> tokio::task::JoinHandle<Result<(), Error>> {
    let client = session.cloned_client_handle().unwrap();
    let mac = mac.to_vec();
    tokio::spawn(async move {
        client
            .write_property(
                &mac,
                WritePropertyRequest {
                    object_identifier: target(),
                    property_identifier: property,
                    property_array_index: index,
                    property_value: value,
                    priority,
                },
                commandability,
            )
            .await
    })
}

#[tokio::test]
async fn source_write_wire_identity_value_boundaries_and_terminal_results() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    for confirmed in [false, true] {
        let mut session = session(write_database(confirmed), SessionRole::Both, &sink);
        session.start().await.unwrap();
        for (n, terminal) in [
            (0, 0),
            (32, 0),
            (33, 0),
            (1, 1),
            (0, 1),
            (1, 2),
            (1, 3),
            (1, 4),
            (1, 5),
            (1, 6),
        ] {
            // A sequence of NULL tags is valid framed ANY, independent of remote type.
            let value = vec![0; n];
            let task = write_value(
                &session,
                peer.local_mac(),
                PropertyIdentifier::DESCRIPTION,
                Some(2),
                value.clone(),
                Some(8),
                Commandability::Noncommandable,
            );
            let envelope = receive(&mut requests).await;
            let (invoke, wp) = write_request(&envelope);
            assert_eq!(wp.object_identifier, target());
            assert_eq!(wp.property_identifier, PropertyIdentifier::DESCRIPTION);
            assert_eq!(wp.property_array_index, Some(2));
            assert_eq!(wp.priority, Some(8));
            assert_eq!(wp.property_value, value);
            let pdu = match terminal {
                0 => Some(write_ack(invoke, &wp)),
                1 => Some(Apdu::Error(ErrorPdu {
                    invoke_id: invoke,
                    service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
                    error_class: ErrorClass::PROPERTY,
                    error_code: ErrorCode::WRITE_ACCESS_DENIED,
                    error_data: Bytes::new(),
                })),
                2 => Some(Apdu::Reject(RejectPdu {
                    invoke_id: invoke,
                    reject_reason: RejectReason::INVALID_TAG,
                })),
                3 => Some(Apdu::Abort(AbortPdu {
                    sent_by_server: true,
                    invoke_id: invoke,
                    abort_reason: AbortReason::BUFFER_OVERFLOW,
                })),
                4 => Some(Apdu::SimpleAck(SimpleAck {
                    invoke_id: invoke,
                    service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                })),
                5 => Some(Apdu::ComplexAck(ComplexAck {
                    segmented: false,
                    more_follows: false,
                    invoke_id: invoke,
                    sequence_number: None,
                    proposed_window_size: None,
                    service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
                    service_ack: Bytes::new(),
                })),
                _ => None,
            };
            if let Some(pdu) = pdu {
                send(&peer, &envelope.source_mac, pdu).await;
            }
            let result = task.await.unwrap();
            assert_eq!(result.is_ok(), terminal == 0);
            let envelope = receive(&mut records).await;
            let (record, notification_id) = notification(&envelope, confirmed);
            assert_eq!(record.operation, AuditOperation::WRITE);
            assert_eq!(record.invoke_id, Some(invoke));
            assert_eq!(
                record.source_device,
                BACnetRecipient::Device(oid(ObjectType::DEVICE, 123))
            );
            assert_eq!(record.target_object, Some(target()));
            let property = record.target_property.unwrap();
            assert_eq!(property.property_identifier, wp.property_identifier);
            assert_eq!(property.property_array_index, Some(2));
            assert_eq!(record.target_value, (n <= 32).then_some(value));
            assert_eq!(record.target_priority, None);
            assert!(record.current_value.is_none());
            assert!(record.target_timestamp.is_none());
            assert!(
                matches!(record.target_device, BACnetRecipient::Address(address) if address.mac_address.as_slice() == peer.local_mac())
            );
            let expected = match terminal {
                0 => None,
                1 => Some((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)),
                2 => Some((ErrorClass::COMMUNICATION, ErrorCode::REJECT_INVALID_TAG)),
                3 => Some((ErrorClass::COMMUNICATION, ErrorCode::ABORT_BUFFER_OVERFLOW)),
                _ => Some((ErrorClass::COMMUNICATION, ErrorCode::TIMEOUT)),
            };
            assert_eq!(record.result, expected);
            if let Some(invoke_id) = notification_id {
                send(
                    &sink,
                    &envelope.source_mac,
                    Apdu::SimpleAck(SimpleAck {
                        invoke_id,
                        service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
                    }),
                )
                .await;
            }
            assert!(records.try_recv().is_err());
        }
        session.stop().await.unwrap();
    }
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_write_commandability_priority_policy_matrix() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    let mut session = session(write_database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    // No inference from property identity, NULL, or a supplied wire priority.
    for (commandability, priority, filter, property, level, operations, reports) in [
        (
            Commandability::Commandable,
            None,
            0x8000,
            PropertyIdentifier::PRESENT_VALUE,
            AuditLevel::AUDIT_ALL,
            2,
            true,
        ),
        (
            Commandability::Commandable,
            None,
            1,
            PropertyIdentifier::PRESENT_VALUE,
            AuditLevel::AUDIT_ALL,
            2,
            false,
        ),
        (
            Commandability::Commandable,
            Some(8),
            0x80,
            PropertyIdentifier::DESCRIPTION,
            AuditLevel::AUDIT_ALL,
            2,
            true,
        ),
        (
            Commandability::Commandable,
            Some(8),
            0,
            PropertyIdentifier::DESCRIPTION,
            AuditLevel::AUDIT_ALL,
            2,
            false,
        ),
        (
            Commandability::Noncommandable,
            Some(8),
            0,
            PropertyIdentifier::PRESENT_VALUE,
            AuditLevel::AUDIT_ALL,
            2,
            true,
        ),
        (
            Commandability::Noncommandable,
            None,
            0,
            PropertyIdentifier::DESCRIPTION,
            AuditLevel::AUDIT_CONFIG,
            2,
            true,
        ),
        (
            Commandability::Noncommandable,
            None,
            0,
            PropertyIdentifier::PRESENT_VALUE,
            AuditLevel::AUDIT_CONFIG,
            2,
            false,
        ),
        (
            Commandability::Commandable,
            None,
            0xffff,
            PropertyIdentifier::DESCRIPTION,
            AuditLevel::NONE,
            2,
            false,
        ),
        (
            Commandability::Commandable,
            None,
            0xffff,
            PropertyIdentifier::DESCRIPTION,
            AuditLevel::AUDIT_ALL,
            1,
            false,
        ),
    ] {
        session
            .database
            .as_ref()
            .unwrap()
            .write()
            .await
            .get_mut(&selected())
            .unwrap()
            .configure_audit_reporter_internal(
                level,
                AuditOperationFlags::from_bits(operations).unwrap(),
                false,
                None,
                BACnetPriorityFilter::from_bits(filter),
                None,
            )
            .unwrap();
        for value in [vec![0], vec![0x21, 42]] {
            let task = write_value(
                &session,
                peer.local_mac(),
                property,
                None,
                value.clone(),
                priority,
                commandability,
            );
            let envelope = receive(&mut requests).await;
            let (invoke, wp) = write_request(&envelope);
            assert_eq!(wp.priority, priority);
            assert_eq!(wp.property_value, value);
            send(&peer, &envelope.source_mac, write_ack(invoke, &wp)).await;
            assert!(task.await.unwrap().is_ok());
            if reports {
                let (record, _) = notification(&receive(&mut records).await, false);
                assert_eq!(
                    record.target_priority,
                    (commandability == Commandability::Commandable)
                        .then_some(priority.unwrap_or(16))
                );
                assert_eq!(record.target_value, Some(value));
            } else {
                assert!(
                    timeout(Duration::from_millis(15), records.recv())
                        .await
                        .is_err()
                );
            }
        }
    }
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[path = "source_write_lifecycle_tests.rs"]
mod lifecycle;
#[path = "source_write_preflight_tests.rs"]
mod preflight;

#[path = "source_write_queue_tests.rs"]
mod queue;
