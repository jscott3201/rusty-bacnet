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
    session.static_source_audit_recipient = Some(crate::bip::StaticSourceAuditRecipient {
        device: oid(ObjectType::DEVICE, 999),
        address: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 30002),
    });
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
