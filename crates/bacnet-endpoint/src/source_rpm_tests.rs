use super::*;
use bacnet_services::rpm::{
    ReadAccessResult, ReadPropertyMultipleACK, ReadPropertyMultipleRequest, ReadResultElement,
};
use bacnet_transport::test_transport::TestTransport;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};

fn specs() -> Vec<ReadAccessSpecification> {
    vec![
        ReadAccessSpecification {
            object_identifier: oid(ObjectType::DEVICE, 10),
            list_of_property_references: vec![
                PropertyReference {
                    property_identifier: PropertyIdentifier::OBJECT_NAME,
                    property_array_index: None,
                },
                PropertyReference {
                    property_identifier: PropertyIdentifier::OBJECT_LIST,
                    property_array_index: Some(0),
                },
                PropertyReference {
                    property_identifier: PropertyIdentifier::OBJECT_LIST,
                    property_array_index: Some(2),
                },
                PropertyReference {
                    property_identifier: PropertyIdentifier::OBJECT_LIST,
                    property_array_index: Some(2),
                },
            ],
        },
        ReadAccessSpecification {
            object_identifier: target(),
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
            }],
        },
    ]
}
/// A ReadPropertyMultiple of `specs`, in a task of its own.
fn start(
    client: &Client,
    mac: &[u8],
    specs: Vec<ReadAccessSpecification>,
) -> tokio::task::JoinHandle<Result<ReadPropertyMultipleACK, Error>> {
    let client = Arc::clone(client);
    let mac = mac.to_vec();
    tokio::spawn(async move { client.read_property_multiple(&mac[..], specs).await })
}
fn request(envelope: &ReceivedApdu) -> (u8, ReadPropertyMultipleRequest) {
    let Apdu::ConfirmedRequest(pdu) = decode_apdu(envelope.apdu.clone()).unwrap() else {
        panic!()
    };
    assert_eq!(
        pdu.service_choice,
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE
    );
    (
        pdu.invoke_id,
        ReadPropertyMultipleRequest::decode(&pdu.service_request).unwrap(),
    )
}
fn success(request: &ReadPropertyMultipleRequest) -> ReadPropertyMultipleACK {
    ReadPropertyMultipleACK {
        list_of_read_access_results: request
            .list_of_read_access_specs
            .iter()
            .map(|s| ReadAccessResult {
                object_identifier: s.object_identifier,
                list_of_results: s
                    .list_of_property_references
                    .iter()
                    .map(|p| ReadResultElement {
                        property_identifier: p.property_identifier,
                        property_array_index: p.property_array_index,
                        property_value: Some(vec![0x21, 42]),
                        error: None,
                    })
                    .collect(),
            })
            .collect(),
    }
}
fn response(invoke: u8, ack: &ReadPropertyMultipleACK) -> Apdu {
    let mut bytes = BytesMut::new();
    ack.encode(&mut bytes).unwrap();
    Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id: invoke,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        service_ack: bytes.freeze(),
    })
}
/// What a record names: its object, property and array index.
fn what(record: &BACnetAuditNotification) -> (ObjectIdentifier, PropertyIdentifier, Option<u64>) {
    let property = record.target_property.as_ref().expect("a property");
    (
        record.target_object.expect("an object"),
        property.property_identifier,
        property.property_array_index,
    )
}
/// What `request` asks for, reference by reference.
fn asked(
    request: &ReadPropertyMultipleRequest,
) -> Vec<(ObjectIdentifier, PropertyIdentifier, Option<u64>)> {
    request
        .list_of_read_access_specs
        .iter()
        .flat_map(|spec| {
            spec.list_of_property_references.iter().map(|reference| {
                (
                    spec.object_identifier,
                    reference.property_identifier,
                    reference.property_array_index.map(u64::from),
                )
            })
        })
        .collect()
}
async fn records(
    receiver: &mut bacnet_network::layer::ApduReceiver,
    sink: &NetworkLayer<TestTransport>,
    confirmed: bool,
    n: usize,
) -> Vec<BACnetAuditNotification> {
    let mut out = Vec::new();
    for _ in 0..n {
        let env = receive(receiver).await;
        let (record, id) = notification(&env, confirmed);
        assert!(record.target_value.is_none() && record.current_value.is_none());
        if let Some(invoke_id) = id {
            send(
                sink,
                &env.source_mac,
                Apdu::SimpleAck(SimpleAck {
                    invoke_id,
                    service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
                }),
            )
            .await;
        }
        out.push(record);
    }
    out
}
#[tokio::test(start_paused = true)]
async fn source_rpm_ordered_occurrences_mixed_results_both_roles_modes() {
    let (peer, mut requests) = network().await;
    let (sink, mut received) = network().await;
    for role in [SessionRole::ClientOnly, SessionRole::Both] {
        for confirmed in [false, true] {
            let mut session = session(database(confirmed), role, &sink);
            session.start().await.unwrap();
            let client = client_of(&session);
            let read = start(&client, peer.local_mac(), specs());
            let env = receive(&mut requests).await;
            let (invoke, rpm) = request(&env);
            let mut ack = success(&rpm);
            for (i, code) in [
                (2, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
                (3, ErrorCode::UNKNOWN_PROPERTY),
            ] {
                let result = &mut ack.list_of_read_access_results[0].list_of_results[i];
                result.property_value = None;
                result.error = Some((ErrorClass::PROPERTY, code));
                result.property_array_index = None;
            }
            send(&peer, &env.source_mac, response(invoke, &ack)).await;
            assert_eq!(read.await.unwrap().unwrap(), ack);
            let actual = records(&mut received, &sink, confirmed, 5).await;
            for (i, record) in actual.iter().enumerate() {
                assert_eq!(record.invoke_id, Some(invoke));
                assert_stamp(&record.source_timestamp, role == SessionRole::Both, 0);
                assert_eq!(
                    record.target_device,
                    BACnetRecipient::Device(oid(ObjectType::DEVICE, 10))
                );
                let (spec, element) = if i < 4 {
                    (&rpm.list_of_read_access_specs[0], i)
                } else {
                    (&rpm.list_of_read_access_specs[1], 0)
                };
                assert_eq!(record.target_object, Some(spec.object_identifier));
                let prop = record.target_property.as_ref().unwrap();
                assert_eq!(
                    prop.property_identifier,
                    spec.list_of_property_references[element].property_identifier
                );
                assert_eq!(
                    prop.property_array_index,
                    spec.list_of_property_references[element]
                        .property_array_index
                        .map(u64::from)
                );
                assert_eq!(
                    record.result,
                    if i < 4 {
                        ack.list_of_read_access_results[0].list_of_results[i].error
                    } else {
                        None
                    }
                );
            }
            assert!(received.try_recv().is_err());
            session.stop().await.unwrap();
            assert_eq!(session.active_leases(), 0);
        }
    }
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}
/// A ReadPropertyMultiple that fails as a whole records every reference
/// it asked for with the failure's Result, on the station it was sent to:
/// a peer's Error, an answer that doesn't decode, no answer, an Abort and
/// a Reject.
#[tokio::test(start_paused = true)]
async fn source_rpm_whole_operation_failure_records_every_reference() {
    let (peer, mut requests) = network().await;
    let (sink, mut received) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let client = client_of(&session);
    let communication = |code| (ErrorClass::COMMUNICATION, code);
    for (fault, expected) in [
        (9, (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT)),
        (10, communication(ErrorCode::OTHER)),
        (11, communication(ErrorCode::TIMEOUT)),
        (12, communication(ErrorCode::ABORT_OTHER)),
        (13, communication(ErrorCode::REJECT_UNRECOGNIZED_SERVICE)),
    ] {
        let read = start(&client, peer.local_mac(), specs());
        let env = receive(&mut requests).await;
        let (invoke, rpm) = request(&env);
        let pdu = match fault {
            12 => Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id: invoke,
                abort_reason: AbortReason::OTHER,
            }),
            13 => Apdu::Reject(RejectPdu {
                invoke_id: invoke,
                reject_reason: RejectReason::UNRECOGNIZED_SERVICE,
            }),
            9 => Apdu::Error(ErrorPdu {
                invoke_id: invoke,
                service_choice: ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                error_class: ErrorClass::OBJECT,
                error_code: ErrorCode::UNKNOWN_OBJECT,
                error_data: Bytes::new(),
            }),
            _ => {
                let Apdu::ComplexAck(mut a) = response(invoke, &success(&rpm)) else {
                    unreachable!()
                };
                a.service_ack = Bytes::from_static(&[0]);
                Apdu::ComplexAck(a)
            }
        };
        if fault != 11 {
            send(&peer, &env.source_mac, pdu).await;
        }
        assert!(read.await.unwrap().is_err(), "fault {fault}");
        let found = records(&mut received, &sink, false, 5).await;
        assert_eq!(
            found.iter().map(what).collect::<Vec<_>>(),
            asked(&rpm),
            "fault {fault}"
        );
        for record in found {
            assert!(matches!(record.target_device, BACnetRecipient::Address(_)));
            assert_eq!(record.result, Some(expected), "fault {fault}");
            assert_eq!(record.invoke_id, Some(invoke));
        }
    }
    assert!(received.try_recv().is_err());
    session.stop().await.unwrap();
    assert_eq!(session.active_leases(), 0);
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}
#[tokio::test(start_paused = true)]
async fn source_rpm_unique_successful_device_projection_and_per_reference_filters() {
    let (peer, mut requests) = network().await;
    let (sink, mut received) = network().await;
    for case in 0..6 {
        let mut db = database(false);
        if case == 3 {
            db.get_mut(&selected())
                .unwrap()
                .audit_reporter_mut()
                .expect("an Audit Reporter")
                .configure(
                    AuditLevel::AUDIT_CONFIG,
                    AuditOperationFlags::from_bits(1).unwrap(),
                    false,
                    None,
                    BACnetPriorityFilter::empty(),
                    None,
                )
                .unwrap();
        }
        let mut session = session(db, SessionRole::ClientOnly, &sink);
        session.start().await.unwrap();
        let client = client_of(&session);
        let mut specs = specs();
        if matches!(case, 1 | 4 | 5) {
            let mut second = specs[0].clone();
            second.object_identifier = oid(ObjectType::DEVICE, if case == 4 { 10 } else { 11 });
            specs.push(second);
        }
        let read = start(&client, peer.local_mac(), specs);
        let env = receive(&mut requests).await;
        let (invoke, rpm) = request(&env);
        let mut ack = success(&rpm);
        if case == 2 {
            for element in &mut ack.list_of_read_access_results[0].list_of_results {
                element.property_value = None;
                element.error = Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY));
            }
        }
        if case == 5 {
            for element in &mut ack.list_of_read_access_results[2].list_of_results {
                element.property_value = None;
                element.error = Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY));
            }
        }
        send(&peer, &env.source_mac, response(invoke, &ack)).await;
        assert!(read.await.unwrap().is_ok());
        for record in records(
            &mut received,
            &sink,
            false,
            if matches!(case, 1 | 4 | 5) {
                9
            } else if case == 3 {
                4
            } else {
                5
            },
        )
        .await
        {
            if case == 1 || case == 2 {
                assert!(matches!(record.target_device, BACnetRecipient::Address(_)));
            } else {
                assert_eq!(
                    record.target_device,
                    BACnetRecipient::Device(oid(ObjectType::DEVICE, 10))
                );
            }
            if case == 3 {
                assert_ne!(
                    record.target_property.unwrap().property_identifier,
                    PropertyIdentifier::PRESENT_VALUE
                );
            }
        }
        assert!(received.try_recv().is_err());
        session.stop().await.unwrap();
    }
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[path = "source_rpm_lifecycle_tests.rs"]
mod lifecycle;

#[path = "source_rpm_congestion_tests.rs"]
mod congestion;
