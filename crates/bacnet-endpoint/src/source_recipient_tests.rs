//! Real source session changes: one Device value, paired delivery, and read snapshots.
use super::*;
use bacnet_encoding::apdu::ConfirmedRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::{AuditPropertyReference, BACnetAddress};

fn device(n: u32) -> BACnetRecipient {
    BACnetRecipient::Device(oid(ObjectType::DEVICE, n))
}
fn value(recipient: &BACnetRecipient) -> Vec<u8> {
    let mut data = BytesMut::new();
    bacnet_encoding::constructed::encode_recipient(&mut data, recipient);
    data.to_vec()
}
fn direct(network: &NetworkLayer<BipTransport>) -> BACnetRecipient {
    BACnetRecipient::Address(BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(network.local_mac()),
    })
}
async fn local(session: &EndpointSession<BipTransport>, value: PropertyValue) -> Result<(), Error> {
    session
        .database
        .as_ref()
        .unwrap()
        .write()
        .await
        .get_mut(&oid(ObjectType::DEVICE, 123))
        .unwrap()
        .write_property(
            PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
            None,
            value,
            None,
        )
}
async fn current(session: &EndpointSession<BipTransport>) -> PropertyValue {
    session
        .database
        .as_ref()
        .unwrap()
        .read()
        .await
        .get(&oid(ObjectType::DEVICE, 123))
        .unwrap()
        .read_property(PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT, None)
        .unwrap()
}
async fn wire(
    session: &EndpointSession<BipTransport>,
    peer: &NetworkLayer<BipTransport>,
    replies: &mut mpsc::Receiver<ReceivedApdu>,
    bytes: Vec<u8>,
    invoke: u8,
) -> Apdu {
    let mut service = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid(ObjectType::DEVICE, 123),
        property_identifier: PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
        property_array_index: None,
        property_value: bytes,
        priority: None,
    }
    .encode(&mut service)
    .unwrap();
    // Discover the ephemeral local address through the session's actual egress.
    session
        .egress
        .as_ref()
        .unwrap()
        .send_apdu(
            vec![0x10, 8],
            bacnet_endpoint_core::endpoint_ingress::EndpointApduDestination::Direct {
                destination_mac: MacAddr::from_slice(peer.local_mac()),
            },
            false,
            NetworkPriority::NORMAL,
            Vec::new(),
        )
        .await
        .unwrap();
    let source = receive(replies).await.source_mac;
    send(
        peer,
        &source,
        Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: invoke,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            service_request: service.freeze(),
        }),
    )
    .await;
    decode_apdu(receive(replies).await.apdu).unwrap()
}
fn assert_record(
    record: &BACnetAuditNotification,
    old: &BACnetRecipient,
    new: &BACnetRecipient,
    source: BACnetRecipient,
    invoke: Option<u8>,
    sequence: u16,
) {
    assert_eq!(record.operation, AuditOperation::WRITE);
    assert_eq!(record.source_device, source);
    assert_eq!(record.invoke_id, invoke);
    assert_eq!(record.source_timestamp, None);
    assert_eq!(
        record.target_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(sequence))
    );
    assert_eq!(record.target_device, device(123));
    assert_eq!(record.target_object, Some(oid(ObjectType::DEVICE, 123)));
    assert_eq!(
        record.target_property,
        Some(AuditPropertyReference {
            property_identifier: PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
            property_array_index: None
        })
    );
    assert_eq!(record.current_value, Some(value(old)));
    assert_eq!(record.target_value, Some(value(new)));
    assert_eq!(record.target_priority, None);
    assert_eq!(record.result, None);
}

#[tokio::test]
async fn source_recipient_local_and_authorized_wire_pair_bypass_ordinary_filters() {
    let (mut old, mut old_records) = network().await;
    let (mut new, mut new_records) = network().await;
    let (mut peer, mut replies) = network().await;
    for role in [SessionRole::ClientOnly, SessionRole::Both] {
        for level in [AuditLevel::NONE, AuditLevel::AUDIT_ALL] {
            let mut db = database(false);
            db.set_clock_reader(None);
            db.get_mut(&selected())
                .unwrap()
                .configure_audit_reporter_internal(
                    level,
                    AuditOperationFlags::empty(),
                    false,
                    None,
                    BACnetPriorityFilter::empty(),
                )
                .unwrap();
            let mut session = session(db, role, &old);
            session
                .source_audit_bindings
                .push((oid(ObjectType::DEVICE, 1000), address(&new)));
            session.start().await.unwrap();
            assert!(old_records.try_recv().is_err());
            assert!(new_records.try_recv().is_err());
            let next = if level == AuditLevel::NONE {
                device(1000)
            } else {
                direct(&new)
            };
            if role == SessionRole::Both {
                assert!(
                    matches!(wire(&session,&peer,&mut replies,value(&next),71).await,Apdu::SimpleAck(a) if a.invoke_id==71)
                );
            } else {
                session
                    .write_audit_recipient(Some(next.clone()))
                    .await
                    .unwrap();
            }
            let (first, _) = notification(&receive(&mut old_records).await, false);
            let (second, _) = notification(&receive(&mut new_records).await, false);
            assert_eq!(first, second);
            assert_record(
                &first,
                &device(999),
                &next,
                if role == SessionRole::Both {
                    direct(&peer)
                } else {
                    device(123)
                },
                (role == SessionRole::Both).then_some(71),
                0,
            );
            assert_eq!(
                current(&session).await,
                PropertyValue::ApplicationData(value(&next))
            );
            assert!(old_records.try_recv().is_err());
            assert!(new_records.try_recv().is_err());
            session.stop().await.unwrap();
        }
    }
    old.stop().await.unwrap();
    new.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn source_recipient_null_equal_invalid_and_second_permit_preserve_commit_state() {
    let (mut sink, mut records) = network().await;
    let mut db = database(false);
    db.set_clock_reader(None);
    let mut session = session(db, SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let status = session
        .database
        .as_ref()
        .unwrap()
        .read()
        .await
        .get(&selected())
        .unwrap()
        .audit_reporter_internal()
        .unwrap()
        .status_internal();
    let initial_epoch = status.auditing_failure_epoch();
    local(&session, PropertyValue::Null).await.unwrap();
    local(
        &session,
        PropertyValue::ApplicationData(value(&device(999))),
    )
    .await
    .unwrap();
    for invalid in [
        PropertyValue::Unsigned(7),
        PropertyValue::ApplicationData(vec![0]),
        PropertyValue::ApplicationData(value(&device(1000))),
    ] {
        assert!(local(&session, invalid).await.is_err());
    }
    let permits: Vec<_> = (0..63)
        .map(|_| {
            session
                .notifications
                .as_ref()
                .unwrap()
                .try_admit_audit()
                .unwrap()
        })
        .collect();
    let next = direct(&sink); // different value, same physical route still needs two permits
    assert!(
        local(&session, PropertyValue::ApplicationData(value(&next)))
            .await
            .is_err()
    );
    assert_eq!(
        current(&session).await,
        PropertyValue::ApplicationData(value(&device(999)))
    );
    assert_eq!(status.auditing_failure_epoch(), initial_epoch);
    assert_eq!(
        session
            .database
            .as_ref()
            .unwrap()
            .read()
            .await
            .reserve_event_sequence_number()
            .number(),
        0
    );
    assert_eq!(session.active_leases(), 0);
    assert!(records.try_recv().is_err());
    drop(permits);
    local(&session, PropertyValue::ApplicationData(value(&next)))
        .await
        .unwrap();
    let (a, _) = notification(&receive(&mut records).await, false);
    let (b, _) = notification(&receive(&mut records).await, false);
    assert_eq!(a, b);
    assert_record(&a, &device(999), &next, device(123), None, 0);
    assert_eq!(
        session
            .database
            .as_ref()
            .unwrap()
            .read()
            .await
            .reserve_event_sequence_number()
            .number(),
        1
    );
    session.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_recipient_read_keeps_admitted_route_and_later_read_uses_new_value() {
    let (mut old, mut old_records) = network().await;
    let (mut new, mut new_records) = network().await;
    let (mut peer, mut requests) = network().await;
    let mut db = database(false);
    db.set_clock_reader(None);
    let mut session = session(db, SessionRole::ClientOnly, &old);
    session.start().await.unwrap();
    let read = start_read(
        &session,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let request = receive(&mut requests).await;
    let (invoke, rp) = read_request(&request);
    local(
        &session,
        PropertyValue::ApplicationData(value(&direct(&new))),
    )
    .await
    .unwrap();
    let (change, _) = notification(&receive(&mut old_records).await, false);
    assert_eq!(change.operation, AuditOperation::WRITE);
    assert_eq!(
        notification(&receive(&mut new_records).await, false).0,
        change
    );
    send(&peer, &request.source_mac, ack(invoke, &rp)).await;
    read.await.unwrap().unwrap();
    let (record, _) = notification(&receive(&mut old_records).await, false);
    assert_eq!(record.operation, AuditOperation::READ);
    assert_eq!(
        record.source_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(0))
    );
    failures::complete_read(&session, &peer, &mut requests).await;
    assert_eq!(
        notification(&receive(&mut new_records).await, false)
            .0
            .operation,
        AuditOperation::READ
    );
    assert!(old_records.try_recv().is_err());
    session.stop().await.unwrap();
    old.stop().await.unwrap();
    new.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[path = "source_recipient_lifecycle_tests.rs"]
mod lifetime;

#[tokio::test]
async fn source_recipient_wire_denial_panic_and_invalid_encoding_are_atomic() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (mut sink, mut records) = network().await;
    let (mut peer, mut replies) = network().await;
    let decision = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let d = decision.clone();
    let c = calls.clone();
    let mut db = database(false);
    db.set_clock_reader(None);
    let mut session =
        session(db, SessionRole::Both, &sink).with_device_writes(Arc::new(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
            match d.load(Ordering::SeqCst) {
                0 => false,
                1 => panic!("controlled authorizer panic"),
                _ => true,
            }
        }));
    session.start().await.unwrap();
    let next = direct(&sink);
    for mode in [0, 1] {
        decision.store(mode, Ordering::SeqCst);
        assert!(matches!(
            wire(&session, &peer, &mut replies, value(&next), 72).await,
            Apdu::Error(_)
        ));
    }
    decision.store(2, Ordering::SeqCst);
    for bad in [vec![], vec![0, 0], vec![0x21, 7], {
        let mut b = value(&next);
        b.push(0);
        b
    }] {
        assert!(matches!(
            wire(&session, &peer, &mut replies, bad, 73).await,
            Apdu::Error(_)
        ));
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "malformed values never reach policy"
    );
    assert!(
        matches!(wire(&session,&peer,&mut replies,vec![0],74).await,Apdu::SimpleAck(a) if a.invoke_id==74)
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "NULL still requires policy"
    );
    assert_eq!(
        current(&session).await,
        PropertyValue::ApplicationData(value(&device(999)))
    );
    assert_eq!(
        session
            .database
            .as_ref()
            .unwrap()
            .read()
            .await
            .reserve_event_sequence_number()
            .number(),
        0
    );
    assert!(records.try_recv().is_err());
    assert_eq!(session.active_leases(), 0);
    session.stop().await.unwrap();
    sink.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn source_recipient_unresolved_device_preserves_read_and_rejects_live_change() {
    let (mut sink, mut records) = network().await;
    let (mut peer, mut requests) = network().await;
    let mut db = database(false);
    db.set_clock_reader(None);
    let mut session = session(db, SessionRole::ClientOnly, &sink);
    session.source_audit_bindings.clear();
    session.start().await.unwrap();
    assert_eq!(
        reliability(&session).await,
        Reliability::CONFIGURATION_ERROR.to_raw()
    );
    failures::complete_read(&session, &peer, &mut requests).await;
    assert!(records.try_recv().is_err());
    assert_eq!(
        session
            .database
            .as_ref()
            .unwrap()
            .read()
            .await
            .reserve_event_sequence_number()
            .number(),
        0
    );
    assert!(local(
        &session,
        PropertyValue::ApplicationData(value(&direct(&sink)))
    )
    .await
    .is_err());
    assert_eq!(
        current(&session).await,
        PropertyValue::ApplicationData(value(&device(999)))
    );
    session.stop().await.unwrap();
    sink.stop().await.unwrap();
    peer.stop().await.unwrap();
}
