//! The services the full client's source Audit covers (#2617): each READ
//! and WRITE service of Table 19-5 this release audits, the records each
//! makes, and the writes' priorities.
use super::*;
use crate::roles::Commandability;
use bacnet_services::audit::{AuditLogQueryAck, AuditLogQueryRequest};
use bacnet_services::file::{
    AtomicReadFileAck, FileAccessMethod, FileReadAckMethod, FileWriteAccessMethod,
};
use bacnet_services::read_range::ReadRangeRequest;
use bacnet_services::read_range::{RangeSpec, ReadRangeAck, ReadRangeValidation};
use bacnet_services::rpm::{ReadAccessResult, ReadPropertyMultipleACK, ReadResultElement};
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleError};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::{
    AuditPropertyReference, BACnetAuditLogQueryParameters, BACnetObjectPropertyReference,
    BACnetPropertyValue, PropertyReference, ReadAccessSpecification,
};
use bacnet_types::enums::BACnetSuccessFilter;

/// A source Reporter that records READ and WRITE, with `priorities` as its
/// Audit_Priority_Filter.
fn database_for(priorities: BACnetPriorityFilter) -> ObjectDatabase {
    let mut db = database(false);
    db.get_mut(&selected())
        .unwrap()
        .audit_reporter_mut()
        .expect("an Audit Reporter")
        .configure(
            AuditLevel::AUDIT_ALL,
            AuditOperationFlags::from_bits(0b11).unwrap(),
            false,
            None,
            priorities,
            None,
        )
        .unwrap();
    db
}

/// The next confirmed request the peer receives: the MAC it came from,
/// its invoke ID and its service.
async fn next_request(
    requests: &mut bacnet_network::layer::ApduReceiver,
) -> (MacAddr, u8, ConfirmedServiceChoice) {
    let envelope = receive(requests).await;
    let Apdu::ConfirmedRequest(pdu) = decode_apdu(envelope.apdu.clone()).unwrap() else {
        panic!("a confirmed request")
    };
    (envelope.source_mac, pdu.invoke_id, pdu.service_choice)
}

fn complex(invoke_id: u8, service_choice: ConfirmedServiceChoice, ack: BytesMut) -> Apdu {
    Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice,
        service_ack: ack.freeze(),
    })
}

fn simple(invoke_id: u8, service_choice: ConfirmedServiceChoice) -> Apdu {
    Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice,
    })
}

/// A call on the full client in a task of its own.
fn spawn_call<F, R>(client: &Client, call: impl FnOnce(Client) -> F) -> tokio::task::JoinHandle<R>
where
    F: std::future::Future<Output = R> + Send + 'static,
    R: Send + 'static,
{
    tokio::spawn(call(Arc::clone(client)))
}

/// The next `n` unconfirmed records.
async fn next_records(
    records: &mut bacnet_network::layer::ApduReceiver,
    n: usize,
) -> Vec<BACnetAuditNotification> {
    let mut out = Vec::new();
    for _ in 0..n {
        out.push(notification(&receive(records).await, false).0);
    }
    out
}

fn reference(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u64>,
) -> (Option<ObjectIdentifier>, Option<AuditPropertyReference>) {
    (
        Some(object),
        Some(AuditPropertyReference {
            property_identifier: property,
            property_array_index: index,
        }),
    )
}

fn what(
    record: &BACnetAuditNotification,
) -> (Option<ObjectIdentifier>, Option<AuditPropertyReference>) {
    (record.target_object, record.target_property.clone())
}

fn rpm_specs() -> Vec<ReadAccessSpecification> {
    let property = |property_identifier, property_array_index| PropertyReference {
        property_identifier,
        property_array_index,
    };
    vec![
        ReadAccessSpecification {
            object_identifier: oid(ObjectType::DEVICE, 10),
            list_of_property_references: vec![
                property(PropertyIdentifier::OBJECT_NAME, None),
                property(PropertyIdentifier::OBJECT_LIST, Some(2)),
            ],
        },
        ReadAccessSpecification {
            object_identifier: target(),
            list_of_property_references: vec![property(PropertyIdentifier::PRESENT_VALUE, None)],
        },
    ]
}

/// Values for every reference but the array element, which fails.
fn rpm_answer(specs: &[ReadAccessSpecification]) -> ReadPropertyMultipleACK {
    ReadPropertyMultipleACK {
        list_of_read_access_results: specs
            .iter()
            .map(|spec| ReadAccessResult {
                object_identifier: spec.object_identifier,
                list_of_results: spec
                    .list_of_property_references
                    .iter()
                    .map(|p| {
                        let fails = p.property_array_index.is_some();
                        ReadResultElement {
                            property_identifier: p.property_identifier,
                            property_array_index: p.property_array_index,
                            property_value: (!fails).then(|| vec![0x21, 42]),
                            error: fails
                                .then_some((ErrorClass::PROPERTY, ErrorCode::INVALID_ARRAY_INDEX)),
                        }
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// One ReadPropertyMultiple, a record per reference, matching the narrow
/// requester's: each result's Result, the Device the answer names.
#[tokio::test(start_paused = true)]
async fn a_full_client_rpm_records_each_reference_as_the_narrow_requester_does() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let client = full_client(&session);
    let mut both = Vec::new();
    for narrow in [true, false] {
        let mac = peer.local_mac().to_vec();
        let read = if narrow {
            let handle = session.cloned_client_handle().unwrap();
            tokio::spawn(async move { handle.read_property_multiple(&mac, rpm_specs()).await })
        } else {
            spawn_call(&client, move |client| async move {
                client.read_property_multiple(&mac[..], rpm_specs()).await
            })
        };
        let (from, invoke, service) = next_request(&mut requests).await;
        assert_eq!(service, ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE);
        let mut ack = BytesMut::new();
        rpm_answer(&rpm_specs()).encode(&mut ack).unwrap();
        send(&peer, &from, complex(invoke, service, ack)).await;
        assert!(read.await.unwrap().is_ok());
        let found = next_records(&mut records, 3).await;
        assert!(found.iter().all(|r| r.invoke_id == Some(invoke)));
        both.push(found);
    }
    let full = both.pop().unwrap();
    let narrow = both.pop().unwrap();
    assert_eq!(
        full.iter().cloned().map(fields).collect::<Vec<_>>(),
        narrow.iter().cloned().map(fields).collect::<Vec<_>>()
    );
    assert_eq!(
        full.iter().map(|r| r.result).collect::<Vec<_>>(),
        [
            None,
            Some((ErrorClass::PROPERTY, ErrorCode::INVALID_ARRAY_INDEX)),
            None
        ]
    );
    // The successful results name Device 10 alone.
    assert_eq!(
        full[0].target_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 10))
    );
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// The narrow requester refuses ALL and the wildcard Device; the full
/// client sends them, and its records follow the answer: one per value,
/// on the Device the answer names.
#[tokio::test(start_paused = true)]
async fn a_full_client_rpm_of_all_records_each_value_the_answer_reports() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let client = full_client(&session);
    let wildcard = oid(ObjectType::DEVICE, ObjectIdentifier::WILDCARD_INSTANCE);
    let device = oid(ObjectType::DEVICE, 10);
    let mac = peer.local_mac().to_vec();
    let read = spawn_call(&client, move |client| async move {
        let all = vec![ReadAccessSpecification {
            object_identifier: wildcard,
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::ALL,
                property_array_index: None,
            }],
        }];
        client.read_property_multiple(&mac[..], all).await
    });
    let (from, invoke, service) = next_request(&mut requests).await;
    let element = |property_identifier, error: Option<(ErrorClass, ErrorCode)>| ReadResultElement {
        property_identifier,
        property_array_index: None,
        property_value: error.is_none().then(|| vec![0x21, 1]),
        error,
    };
    let answer = ReadPropertyMultipleACK {
        list_of_read_access_results: vec![ReadAccessResult {
            object_identifier: device,
            list_of_results: vec![
                element(PropertyIdentifier::OBJECT_NAME, None),
                element(PropertyIdentifier::PRESENT_VALUE, None),
                element(
                    PropertyIdentifier::DESCRIPTION,
                    Some((ErrorClass::PROPERTY, ErrorCode::READ_ACCESS_DENIED)),
                ),
            ],
        }],
    };
    let mut ack = BytesMut::new();
    answer.encode(&mut ack).unwrap();
    send(&peer, &from, complex(invoke, service, ack)).await;
    assert!(read.await.unwrap().is_ok());
    let found = next_records(&mut records, 3).await;
    assert_eq!(
        found.iter().map(what).collect::<Vec<_>>(),
        [
            reference(device, PropertyIdentifier::OBJECT_NAME, None),
            reference(device, PropertyIdentifier::PRESENT_VALUE, None),
            reference(device, PropertyIdentifier::DESCRIPTION, None),
        ]
    );
    assert_eq!(
        found.iter().map(|r| r.result).collect::<Vec<_>>(),
        [
            None,
            None,
            Some((ErrorClass::PROPERTY, ErrorCode::READ_ACCESS_DENIED))
        ]
    );
    assert!(
        found
            .iter()
            .all(|r| r.target_device == BACnetRecipient::Device(device))
    );
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

fn range_answer(request: &ReadRangeRequest, object: ObjectIdentifier) -> ReadRangeAck {
    ReadRangeAck {
        object_identifier: object,
        property_identifier: request.property_identifier,
        property_array_index: request.property_array_index,
        result_flags: (true, true, false),
        item_count: 1,
        item_data: vec![0x21, 1],
        // A sequenced page numbered from 0 breaks a rule a strict caller
        // refuses; it still answers what was read.
        first_sequence_number: Some(0),
    }
}

/// A ReadRange is one READ record. An answer that names what was read is
/// the device's answer, whichever rule of its page a strict caller refuses
/// it for; one that names another object answers nothing that was asked.
#[tokio::test(start_paused = true)]
async fn a_full_client_read_range_is_recorded_by_whether_it_was_answered() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let client = full_client(&session);
    let request = ReadRangeRequest {
        object_identifier: target(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range: Some(RangeSpec::BySequenceNumber {
            reference_seq: 1,
            count: 2,
        }),
    };
    for (object, expected) in [
        (target(), None),
        (
            oid(ObjectType::ANALOG_VALUE, 8),
            Some((ErrorClass::COMMUNICATION, ErrorCode::OTHER)),
        ),
    ] {
        let mac = peer.local_mac().to_vec();
        let sent = request.clone();
        let read = spawn_call(&client, move |client| async move {
            client
                .read_range_with(&mac[..], &sent, ReadRangeValidation::Strict)
                .await
        });
        let (from, invoke, service) = next_request(&mut requests).await;
        let mut ack = BytesMut::new();
        range_answer(&request, object).encode(&mut ack);
        send(&peer, &from, complex(invoke, service, ack)).await;
        assert!(read.await.unwrap().is_err(), "a strict caller refuses both");
        let [record] = <[_; 1]>::try_from(next_records(&mut records, 1).await).unwrap();
        assert_eq!(record.operation, AuditOperation::READ);
        assert_eq!(
            what(&record),
            reference(target(), PropertyIdentifier::LOG_BUFFER, None)
        );
        assert_eq!(record.result, expected);
    }
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// AtomicReadFile and AuditLogQuery are READ records naming the object
/// read and no property (Table 19-5).
#[tokio::test(start_paused = true)]
async fn a_full_client_file_read_and_log_query_name_no_property() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let client = full_client(&session);
    let file = oid(ObjectType::FILE, 3);
    let log = oid(ObjectType::AUDIT_LOG, 1);
    let mac = peer.local_mac().to_vec();
    let read = spawn_call(&client, move |client| async move {
        client
            .atomic_read_file(
                &mac[..],
                file,
                FileAccessMethod::Stream {
                    file_start_position: 0,
                    requested_octet_count: 4,
                },
            )
            .await
            .map(drop)
    });
    let (from, invoke, service) = next_request(&mut requests).await;
    let mut ack = BytesMut::new();
    AtomicReadFileAck {
        end_of_file: true,
        access: FileReadAckMethod::Stream {
            file_start_position: 0,
            file_data: vec![1, 2],
        },
    }
    .encode(&mut ack);
    send(&peer, &from, complex(invoke, service, ack)).await;
    assert!(read.await.unwrap().is_ok());
    let mac = peer.local_mac().to_vec();
    let query = spawn_call(&client, move |client| async move {
        let request = AuditLogQueryRequest {
            audit_log: log,
            query_parameters: BACnetAuditLogQueryParameters::BySource {
                source_device_identifier: oid(ObjectType::DEVICE, 1),
                source_device_address: None,
                source_object_identifier: None,
                operations: None,
                successful_actions_only: BACnetSuccessFilter::ALL,
            },
            start_at_sequence_number: None,
            requested_count: 5,
        };
        client.audit_log_query(&mac[..], &request).await.map(drop)
    });
    let (from, invoke, service) = next_request(&mut requests).await;
    let mut ack = BytesMut::new();
    AuditLogQueryAck {
        audit_log: log,
        records: Vec::new(),
        no_more_items: true,
    }
    .try_encode(&mut ack)
    .unwrap();
    send(&peer, &from, complex(invoke, service, ack)).await;
    assert!(query.await.unwrap().is_ok());
    let found = next_records(&mut records, 2).await;
    assert_eq!(
        found.iter().map(what).collect::<Vec<_>>(),
        [(Some(file), None), (Some(log), None)]
    );
    assert!(found.iter().all(|r| r.operation == AuditOperation::READ
        && r.result.is_none()
        && r.target_value.is_none()));
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// A WriteProperty with a priority is a command at that priority: its
/// record matches the narrow requester's commandable write. One without
/// is no command, as the narrow requester's noncommandable write is.
#[tokio::test(start_paused = true)]
async fn a_full_client_write_records_its_priority_as_a_commandable_write_does() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(
        database_for(BACnetPriorityFilter::all()),
        SessionRole::ClientOnly,
        &sink,
    );
    session.start().await.unwrap();
    let client = full_client(&session);
    for (priority, commandability) in [
        (Some(8), Commandability::Commandable),
        (None, Commandability::Noncommandable),
    ] {
        let mut pair = Vec::new();
        for narrow in [true, false] {
            let request = WritePropertyRequest {
                object_identifier: target(),
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                property_value: vec![0x44, 0x42, 0x28, 0, 0],
                priority,
            };
            let mac = peer.local_mac().to_vec();
            let write = if narrow {
                let handle = session.cloned_client_handle().unwrap();
                tokio::spawn(
                    async move { handle.write_property(&mac, request, commandability).await },
                )
            } else {
                spawn_call(&client, move |client| async move {
                    let WritePropertyRequest {
                        object_identifier,
                        property_identifier,
                        property_array_index,
                        property_value,
                        priority,
                    } = request;
                    client
                        .write_property(
                            &mac[..],
                            object_identifier,
                            property_identifier,
                            property_array_index,
                            property_value,
                            priority,
                        )
                        .await
                })
            };
            let (from, invoke, service) = next_request(&mut requests).await;
            send(&peer, &from, simple(invoke, service)).await;
            assert!(write.await.unwrap().is_ok());
            pair.extend(next_records(&mut records, 1).await);
        }
        let full = pair.pop().unwrap();
        let narrow = pair.pop().unwrap();
        assert_eq!(full.operation, AuditOperation::WRITE);
        assert_eq!(full.target_priority, priority);
        assert_eq!(full.target_value, Some(vec![0x44, 0x42, 0x28, 0, 0]));
        assert_eq!(fields(full), fields(narrow), "{priority:?}");
    }
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// The Audit_Priority_Filter skips a command at a priority it leaves out:
/// the write still goes, unrecorded. A write with no priority isn't
/// filtered by it, and a value over 32 octets is recorded without it.
#[tokio::test(start_paused = true)]
async fn a_full_client_write_is_filtered_by_its_priority_alone() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    // Every priority but 8.
    let mut filter = BACnetPriorityFilter::all();
    filter.set(8, false).unwrap();
    let mut session = session(database_for(filter), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let client = full_client(&session);
    let long = [vec![0x75, 33, 0], vec![b'x'; 32]].concat();
    for (priority, value, recorded) in [
        (Some(8), vec![0x21, 1], false),
        (None, vec![0x21, 2], true),
        (Some(9), long.clone(), true),
    ] {
        let mac = peer.local_mac().to_vec();
        let write = spawn_call(&client, move |client| async move {
            client
                .write_property(
                    &mac[..],
                    target(),
                    PropertyIdentifier::DESCRIPTION,
                    None,
                    value,
                    priority,
                )
                .await
        });
        let (from, invoke, service) = next_request(&mut requests).await;
        send(&peer, &from, simple(invoke, service)).await;
        assert!(write.await.unwrap().is_ok());
        if recorded {
            let [record] = <[_; 1]>::try_from(next_records(&mut records, 1).await).unwrap();
            assert_eq!(record.invoke_id, Some(invoke));
            assert_eq!(record.target_priority, priority);
            let expected = (priority.is_none()).then(|| vec![0x21, 2]);
            assert_eq!(record.target_value, expected);
        } else {
            assert!(
                timeout(Duration::from_millis(30), records.recv())
                    .await
                    .is_err()
            );
        }
    }
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// WritePropertyMultiple makes a record per write. When the device
/// refuses one, those before it were made and those from it on were not.
#[tokio::test(start_paused = true)]
async fn a_full_client_wpm_records_each_write_up_to_the_one_refused() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(
        database_for(BACnetPriorityFilter::all()),
        SessionRole::ClientOnly,
        &sink,
    );
    session.start().await.unwrap();
    let client = full_client(&session);
    let other = oid(ObjectType::ANALOG_VALUE, 8);
    let value = |property_identifier, priority, value: u8| BACnetPropertyValue {
        property_identifier,
        property_array_index: None,
        value: vec![0x21, value],
        priority,
    };
    let specs = vec![
        WriteAccessSpecification {
            object_identifier: target(),
            list_of_properties: vec![
                value(PropertyIdentifier::PRESENT_VALUE, Some(10), 1),
                value(PropertyIdentifier::DESCRIPTION, None, 2),
            ],
        },
        WriteAccessSpecification {
            object_identifier: other,
            list_of_properties: vec![value(PropertyIdentifier::PRESENT_VALUE, Some(12), 3)],
        },
    ];
    for refused in [false, true] {
        let mac = peer.local_mac().to_vec();
        let sent = specs.clone();
        let write = spawn_call(&client, move |client| async move {
            client.write_property_multiple(&mac[..], sent).await
        });
        let (from, invoke, service) = next_request(&mut requests).await;
        let answer = if refused {
            // The second write failed: WritePropertyMultiple-Error names it.
            Apdu::Error(
                WritePropertyMultipleError {
                    error_class: ErrorClass::PROPERTY,
                    error_code: ErrorCode::WRITE_ACCESS_DENIED,
                    first_failed_write_attempt: BACnetObjectPropertyReference {
                        object_identifier: target(),
                        property_identifier: PropertyIdentifier::DESCRIPTION.to_raw(),
                        property_array_index: None,
                    },
                }
                .to_error_pdu(invoke),
            )
        } else {
            simple(invoke, service)
        };
        send(&peer, &from, answer).await;
        assert_eq!(write.await.unwrap().is_ok(), !refused);
        let found = next_records(&mut records, 3).await;
        assert_eq!(
            found.iter().map(what).collect::<Vec<_>>(),
            [
                reference(target(), PropertyIdentifier::PRESENT_VALUE, None),
                reference(target(), PropertyIdentifier::DESCRIPTION, None),
                reference(other, PropertyIdentifier::PRESENT_VALUE, None),
            ]
        );
        assert_eq!(
            found
                .iter()
                .map(|r| (r.target_priority, r.target_value.clone()))
                .collect::<Vec<_>>(),
            [
                (Some(10), Some(vec![0x21, 1])),
                (None, Some(vec![0x21, 2])),
                (Some(12), Some(vec![0x21, 3])),
            ]
        );
        let failed = Some((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED));
        let expected = if refused {
            [None, failed, failed]
        } else {
            [None, None, None]
        };
        assert_eq!(found.iter().map(|r| r.result).collect::<Vec<_>>(), expected);
    }
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// AtomicWriteFile is a WRITE record naming the file and no property or
/// value (Table 19-5); AddListElement and RemoveListElement name the list
/// and the elements as their value.
#[tokio::test(start_paused = true)]
async fn a_full_client_file_and_list_writes_are_write_records() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(
        database_for(BACnetPriorityFilter::all()),
        SessionRole::ClientOnly,
        &sink,
    );
    session.start().await.unwrap();
    let client = full_client(&session);
    let file = oid(ObjectType::FILE, 3);
    let mac = peer.local_mac().to_vec();
    let write = spawn_call(&client, move |client| async move {
        let access = FileWriteAccessMethod::Stream {
            file_start_position: 0,
            file_data: vec![1, 2, 3],
        };
        client
            .atomic_write_file(&mac[..], file, access)
            .await
            .map(drop)
    });
    let (from, invoke, service) = next_request(&mut requests).await;
    let mut ack = BytesMut::new();
    bacnet_encoding::primitives::encode_ctx_signed(&mut ack, 0, 0);
    send(&peer, &from, complex(invoke, service, ack)).await;
    assert!(write.await.unwrap().is_ok());
    for add in [true, false] {
        let mac = peer.local_mac().to_vec();
        let change = spawn_call(&client, move |client| async move {
            let (object, property, index, elements) = (
                target(),
                PropertyIdentifier::RECIPIENT_LIST,
                None,
                vec![0x21, 7],
            );
            if add {
                client
                    .add_list_element(&mac[..], object, property, index, elements)
                    .await
            } else {
                client
                    .remove_list_element(&mac[..], object, property, index, elements)
                    .await
            }
        });
        let (from, invoke, service) = next_request(&mut requests).await;
        send(&peer, &from, simple(invoke, service)).await;
        assert!(change.await.unwrap().is_ok());
    }
    let found = next_records(&mut records, 3).await;
    assert!(found.iter().all(|r| r.operation == AuditOperation::WRITE
        && r.result.is_none()
        && r.target_priority.is_none()));
    assert_eq!(
        (what(&found[0]), found[0].target_value.clone()),
        ((Some(file), None), None)
    );
    for record in &found[1..] {
        assert_eq!(
            what(record),
            reference(target(), PropertyIdentifier::RECIPIENT_LIST, None)
        );
        assert_eq!(record.target_value, Some(vec![0x21, 7]));
    }
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}
