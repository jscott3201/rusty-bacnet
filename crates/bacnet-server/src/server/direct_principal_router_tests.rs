//! The unicast path (#518): a control that reaches the router over a direct
//! BACnet/SC connection carries its sender's principal, so a hardened router
//! can take it only from configured router certificates. Every SC envelope
//! comes from a real direct TLS connection; the router's ports are queue-fed
//! test transports. Broadcast announcements, which SC relays through a hub
//! and so carry no principal, are in `sc_router_hub_tests`.
use super::*;
use bacnet_network::router::control_policy::{ControlAuthorizer, ControlPolicy};
use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort};
use bacnet_transport::port::ScPrincipal;

/// I-Am-Router-To-Network for one network.
fn i_am_router(network: u16) -> Vec<u8> {
    let [high, low] = network.to_be_bytes();
    vec![0x01, 0x80, 0x01, high, low]
}

/// Send `npdu` over `peer`'s direct connection; return what the listener
/// admitted.
async fn capture_npdu(f: &mut Fixture, peer: &mut Peer, npdu: &[u8]) -> ReceivedNpdu {
    let mut wire = BytesMut::new();
    encode_sc_message(
        &mut wire,
        &peer
            .connection
            .build_direct_encapsulated_npdu(npdu, &[])
            .unwrap(),
    );
    peer.ws.send(&wire).await.unwrap();
    bounded(f.admitted.recv()).await.unwrap()
}

fn port(network_number: u16, inbound: mpsc::Receiver<ReceivedNpdu>) -> RouterPort<TestTransport> {
    RouterPort {
        transport: TestTransport::builder()
            .local_mac(&[0xaa; 6])
            .inbound(inbound)
            .build(),
        network_number,
    }
}

#[tokio::test]
async fn a_direct_unicast_control_is_taken_only_from_configured_router_principals() {
    let ca = TestCa::new();
    let mut f = Fixture::new(
        &ca,
        ServerConfig::default(),
        database(Arc::new(AtomicUsize::new(0))),
    )
    .await;
    let routers = Arc::new(StdMutex::new(Vec::<ScPrincipal>::new()));
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let (allowed, record) = (routers.clone(), seen.clone());
    let authorizer: ControlAuthorizer = Arc::new(move |context| {
        record
            .lock()
            .unwrap()
            .push(context.sending_peer_principal());
        context
            .sending_peer_principal()
            .is_some_and(|peer| allowed.lock().unwrap().contains(&peer))
    });
    let (sc_tx, sc_rx) = mpsc::channel(32);
    let (_other_tx, other_rx) = mpsc::channel(32);
    let mut router = BACnetRouter::start(
        vec![port(10, sc_rx), port(20, other_rx)],
        RouterOptions::new()
            .control_policy(ControlPolicy::Hardened)
            .control_authorizer(authorizer),
    )
    .await
    .unwrap()
    .router;
    // A configured router's advertisement is learned.
    let mut trusted = f.peer(ca.tls("router")).await;
    let from_router = capture_npdu(&mut f, &mut trusted, &i_am_router(300)).await;
    let router_principal = from_router.provenance.principal().unwrap();
    routers.lock().unwrap().push(router_principal);
    sc_tx.send(from_router).await.unwrap();
    bounded(async {
        loop {
            let found = router.table().lookup(300).is_some();
            if found {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;

    // Another CA-valid node, claiming the same VMAC, is refused.
    let mut node = f.peer(ca.tls("node")).await;
    let from_node = capture_npdu(&mut f, &mut node, &i_am_router(400)).await;
    let node_principal = from_node.provenance.principal().unwrap();
    sc_tx.send(from_node).await.unwrap();
    // A sender with no principal (unverified, as a hub relay or B/IP is) too.
    sc_tx
        .send(ReceivedNpdu::unverified(
            Bytes::from(i_am_router(500)),
            MacAddr::from_slice(&PEER_MAC),
            false,
            Vec::new(),
            None,
        ))
        .await
        .unwrap();
    bounded(async {
        while seen.lock().unwrap().len() < 3 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    {
        let table = router.table();
        assert!(table.lookup(400).is_none());
        assert!(table.lookup(500).is_none());
        assert!(table.lookup(300).is_some());
    }
    assert_eq!(
        *seen.lock().unwrap(),
        [Some(router_principal), Some(node_principal), None]
    );
    let counted = router.control_snapshot().i_am;
    assert_eq!((counted.allow_total, counted.deny_total), (1, 2));
    router.stop().await;
    f.stop().await;
}
