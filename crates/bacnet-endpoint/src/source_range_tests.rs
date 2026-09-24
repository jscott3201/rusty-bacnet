use super::*;
use bacnet_services::read_range::{RangeSpec, ReadRangeAck, ReadRangeRequest};
use bacnet_types::primitives::{Date, Time};

fn range_start(
    session: &EndpointSession<BipTransport>,
    mac: &[u8],
    range: Option<RangeSpec>,
) -> tokio::task::JoinHandle<Result<ReadRangeAck, Error>> {
    let client = session.cloned_client_handle().unwrap();
    let mac = mac.to_vec();
    tokio::spawn(async move {
        client
            .read_range(&mac, target(), PropertyIdentifier::LOG_BUFFER, None, range)
            .await
    })
}
fn range_request(envelope: &ReceivedApdu) -> (u8, ReadRangeRequest) {
    let Apdu::ConfirmedRequest(pdu) = decode_apdu(envelope.apdu.clone()).unwrap() else {
        panic!()
    };
    assert_eq!(pdu.service_choice, ConfirmedServiceChoice::READ_RANGE);
    assert!(!pdu.segmented_response_accepted);
    (
        pdu.invoke_id,
        ReadRangeRequest::decode(&pdu.service_request).unwrap(),
    )
}
fn range_ack(invoke_id: u8, request: &ReadRangeRequest, count: u32) -> (ReadRangeAck, Apdu) {
    let sequence = matches!(
        request.range,
        Some(RangeSpec::BySequenceNumber { .. } | RangeSpec::ByTime { .. })
    );
    let ack = ReadRangeAck {
        object_identifier: request.object_identifier,
        property_identifier: request.property_identifier,
        property_array_index: request.property_array_index,
        result_flags: (true, true, false),
        item_count: count,
        item_data: if count == 0 {
            vec![]
        } else {
            vec![0x21, 7, 0x21, 9]
        },
        first_sequence_number: (sequence && count > 0).then_some(42),
    };
    let mut bytes = BytesMut::new();
    ack.encode(&mut bytes);
    let pdu = Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_RANGE,
        service_ack: bytes.freeze(),
    });
    (ack, pdu)
}
fn ranges() -> Vec<Option<RangeSpec>> {
    vec![
        None,
        Some(RangeSpec::ByPosition {
            reference_index: 1,
            count: 2,
        }),
        Some(RangeSpec::BySequenceNumber {
            reference_seq: 43,
            count: -2,
        }),
        Some(RangeSpec::ByTime {
            reference_time: (
                Date {
                    year: 126,
                    month: 9,
                    day: 23,
                    day_of_week: 3,
                },
                Time {
                    hour: 12,
                    minute: 0,
                    second: 0,
                    hundredths: 17,
                },
            ),
            count: 2,
        }),
    ]
}

#[tokio::test]
async fn source_read_range_choices_empty_and_multiple_items_both_modes() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    for confirmed in [false, true] {
        let mut session = session(database(confirmed), SessionRole::Both, &sink);
        session.start().await.unwrap();
        let mut sequence = 0;
        for range in ranges() {
            for count in [0, 2] {
                let read = range_start(&session, peer.local_mac(), range.clone());
                let request = receive(&mut requests).await;
                let (invoke, rr) = range_request(&request);
                assert_eq!(rr.range, range);
                let (expected, ack) = range_ack(invoke, &rr, count);
                send(&peer, &request.source_mac, ack).await;
                let actual = timeout(WAIT, read).await.unwrap().unwrap().unwrap();
                assert_eq!(actual.object_identifier, expected.object_identifier);
                assert_eq!(actual.property_identifier, expected.property_identifier);
                assert_eq!(actual.property_array_index, expected.property_array_index);
                assert_eq!(actual.result_flags, expected.result_flags);
                assert_eq!(actual.item_count, expected.item_count);
                assert_eq!(actual.item_data, expected.item_data);
                assert_eq!(actual.first_sequence_number, expected.first_sequence_number);
                // Caller completes before the confirmed Audit delivery is acknowledged.
                let envelope = receive(&mut records).await;
                let (record, delivery_id) = notification(&envelope, confirmed);
                assert_eq!(record.invoke_id, Some(invoke));
                assert_eq!(record.operation, AuditOperation::READ);
                assert_eq!(
                    record.source_timestamp,
                    Some(BACnetTimeStamp::SequenceNumber(sequence))
                );
                sequence += 1;
                assert_eq!(
                    record.source_device,
                    BACnetRecipient::Device(oid(ObjectType::DEVICE, 123))
                );
                assert_eq!(record.target_object, Some(target()));
                let property = record.target_property.unwrap();
                assert_eq!(property.property_identifier, PropertyIdentifier::LOG_BUFFER);
                assert_eq!(property.property_array_index, None);
                assert!(
                    record.target_value.is_none()
                        && record.current_value.is_none()
                        && record.target_timestamp.is_none()
                );
                assert_eq!(record.result, None);
                if let Some(invoke_id) = delivery_id {
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
            }
        }
        session.stop().await.unwrap();
        assert_eq!(session.active_leases(), 0);
    }
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_read_range_terminal_errors_and_segmented_attempt_are_reported() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    for case in 0..9 {
        let read = range_start(&session, peer.local_mac(), None);
        let request = receive(&mut requests).await;
        let (invoke_id, mut rr) = range_request(&request);
        let communication = (ErrorClass::COMMUNICATION, ErrorCode::OTHER);
        let (reply, expected) = match case {
            0 => (
                Some(Apdu::Error(ErrorPdu {
                    invoke_id,
                    service_choice: ConfirmedServiceChoice::READ_RANGE,
                    error_class: ErrorClass::PROPERTY,
                    error_code: ErrorCode::UNKNOWN_PROPERTY,
                    error_data: Bytes::new(),
                })),
                (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
            ),
            1 => (
                Some(Apdu::Reject(RejectPdu {
                    invoke_id,
                    reject_reason: RejectReason::INVALID_PARAMETER_DATA_TYPE,
                })),
                (
                    ErrorClass::COMMUNICATION,
                    ErrorCode::REJECT_INVALID_PARAMETER_DATA_TYPE,
                ),
            ),
            2 => (
                Some(Apdu::Abort(AbortPdu {
                    invoke_id,
                    sent_by_server: true,
                    abort_reason: AbortReason::OUT_OF_RESOURCES,
                })),
                (ErrorClass::COMMUNICATION, ErrorCode::ABORT_OUT_OF_RESOURCES),
            ),
            3 => (None, (ErrorClass::COMMUNICATION, ErrorCode::TIMEOUT)),
            4..=6 => {
                match case {
                    4 => rr.object_identifier = oid(ObjectType::ANALOG_VALUE, 99),
                    5 => rr.property_identifier = PropertyIdentifier::PRESENT_VALUE,
                    _ => rr.property_array_index = Some(2),
                };
                (Some(range_ack(invoke_id, &rr, 2).1), communication)
            }
            7 => {
                let (_, mut ack) = range_ack(invoke_id, &rr, 2);
                let Apdu::ComplexAck(ref mut pdu) = ack else {
                    unreachable!()
                };
                pdu.service_ack = Bytes::from_static(&[0]);
                (Some(ack), communication)
            }
            _ => {
                let (_, mut ack) = range_ack(invoke_id, &rr, 2);
                let Apdu::ComplexAck(ref mut pdu) = ack else {
                    unreachable!()
                };
                pdu.segmented = true;
                pdu.more_follows = true;
                pdu.sequence_number = Some(0);
                pdu.proposed_window_size = Some(1);
                (
                    Some(ack),
                    (
                        ErrorClass::COMMUNICATION,
                        ErrorCode::ABORT_INVALID_APDU_IN_THIS_STATE,
                    ),
                )
            }
        };
        if let Some(reply) = reply {
            send(&peer, &request.source_mac, reply).await;
        }
        assert!(timeout(WAIT, read).await.unwrap().unwrap().is_err());
        let (record, _) = notification(&receive(&mut records).await, false);
        assert_eq!(record.result, Some(expected), "case{case}");
        assert_eq!(record.invoke_id, Some(invoke_id));
        assert_eq!(
            record.source_timestamp,
            Some(BACnetTimeStamp::SequenceNumber(case))
        );
        assert!(record.target_value.is_none());
        if case == 8 {
            let abort = receive(&mut requests).await;
            assert!(
                matches!(decode_apdu(abort.apdu).unwrap(),Apdu::Abort(pdu) if pdu.abort_reason==AbortReason::SEGMENTATION_NOT_SUPPORTED)
            );
        }
    }
    session.stop().await.unwrap();
    assert_eq!(session.active_leases(), 0);
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_read_range_invalid_preflight_does_not_consume_sequence_or_lease() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let client = session.cloned_client_handle().unwrap();
    for (property, index, range) in [
        (PropertyIdentifier::ALL, None, None),
        (PropertyIdentifier::REQUIRED, None, None),
        (PropertyIdentifier::OPTIONAL, None, None),
        (PropertyIdentifier::LOG_BUFFER, Some(0), None),
        (
            PropertyIdentifier::LOG_BUFFER,
            None,
            Some(RangeSpec::ByPosition {
                reference_index: 1,
                count: 0,
            }),
        ),
        (
            PropertyIdentifier::LOG_BUFFER,
            None,
            Some(RangeSpec::BySequenceNumber {
                reference_seq: 1,
                count: 32768,
            }),
        ),
        (
            PropertyIdentifier::LOG_BUFFER,
            None,
            Some(RangeSpec::ByTime {
                reference_time: (
                    Date {
                        year: 255,
                        month: 1,
                        day: 1,
                        day_of_week: 1,
                    },
                    Time {
                        hour: 0,
                        minute: 0,
                        second: 0,
                        hundredths: 0,
                    },
                ),
                count: 1,
            }),
        ),
    ] {
        assert!(client
            .read_range(peer.local_mac(), target(), property, index, range)
            .await
            .is_err());
    }
    assert!(client
        .read_range_with_destination(
            bacnet_endpoint_core::endpoint_ingress::EndpointApduDestination::LocalBroadcast,
            vec![],
            target(),
            PropertyIdentifier::LOG_BUFFER,
            None,
            None
        )
        .await
        .is_err());
    assert_eq!(session.active_leases(), 0);
    assert_eq!(
        session.source_read.as_ref().unwrap().available_operations(),
        64
    );
    assert!(requests.try_recv().is_err());
    assert!(records.try_recv().is_err());
    let read = range_start(&session, peer.local_mac(), None);
    let envelope = receive(&mut requests).await;
    let (invoke, rr) = range_request(&envelope);
    send(&peer, &envelope.source_mac, range_ack(invoke, &rr, 0).1).await;
    read.await.unwrap().unwrap();
    let (record, _) = notification(&receive(&mut records).await, false);
    assert_eq!(
        record.source_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(0))
    );
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_read_range_cancelled_caller_keeps_one_retry_record_and_stop_releases() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.client_config.apdu_retries = 1;
    session.start().await.unwrap();
    let read = range_start(&session, peer.local_mac(), None);
    let first = receive(&mut requests).await;
    let (invoke, rr) = range_request(&first);
    read.abort();
    assert!(read.await.unwrap_err().is_cancelled());
    let retry = receive(&mut requests).await;
    assert_eq!(range_request(&retry).0, invoke);
    send(&peer, &retry.source_mac, range_ack(invoke, &rr, 2).1).await;
    let (record, _) = notification(&receive(&mut records).await, false);
    assert_eq!(record.invoke_id, Some(invoke));
    assert_eq!(record.result, None);
    assert!(timeout(Duration::from_millis(30), records.recv())
        .await
        .is_err());
    let read = range_start(&session, peer.local_mac(), None);
    receive(&mut requests).await;
    session.stop().await.unwrap();
    assert!(read.await.unwrap().is_err());
    assert_eq!(session.active_leases(), 0);
    let client = session.cloned_client_handle();
    assert!(client.is_none());
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_read_range_snapshot_and_generation_survive_recipient_change() {
    let (mut peer, mut requests) = network().await;
    let (mut old, mut old_records) = network().await;
    let (mut new, mut new_records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &old);
    session.start().await.unwrap();
    let read = range_start(&session, peer.local_mac(), None);
    let envelope = receive(&mut requests).await;
    let (invoke, rr) = range_request(&envelope);
    let recipient = BACnetRecipient::Address(bacnet_types::constructed::BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(new.local_mac()),
    });
    session
        .write_audit_recipient(Some(recipient))
        .await
        .unwrap();
    assert_eq!(
        notification(&receive(&mut old_records).await, false)
            .0
            .operation,
        AuditOperation::WRITE
    );
    assert_eq!(
        notification(&receive(&mut new_records).await, false)
            .0
            .operation,
        AuditOperation::WRITE
    );
    // The admitted request keeps its old route. Its failed operation is content,
    // not a late delivery completion permitted to overwrite a new generation.
    let (_, mut bad) = range_ack(invoke, &rr, 2);
    let Apdu::ComplexAck(ref mut pdu) = bad else {
        unreachable!()
    };
    pdu.service_ack = Bytes::from_static(&[0]);
    send(&peer, &envelope.source_mac, bad).await;
    assert!(read.await.unwrap().is_err());
    let record = notification(&receive(&mut old_records).await, false).0;
    assert_eq!(record.operation, AuditOperation::READ);
    assert_eq!(
        record.result,
        Some((ErrorClass::COMMUNICATION, ErrorCode::OTHER))
    );
    let read = range_start(&session, peer.local_mac(), None);
    let envelope = receive(&mut requests).await;
    let (invoke, rr) = range_request(&envelope);
    send(&peer, &envelope.source_mac, range_ack(invoke, &rr, 0).1).await;
    read.await.unwrap().unwrap();
    assert_eq!(
        notification(&receive(&mut new_records).await, false)
            .0
            .operation,
        AuditOperation::READ
    );
    assert!(old_records.try_recv().is_err());
    session.stop().await.unwrap();
    assert_eq!(session.active_leases(), 0);
    peer.stop().await.unwrap();
    old.stop().await.unwrap();
    new.stop().await.unwrap();
}

#[tokio::test]
async fn source_read_range_drop_unblocks_retained_caller_and_releases_database() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, _records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let db = Arc::clone(session.database.as_ref().unwrap());
    let client = session.cloned_client_handle().unwrap();
    let read = range_start(&session, peer.local_mac(), None);
    receive(&mut requests).await;
    let coordinator = Arc::clone(&session.coordinator);
    drop(session);
    assert!(timeout(WAIT, read).await.unwrap().unwrap().is_err());
    assert!(client
        .read_range(
            peer.local_mac(),
            target(),
            PropertyIdentifier::LOG_BUFFER,
            None,
            None
        )
        .await
        .is_err());
    timeout(WAIT, async {
        loop {
            let mut db = db.write().await;
            if db.remove(&oid(ObjectType::DEVICE, 123)).is_ok() {
                break;
            }
            drop(db);
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(coordinator.active_count().unwrap(), 0);
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}
