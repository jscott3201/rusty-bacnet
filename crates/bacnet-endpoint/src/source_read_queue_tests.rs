use super::*;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::ReceivedNpdu;
use tokio::sync::{Notify, Semaphore};

struct Gated {
    inner: LoopbackTransport,
    entered: Arc<Notify>,
    gate: Arc<Semaphore>,
}
impl TransportPort for Gated {
    fn bip_broadcast_endpoint(&self) -> Option<SocketAddrV4> {
        Some("255.255.255.255:30001".parse().unwrap())
    }
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.inner.start().await
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.inner.stop().await
    }
    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.entered.notify_one();
        self.gate.acquire().await.unwrap().forget();
        self.inner.send_unicast(npdu, mac).await
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.inner.send_broadcast(npdu).await
    }
    fn local_mac(&self) -> &[u8] {
        self.inner.local_mac()
    }
}

#[tokio::test]
async fn source_read_queued_egress_is_owned_before_caller_cancellation() {
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
    .with_database(database(false))
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
            NetworkPriority::NORMAL,
            Vec::new(),
            None,
        )
        .unwrap();
    timeout(WAIT, entered.notified()).await.unwrap();
    let client = session.cloned_client_handle().unwrap();
    let caller = tokio::spawn(async move {
        client
            .read_property(&peer_mac, target(), PropertyIdentifier::PRESENT_VALUE, None)
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
    let (invoke, rp) = read_request(&request);
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, &ack(invoke, &rp)).unwrap();
    peer.send_apdu(&encoded, &source_mac, false, NetworkPriority::NORMAL)
        .await
        .unwrap();
    let (record, _) = notification(&receive(&mut received).await, false);
    assert_eq!(record.invoke_id, Some(invoke));
    assert_eq!(record.result, None);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn source_recipient_full_egress_fails_one_sibling_after_commit_without_retry() {
    let source_mac = encode_bip_mac([127, 0, 0, 1], 30001);
    let peer_mac = encode_bip_mac([127, 0, 0, 1], 30002);
    let (transport, remote) = LoopbackTransport::pair(source_mac.to_vec(), peer_mac.to_vec());
    let entered = Arc::new(Notify::new());
    let gate = Arc::new(Semaphore::new(0));
    let mut db = database(false);
    db.set_clock_reader(None);
    db.get_mut(&oid(ObjectType::DEVICE, 123))
        .unwrap()
        .device_authority_internal()
        .unwrap()
        .provision_audit_recipient(BACnetRecipient::Device(oid(ObjectType::DEVICE, 999)))
        .unwrap();
    let mut session = EndpointSession::new(
        Gated {
            inner: transport,
            entered: entered.clone(),
            gate: gate.clone(),
        },
        SessionRole::ClientOnly,
        SessionConfig {
            queue_capacity: 1,
            ..Default::default()
        },
    )
    .unwrap()
    .with_database(db)
    .with_source_audit_reporter(selected());
    session.source_audit_bindings.push((
        oid(ObjectType::DEVICE, 999),
        "127.0.0.1:30002".parse().unwrap(),
    ));
    let mut peer = NetworkLayer::new(remote);
    let mut received = peer.start().await.unwrap();
    session.start().await.unwrap();
    let blocker = session
        .egress
        .as_ref()
        .unwrap()
        .admit_apdu(
            vec![0x10, 8],
            bacnet_endpoint_core::endpoint_ingress::EndpointApduDestination::Direct {
                destination_mac: MacAddr::from_slice(&peer_mac),
            },
            false,
            NetworkPriority::NORMAL,
            Vec::new(),
            None,
        )
        .unwrap();
    timeout(WAIT, entered.notified()).await.unwrap();
    let next = BACnetRecipient::Address(bacnet_types::constructed::BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(&peer_mac),
    });
    let mut bytes = BytesMut::new();
    bacnet_encoding::constructed::encode_recipient(&mut bytes, &next);
    {
        let mut db = session.database.as_ref().unwrap().write().await;
        db.get_mut(&oid(ObjectType::DEVICE, 123))
            .unwrap()
            .write_property(
                PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                None,
                PropertyValue::ApplicationData(bytes.to_vec()),
                None,
            )
            .unwrap();
        assert_eq!(db.reserve_event_sequence_number().number(), 1);
    }
    // The first postcommit attempt queues; the second encounters the bounded
    // queue while the unrelated transport operation is deliberately held.
    timeout(WAIT, async {
        loop {
            let health = session
                .database
                .as_ref()
                .unwrap()
                .read()
                .await
                .get(&selected())
                .unwrap()
                .read_property(PropertyIdentifier::RELIABILITY, None)
                .unwrap();
            if health == PropertyValue::Enumerated(Reliability::COMMUNICATION_FAILURE.to_raw()) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    gate.add_permits(2);
    blocker.complete().await.result.unwrap();
    let probe = receive(&mut received).await;
    assert!(matches!(
        decode_apdu(probe.apdu).unwrap(),
        Apdu::UnconfirmedRequest(_)
    ));
    let (record, _) = notification(&receive(&mut received).await, false);
    assert_eq!(record.operation, AuditOperation::WRITE);
    assert_eq!(record.target_value, Some(bytes.to_vec()));
    tokio::task::yield_now().await;
    let db = session.database.as_ref().unwrap().read().await;
    assert_eq!(
        db.get(&oid(ObjectType::DEVICE, 123))
            .unwrap()
            .read_property(PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT, None)
            .unwrap(),
        PropertyValue::ApplicationData(bytes.to_vec())
    );
    assert_eq!(
        db.get(&selected())
            .unwrap()
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::COMMUNICATION_FAILURE.to_raw())
    );
    drop(db);
    assert!(received.try_recv().is_err());
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
}
