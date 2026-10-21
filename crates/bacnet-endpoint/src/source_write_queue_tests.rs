use super::super::queued::Gated;
use super::*;
use bacnet_transport::loopback::LoopbackTransport;
use tokio::sync::{Notify, Semaphore};
#[tokio::test]
async fn source_write_queued_egress_is_owned_before_caller_cancellation() {
    let source_mac = encode_bip_mac([127, 0, 0, 1], 30001);
    let peer_mac = encode_bip_mac([127, 0, 0, 1], 30002);
    let (transport, peer) = LoopbackTransport::pair(source_mac.to_vec(), peer_mac.to_vec());
    let entered = Arc::new(Notify::new());
    let gate = Arc::new(Semaphore::new(0));
    let mut session = EndpointSession::new(
        Gated {
            inner: transport,
            entered: Arc::clone(&entered),
            gate: Arc::clone(&gate),
        },
        SessionRole::ClientOnly,
        SessionConfig::default(),
    )
    .unwrap()
    .with_database(write_database(false))
    .with_source_audit_reporter(selected());
    session.source_audit_bindings.push((
        oid(ObjectType::DEVICE, 999),
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, 30002),
    ));
    Arc::get_mut(session.database.as_mut().unwrap())
        .unwrap()
        .get_mut()
        .get_mut(&oid(ObjectType::DEVICE, 123))
        .unwrap()
        .device_authority_internal()
        .unwrap()
        .provision_audit_recipient(BACnetRecipient::Device(oid(ObjectType::DEVICE, 999)))
        .unwrap();
    let mut peer = NetworkLayer::new(peer);
    let mut received = peer.start().await.unwrap();
    session.start().await.unwrap();
    // Hold an unrelated egress command in transport, leaving RP in the queue.
    let blocker = session
        .egress
        .as_ref()
        .unwrap()
        .admit_apdu(
            vec![0x10, 8],
            bacnet_endpoint_core::endpoint_ingress::EndpointApduDestination::Direct {
                destination_mac: peer_mac.into_iter().collect(),
            },
            false,
            NetworkPriority::Normal,
            Vec::new(),
            None,
        )
        .unwrap();
    timeout(WAIT, entered.notified()).await.unwrap();
    let client = session.cloned_client_handle().unwrap();
    let caller = tokio::spawn(async move {
        client
            .write_property(
                &peer_mac,
                WritePropertyRequest {
                    object_identifier: target(),
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    property_value: vec![0],
                    priority: None,
                },
                Commandability::Commandable,
            )
            .await
    });
    timeout(WAIT, async {
        while session.active_leases() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // Yield to the owned task's egress admission before cancelling its waiter.
    tokio::task::yield_now().await;
    caller.abort();
    let _ = caller.await;
    assert_eq!(
        session.active_leases(),
        1,
        "caller drop must not cancel owned request lease"
    );
    gate.add_permits(3); // unrelated send, RP, source notification
    assert!(blocker.complete().await.result.is_ok());
    let unrelated = receive(&mut received).await;
    assert!(matches!(
        decode_apdu(unrelated.apdu).unwrap(),
        Apdu::UnconfirmedRequest(_)
    ));
    let request = receive(&mut received).await;
    let (invoke, rp) = write_request(&request);
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, &write_ack(invoke, &rp)).unwrap();
    peer.send_apdu(&encoded, &source_mac, false, NetworkPriority::Normal)
        .await
        .unwrap();
    let (record, _) = notification(&receive(&mut received).await, false);
    assert_eq!(record.invoke_id, Some(invoke));
    assert_eq!(record.result, None);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn endpoint_write_unreported_cancel_retracts_queued_and_in_progress_sends() {
    for queued in [true, false] {
        let source_mac = encode_bip_mac([127, 0, 0, 1], 30001);
        let peer_mac = encode_bip_mac([127, 0, 0, 1], 30002);
        let (transport, peer) = LoopbackTransport::pair(source_mac.to_vec(), peer_mac.to_vec());
        let entered = Arc::new(Notify::new());
        let gate = Arc::new(Semaphore::new(0));
        let mut session = EndpointSession::new(
            Gated {
                inner: transport,
                entered: entered.clone(),
                gate: gate.clone(),
            },
            SessionRole::ClientOnly,
            SessionConfig::default(),
        )
        .unwrap();
        let mut peer = NetworkLayer::new(peer);
        let mut received = peer.start().await.unwrap();
        session.start().await.unwrap();
        let destination = bacnet_endpoint_core::endpoint_ingress::EndpointApduDestination::Direct {
            destination_mac: MacAddr::from_slice(&peer_mac),
        };
        if queued {
            // A genuine fire-and-forget marker survives dropping its completion.
            let blocker = session
                .egress
                .as_ref()
                .unwrap()
                .admit_apdu(
                    vec![0x10, 8],
                    destination.clone(),
                    false,
                    NetworkPriority::Normal,
                    Vec::new(),
                    None,
                )
                .unwrap();
            timeout(WAIT, entered.notified()).await.unwrap();
            drop(blocker);
        }
        let client = session.cloned_client_handle().unwrap();
        let task = tokio::spawn(async move {
            client
                .write_property(
                    &peer_mac,
                    WritePropertyRequest {
                        object_identifier: target(),
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                        property_value: vec![0],
                        priority: None,
                    },
                    Commandability::Noncommandable,
                )
                .await
        });
        timeout(WAIT, async {
            while session.active_leases() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        if !queued {
            timeout(WAIT, entered.notified()).await.unwrap();
        } else {
            tokio::task::yield_now().await;
        }
        task.abort();
        let _ = task.await;
        assert_eq!(session.active_leases(), 0);
        // Flush with an owned completion so absence is not a scheduling guess.
        let marker = session
            .egress
            .as_ref()
            .unwrap()
            .admit_apdu(
                vec![0x10, 8],
                destination,
                false,
                NetworkPriority::Normal,
                Vec::new(),
                None,
            )
            .unwrap();
        gate.add_permits(3);
        timeout(WAIT, marker.complete())
            .await
            .unwrap()
            .result
            .unwrap();
        let expected = if queued { 2 } else { 1 };
        for _ in 0..expected {
            let envelope = receive(&mut received).await;
            assert!(
                matches!(
                    decode_apdu(envelope.apdu).unwrap(),
                    Apdu::UnconfirmedRequest(_)
                ),
                "canceled WP must not reach peer"
            );
        }
        assert!(
            received.try_recv().is_err(),
            "only explicit markers may remain"
        );
        session.stop().await.unwrap();
        peer.stop().await.unwrap();
    }
}
