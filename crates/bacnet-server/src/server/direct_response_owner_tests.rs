//! Network/server lifetime and router propagation over real TLS ingress.
use super::*;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_network::layer::IssuedApdu;
use bacnet_network::response_route::ResponseRoute;

fn idle_network() -> NetworkLayer<TestTransport> {
    let (_, incoming) = mpsc::channel(1);
    let (responses, _) = mpsc::unbounded_channel();
    NetworkLayer::new(queued_port(incoming, responses))
}
async fn issue(
    net: &NetworkLayer<TestTransport>,
    route: &ResponseRoute,
    invoke: u8,
) -> Result<(), Error> {
    net.send_response_apdu_on_issuance(
        IssuedApdu {
            apdu: &[0x20, invoke, 15],
            next_hop: &PEER_MAC,
            destination: None,
            expecting_reply: false,
            priority: NetworkPriority::Normal,
        },
        route,
        || {},
    )
    .await
}

#[tokio::test]
async fn direct_response_network_stop_and_server_drop_seal_retained_routes() {
    let ca = TestCa::new();
    let mut f = Fixture::new(
        &ca,
        ServerConfig::default(),
        database(Arc::new(AtomicUsize::new(0))),
    )
    .await;
    let mut a = f.peer(ca.tls("a")).await;
    let envelope = f.capture(&mut a, &read_name(11)).await;
    let route = ResponseRoute::new(envelope.provenance, envelope.direct_response.clone());
    let mut net = idle_network();
    issue(&net, &route, 11).await.unwrap();
    assert!(matches!(f.response().await, Apdu::SimpleAck(ack) if ack.invoke_id == 11));
    net.stop().await.unwrap();
    net.stop().await.unwrap();
    assert!(issue(&net, &route, 12).await.is_err());
    // Keep the network allocation alive deliberately: Drop must seal before
    // asynchronous request cancellation or the final Arc release.
    let network = f.server.network.as_ref().unwrap().clone();
    let mut queued = Box::pin(issue(&network, &route, 13));
    assert!(futures_util::poll!(&mut queued).is_pending());
    drop(f.server);
    assert!(queued.await.is_err());
    assert!(issue(&network, &route, 14).await.is_err());
    // The standalone listener remains independently owned; stop supplies EOF
    // as a deterministic boundary proving no queued application frame escaped.
    f.listener.stop().await;
    assert!(a.ws.recv().await.is_err());
    assert!(f.responses.try_recv().is_err());
}

#[tokio::test]
async fn direct_response_server_stop_cancels_held_request_and_keeps_listener_handle_explicit() {
    let ca = TestCa::new();
    let mut f = Fixture::new(
        &ca,
        ServerConfig::default(),
        database(Arc::new(AtomicUsize::new(0))),
    )
    .await;
    let mut a = f.peer(ca.tls("a")).await;
    let live = f.capture(&mut a, &read_name(20)).await;
    f.feed(live).await;
    assert_name(socket_response(&mut f, &a).await, 20);
    f.active(0).await;
    let db = f.server.db.clone();
    let held = db.write().await;
    let waiting = f.capture(&mut a, &read_name(21)).await;
    f.feed(waiting).await;
    f.active(1).await;
    bounded(f.server.stop()).await.unwrap();
    f.server.stop().await.unwrap();
    drop(held);
    assert_eq!(f.server.request_tasks.counters().confirmed_active, 0);
    assert!(f.server.seg_ack_senders.lock().is_empty());
    f.listener.stop().await;
    assert!(a.ws.recv().await.is_err());
    assert!(f.responses.try_recv().is_err());
}

#[tokio::test]
async fn direct_response_router_local_delivery_preserves_matching_capability() {
    use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
    let ca = TestCa::new();
    let mut f = Fixture::new(
        &ca,
        ServerConfig::default(),
        database(Arc::new(AtomicUsize::new(0))),
    )
    .await;
    let mut a = f.peer(ca.tls("a")).await;
    let (tx, incoming) = mpsc::channel(8);
    let (responses, mut generic) = mpsc::unbounded_channel();
    let port = RouterPort {
        transport: queued_port(incoming, responses),
        network_number: 44,
    };
    let (_tx_b, incoming_b) = mpsc::channel(1);
    let (responses_b, mut generic_b) = mpsc::unbounded_channel();
    let port_b = RouterPort {
        transport: queued_port(incoming_b, responses_b),
        network_number: 55,
    };
    let StartedRouter {
        mut router,
        apdus: mut local,
        ..
    } = BACnetRouter::start(vec![port, port_b], RouterOptions::new())
        .await
        .unwrap();
    let mut saved = None;
    for (invoke, destination) in [
        (1, None),
        (
            2,
            Some(NpduAddress {
                network: 55,
                mac_address: MacAddr::from_slice(&[0xaa; 6]),
            }),
        ),
    ] {
        let mut envelope = f.capture(&mut a, &read_name(invoke)).await;
        let expected = envelope.provenance.clone();
        let mut npdu = decode_npdu(envelope.npdu).unwrap();
        npdu.destination = destination.clone();
        let mut bytes = BytesMut::new();
        encode_npdu(&mut bytes, &npdu).unwrap();
        envelope.npdu = bytes.freeze();
        tx.send(envelope).await.unwrap();
        let received = bounded(local.recv()).await.unwrap();
        assert_eq!(received.provenance, expected);
        assert_eq!(
            received.direct_response.as_ref().unwrap().identity(),
            expected.direct_sc_identity().unwrap()
        );
        let route = received.response_route();
        let header = if destination.is_some() { 16 } else { 7 };
        let npdu_limit = received
            .direct_response
            .as_ref()
            .unwrap()
            .max_npdu_length()
            .min(1497);
        assert_eq!(
            route.max_apdu_length(u16::MAX, None).unwrap() as usize,
            npdu_limit - header
        );
        let mut missing = received.clone();
        missing.direct_response = None;
        assert!(
            issue(
                f.server.network.as_ref().unwrap(),
                &missing.response_route(),
                99
            )
            .await
            .is_err()
        );
        let retired_network = idle_network();
        retired_network.seal_responses();
        assert!(issue(&retired_network, &route, 98).await.is_err());
        issue(f.server.network.as_ref().unwrap(), &route, invoke)
            .await
            .unwrap();
        let wire = bounded(a.ws.recv()).await.unwrap();
        let frame = decode_sc_message(&wire).unwrap();
        let npdu = decode_npdu(frame.payload).unwrap();
        assert_eq!(npdu.source, destination);
        assert_eq!(
            npdu.destination,
            Some(NpduAddress {
                network: 123,
                mac_address: MacAddr::from_slice(&[3])
            })
        );
        assert!(
            matches!(decode_apdu(npdu.payload).unwrap(), Apdu::SimpleAck(ack) if ack.invoke_id == invoke)
        );
        saved = Some(route);
    }
    assert!(generic.try_recv().is_err());
    assert!(generic_b.try_recv().is_err());
    router.stop().await.unwrap();
    assert!(
        issue(f.server.network.as_ref().unwrap(), &saved.unwrap(), 97)
            .await
            .is_err()
    );
    f.listener.stop().await;
    assert!(
        a.ws.recv().await.is_err(),
        "router retirement cannot leak a direct response"
    );
    f.stop().await;
}
