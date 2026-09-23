use super::*;
use bacnet_encoding::apdu::RejectPdu;
use bacnet_types::enums::RejectReason;

fn recipient(fixture: &Fixture) -> BACnetRecipient {
    BACnetRecipient::Address(BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(fixture.peer.local_mac()),
    })
}
async fn change(fixture: &Fixture, recipient: &BACnetRecipient) -> Result<(), Error> {
    let mut bytes = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_recipient(&mut bytes, recipient);
    fixture
        .source
        .db
        .write()
        .await
        .get_mut(&oid(ObjectType::DEVICE, 123))
        .unwrap()
        .write_property(
            PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
            None,
            PropertyValue::ApplicationData(bytes.to_vec()),
            None,
        )
}
async fn record(fixture: &mut Fixture) -> u8 {
    let envelope = timeout(WAIT, fixture.records.recv())
        .await
        .unwrap()
        .unwrap();
    let Apdu::ConfirmedRequest(pdu) = decode_apdu(envelope.apdu).unwrap() else {
        panic!("confirmed Audit")
    };
    assert_eq!(
        pdu.service_choice,
        ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION
    );
    let notification = AuditNotificationRequest::decode(&pdu.service_request).unwrap();
    assert_eq!(
        notification.notifications[0].operation,
        AuditOperation::WRITE
    );
    pdu.invoke_id
}
fn finish(fixture: &Fixture, id: u8, success: bool) {
    let apdu = if success {
        Apdu::SimpleAck(SimpleAck {
            invoke_id: id,
            service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
        })
    } else {
        Apdu::Reject(RejectPdu {
            invoke_id: id,
            reject_reason: RejectReason::OTHER,
        })
    };
    assert!(fixture
        .owner
        .admit_terminal(fixture.peer.local_mac(), None, &apdu));
}
async fn health(fixture: &Fixture) -> PropertyValue {
    fixture
        .source
        .db
        .read()
        .await
        .get(&fixture.source.selected)
        .unwrap()
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap()
}

#[tokio::test]
async fn source_recipient_second_lease_size_and_close_fail_before_commit() {
    for case in ["second-lease", "size", "closed"] {
        let fixture = Fixture::with_max(true, if case == "size" { 50 } else { 1476 }).await;
        fixture.source.db.write().await.set_clock_reader(None);
        let before = fixture.status.begin_delivery();
        let leases: Vec<_> = if case == "second-lease" {
            (0..255)
                .map(|_| {
                    fixture
                        .coordinator
                        .reserve(LeaseMetadata::requester(
                            CanonicalPeer::direct(&[3]),
                            ConfirmedServiceChoice::READ_PROPERTY,
                            TerminalPolicy::ComplexAck,
                        ))
                        .unwrap()
                })
                .collect()
        } else {
            vec![]
        };
        if case == "closed" {
            fixture.owner.close();
        }
        assert!(
            change(&fixture, &recipient(&fixture)).await.is_err(),
            "{case}"
        );
        assert_eq!(fixture.status.begin_delivery(), before, "{case}");
        assert_eq!(
            fixture
                .source
                .db
                .read()
                .await
                .reserve_event_sequence_number()
                .number(),
            0,
            "{case}"
        );
        assert_eq!(
            fixture.coordinator.active_count().unwrap(),
            leases.len(),
            "{case}"
        );
        for lease in leases {
            fixture.coordinator.release(lease).unwrap();
        }
        fixture.stop().await;
    }
}

#[tokio::test]
async fn source_recipient_pair_completion_orders_and_aba_fence() {
    for failure_first in [true, false] {
        let mut fixture = Fixture::new(true).await;
        change(&fixture, &recipient(&fixture)).await.unwrap();
        let first = record(&mut fixture).await;
        let second = record(&mut fixture).await;
        assert_ne!(first, second);
        finish(&fixture, first, !failure_first);
        tokio::task::yield_now().await;
        finish(&fixture, second, failure_first);
        timeout(WAIT, fixture.owner.join_next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            health(&fixture).await,
            PropertyValue::Enumerated(Reliability::COMMUNICATION_FAILURE.to_raw())
        );
        // A -> B -> A is a new generation even though the final value compares equal to A.
        change(
            &fixture,
            &BACnetRecipient::Device(oid(ObjectType::DEVICE, 999)),
        )
        .await
        .unwrap();
        let stale1 = record(&mut fixture).await;
        let stale2 = record(&mut fixture).await;
        change(&fixture, &recipient(&fixture)).await.unwrap();
        let current1 = record(&mut fixture).await;
        let current2 = record(&mut fixture).await;
        finish(&fixture, current1, true);
        finish(&fixture, current2, true);
        timeout(WAIT, fixture.owner.join_next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        finish(&fixture, stale1, false);
        finish(&fixture, stale2, false);
        timeout(WAIT, fixture.owner.join_next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            health(&fixture).await,
            PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
        );
        fixture.stop().await;
    }
}

#[tokio::test(start_paused = true)]
async fn source_recipient_closed_egress_is_postcommit_and_does_not_retry_or_rollback() {
    let mut fixture = Fixture::new(true).await;
    fixture.source.db.write().await.set_clock_reader(None);
    fixture.ingress.stop().await.unwrap();
    let next = recipient(&fixture);
    change(&fixture, &next).await.unwrap();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(3)).await;
    timeout(WAIT, fixture.owner.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        health(&fixture).await,
        PropertyValue::Enumerated(Reliability::COMMUNICATION_FAILURE.to_raw())
    );
    assert_eq!(
        fixture
            .source
            .db
            .read()
            .await
            .reserve_event_sequence_number()
            .number(),
        1
    );
    assert_eq!(fixture.coordinator.active_count().unwrap(), 0);
    assert!(fixture.records.try_recv().is_err());
    // Equality succeeds without retry despite the closed egress.
    change(&fixture, &next).await.unwrap();
    assert!(fixture.records.try_recv().is_err());
    fixture.source.close();
    fixture.owner.close();
    while fixture.owner.join_next().await.is_some() {}
    fixture.peer.stop().await.unwrap();
}

#[tokio::test]
async fn source_recipient_change_retires_pending_summary_without_new_read() {
    let mut fixture = Fixture::new(true).await;
    let permits: Vec<_> = (0..64)
        .map(|_| fixture.owner.try_admit_audit().unwrap())
        .collect();
    fixture.record(BACnetTimeStamp::SequenceNumber(7), true, false, false);
    tokio::task::yield_now().await;
    // The waiting summary owns the first released permit. Free two more for
    // the pair; its old generation is retired before it can enter the DB.
    let mut permits = permits;
    drop(permits.pop());
    drop(permits.pop());
    drop(permits.pop());
    change(&fixture, &recipient(&fixture)).await.unwrap();
    let first = record(&mut fixture).await;
    let second = record(&mut fixture).await;
    finish(&fixture, first, true);
    finish(&fixture, second, true);
    drop(permits);
    for _ in 0..2 {
        timeout(WAIT, fixture.owner.join_next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
    assert!(fixture.records.try_recv().is_err());
    fixture.stop().await;
}
