//! Direct source writes use the read family's existing ownership and wire fixtures.
use super::*;
use bacnet_services::write_property::WritePropertyRequest;

fn write_database(confirmed: bool) -> ObjectDatabase {
    let mut db = database(confirmed);
    db.get_mut(&selected())
        .unwrap()
        .audit_reporter_mut()
        .expect("an Audit Reporter")
        .configure(
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
/// A WriteProperty of one NULL to `target()`, with no priority, in a task
/// of its own.
fn start_write(
    client: &Client,
    mac: &[u8],
    property: PropertyIdentifier,
    index: Option<u32>,
) -> tokio::task::JoinHandle<Result<(), Error>> {
    write_value(client, mac, property, index, vec![0], None)
}
/// A WriteProperty of `value` to `target()`, in a task of its own.
fn write_value(
    client: &Client,
    mac: &[u8],
    property: PropertyIdentifier,
    index: Option<u32>,
    value: Vec<u8>,
    priority: Option<u8>,
) -> tokio::task::JoinHandle<Result<(), Error>> {
    let client = Arc::clone(client);
    let mac = mac.to_vec();
    tokio::spawn(async move {
        client
            .write_property(&mac[..], target(), property, index, value, priority)
            .await
    })
}

/// A write's request as it goes, its record's fields, and the Result of
/// each way it can end. A value past 32 octets is left out of the record.
#[tokio::test(start_paused = true)]
async fn source_write_wire_identity_value_boundaries_and_terminal_results() {
    let rows = [
        (0, 0),
        (32, 0),
        (33, 0),
        (1, 1),
        (0, 1),
        (1, 2),
        (1, 3),
        (1, 4),
        (1, 6),
    ];
    write_terminals(&rows).await;
}

/// A ComplexACK doesn't answer a WriteProperty, whose only acknowledgment
/// is a SimpleACK (Clause 15.9): the device's answer of the wrong kind ends
/// the write at once, not as a success and not as a timeout, and its record
/// says COMMUNICATION / OTHER, since whether it took is unknown (#2721).
#[tokio::test(start_paused = true)]
async fn source_write_complex_ack_cannot_finish_a_write() {
    write_terminals(&[(1, 5)]).await;
}

/// One audited write per row, of that many NULL octets, ended the row's
/// way: 0 its SimpleACK, 1 an Error, 2 a Reject, 3 an Abort, 4 a SimpleACK
/// for another service, 5 a ComplexACK for its own, 6 no answer.
async fn write_terminals(rows: &[(usize, u8)]) {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    for confirmed in [false, true] {
        let mut session = session(write_database(confirmed), SessionRole::Both, &sink);
        session.start().await.unwrap();
        let client = client_of(&session);
        for &(n, terminal) in rows {
            // A sequence of NULL tags is valid framed ANY, independent of remote type.
            let value = vec![0; n];
            let task = write_value(
                &client,
                peer.local_mac(),
                PropertyIdentifier::DESCRIPTION,
                Some(2),
                value.clone(),
                Some(8),
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
            assert_eq!(
                result.is_ok(),
                terminal == 0,
                "{n} octets, terminal {terminal}"
            );
            if terminal == 5 {
                assert!(
                    matches!(
                        result,
                        Err(bacnet_types::error::Error::UnexpectedAck { complex: true, .. })
                    ),
                    "{result:?}"
                );
            }
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
            // Sent with a priority: a command at that priority.
            assert_eq!(record.target_priority, Some(8));
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
                5 => Some((ErrorClass::COMMUNICATION, ErrorCode::OTHER)),
                _ => Some((ErrorClass::COMMUNICATION, ErrorCode::TIMEOUT)),
            };
            assert_eq!(record.result, expected, "{n} octets, terminal {terminal}");
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

/// Which writes the Reporter selects. A write sent with a priority is a
/// command at that priority, which the Audit_Priority_Filter applies to;
/// one sent without is filtered by level and operation alone, whatever its
/// property or value.
#[tokio::test(start_paused = true)]
async fn source_write_priority_level_and_operation_policy_matrix() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(write_database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let client = client_of(&session);
    let (all, config) = (AuditLevel::AUDIT_ALL, AuditLevel::AUDIT_CONFIG);
    let (present_value, description) = (
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::DESCRIPTION,
    );
    for (priority, filter, property, level, operations, reports) in [
        // No priority: the filter doesn't apply, whichever bits it holds.
        (None, 0x8000, present_value, all, 2, true),
        (None, 0, present_value, all, 2, true),
        // A priority: reported when the filter holds it.
        (Some(8), 0x80, description, all, 2, true),
        (Some(8), 0, description, all, 2, false),
        (Some(8), 0, present_value, all, 2, false),
        // AUDIT_CONFIG leaves Present_Value out.
        (None, 0, description, config, 2, true),
        (None, 0, present_value, config, 2, false),
        // Level NONE, and WRITE not among the operations.
        (None, 0xffff, description, AuditLevel::NONE, 2, false),
        (None, 0xffff, description, all, 1, false),
    ] {
        session
            .database
            .as_ref()
            .unwrap()
            .write()
            .await
            .get_mut(&selected())
            .unwrap()
            .audit_reporter_mut()
            .expect("an Audit Reporter")
            .configure(
                level,
                AuditOperationFlags::from_bits(operations).unwrap(),
                false,
                None,
                BACnetPriorityFilter::from_bits(filter),
                None,
            )
            .unwrap();
        let row = format!("{priority:?} {filter:#x} {property:?} {level:?} {operations}");
        for value in [vec![0], vec![0x21, 42]] {
            let task = write_value(
                &client,
                peer.local_mac(),
                property,
                None,
                value.clone(),
                priority,
            );
            let envelope = receive(&mut requests).await;
            let (invoke, wp) = write_request(&envelope);
            assert_eq!(wp.priority, priority);
            assert_eq!(wp.property_value, value);
            send(&peer, &envelope.source_mac, write_ack(invoke, &wp)).await;
            assert!(task.await.unwrap().is_ok(), "{row}");
            if reports {
                let (record, _) = notification(&receive(&mut records).await, false);
                assert_eq!(record.target_priority, priority, "{row}");
                assert_eq!(record.target_value, Some(value));
            } else {
                assert!(
                    timeout(Duration::from_millis(15), records.recv())
                        .await
                        .is_err(),
                    "{row}"
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
