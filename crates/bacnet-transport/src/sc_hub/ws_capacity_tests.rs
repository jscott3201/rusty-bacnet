use super::deadline_capacity_tests::CountedHub;
use super::deadline_test_support::*;
use super::ws_limits_test_support::*;
use super::*;

#[tokio::test]
async fn hub_relays_required_npdu_and_encoded_option_capacity() {
    let tls = TestTls::new();
    let hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut sender = tls.websocket(hub.address).await;
    let mut recipient = tls.websocket(hub.address).await;
    register(&mut sender, [0x22; 6]).await;
    register(&mut recipient, [0x32; 6]).await;
    for destination in [[0x32; 6], [0xff; 6]] {
        let message = npdu_message(destination, 1497, 4192);
        let wire = encoded(&message);
        assert_eq!(wire.len(), 5699); // 4 + destination(6) + encoded options(4192) + NPDU(1497)
        sender.send(Message::Binary(wire.clone())).await.unwrap();
        let relayed = receive(&mut recipient).await;
        let broadcast = destination == [0xff; 6];
        assert_eq!(relayed.len(), if broadcast { 5705 } else { 5699 });
        let decoded = decode_sc_message(&relayed).unwrap();
        assert_eq!(decoded.originating_vmac, Some([0x22; 6]));
        assert_eq!(
            decoded.destination_vmac,
            if broadcast { Some(destination) } else { None }
        );
        assert_eq!(decoded.dest_options, message.dest_options);
        assert_eq!(decoded.data_options, message.data_options);
        assert_eq!(decoded.payload, message.payload);
        assert_eq!(&relayed[if broadcast { 16 } else { 10 }..], &wire[10..]);
    }
    sender.close(None).await.unwrap();
    recipient.close(None).await.unwrap();
    until(|| hub.active.load(Ordering::Acquire) == 0).await;
}

#[tokio::test]
async fn hub_drops_npdu_above_own_capacity_before_relay() {
    let tls = TestTls::new();
    let hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut sender = tls.websocket(hub.address).await;
    let mut recipient = tls.websocket(hub.address).await;
    register(&mut sender, [0x22; 6]).await;
    register(&mut recipient, [0x32; 6]).await;
    let activity = hub
        .clients
        .lock()
        .await
        .get(&[0x32; 6])
        .unwrap()
        .last_activity
        .clone();
    let source_activity = hub
        .clients
        .lock()
        .await
        .get(&[0x22; 6])
        .unwrap()
        .last_activity
        .clone();
    activity.store(123, Ordering::Release);
    for destination in [[0x32; 6], [0xff; 6]] {
        let message = npdu_message(destination, 1498, 0);
        assert_eq!(encoded(&message).len(), 1508); // Below full-message cap
        source_activity.store(456, Ordering::Release);
        sender
            .send(Message::Binary(encoded(&message)))
            .await
            .unwrap();
        // Malformed HeartbeatRequest produces a NAK without counting as activity.
        sender
            .send(Message::Binary(vec![0x0a, 0, 0x87, 0x65, 1].into()))
            .await
            .unwrap();
        let barrier = decode_sc_message(&receive(&mut sender).await).unwrap();
        assert_eq!(
            (barrier.function, barrier.message_id),
            (ScFunction::Result, 0x8765)
        );
        assert_eq!(source_activity.load(Ordering::Acquire), 456);
        assert_eq!(activity.load(Ordering::Acquire), 123);
        assert_eq!(hub.clients.lock().await.len(), 2);
        heartbeat(&mut recipient, 0x1234).await; // any illegal relay would arrive first
        activity.store(123, Ordering::Release);
    }
    sender.close(None).await.unwrap();
    recipient.close(None).await.unwrap();
    until(|| hub.active.load(Ordering::Acquire) == 0).await;
}

#[tokio::test]
async fn node_and_hub_advertise_distinct_full_message_and_npdu_capacities() {
    use crate::port::TransportPort;
    let (mut server, node) = initiating_pair().await;
    let mut transport = crate::sc::ScTransport::new(node, [0x22; 6]);
    let accept = async {
        let Message::Binary(wire) = server.next().await.unwrap().unwrap() else {
            panic!("expected ConnectRequest")
        };
        assert_eq!(&wire[26..30], &[0x16, 0x49, 0x05, 0xc4]); // 5705 full BVLC, 1476 NPDU
        let mut reply = wire.to_vec();
        reply[0] = 7;
        reply[4..10].fill(0x10);
        reply[10..26].fill(0x10);
        reply[26..30].copy_from_slice(&[0x16, 0x49, 0x05, 0xd9]); // 5705, 1497
        server.send(Message::Binary(reply.into())).await.unwrap();
    };
    let ((), receive) = tokio::join!(accept, transport.start());
    let _receive = receive.unwrap();
    let connection = transport.connection().unwrap().lock().await;
    assert_eq!(
        (
            connection.hub_max_bvlc_length,
            connection.hub_max_apdu_length
        ),
        (5705, 1497)
    );
    drop(connection);
    transport.stop().await.unwrap();
    let tls = TestTls::new();
    let hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut peer = tls.websocket(hub.address).await;
    let accept = register(&mut peer, [0x22; 6]).await;
    assert_eq!(&accept.payload[22..26], &[0x16, 0x49, 0x05, 0xd9]);
    peer.close(None).await.unwrap();
    until(|| hub.active.load(Ordering::Acquire) == 0).await;
}

#[tokio::test]
async fn initiating_runtime_limits_remain_mutable_below_adapter_ceiling() {
    use crate::port::TransportPort;
    use std::time::Duration;
    let (mut server, node) = initiating_pair().await;
    let mut transport = crate::sc::ScTransport::new(node, [0x22; 6]);
    let accept = async {
        let Message::Binary(wire) = server.next().await.unwrap().unwrap() else {
            panic!("expected ConnectRequest")
        };
        let mut reply = wire.to_vec();
        reply[0] = 7;
        reply[4..26].fill(0x10);
        server.send(Message::Binary(reply.into())).await.unwrap();
    };
    let ((), receive) = tokio::join!(accept, transport.start());
    let mut receive = receive.unwrap();
    let mut large = npdu_message([0xff; 6], 61327, 4192);
    large.originating_vmac = Some([0x10; 6]);
    let mut small = npdu_message([0xff; 6], 10, 0);
    small.originating_vmac = Some([0x10; 6]);
    // Allow large NPDUs, retain lower full BVLC bound: the adapter can receive
    // 65535 bytes, but current protocol capacity must still discard the message.
    transport.connection().unwrap().lock().await.max_apdu_length = 61327;
    server.send(Message::Binary(encoded(&large))).await.unwrap();
    server.send(Message::Binary(encoded(&small))).await.unwrap();
    let first = tokio::time::timeout(Duration::from_secs(2), receive.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.npdu, small.payload);
    // Public mutation after startup remains effective up to the u16 ceiling.
    transport.connection().unwrap().lock().await.max_bvlc_length = 65535;
    server.send(Message::Binary(encoded(&large))).await.unwrap();
    let next = tokio::time::timeout(Duration::from_secs(2), receive.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.npdu, large.payload);
    assert_eq!(next.source_mac.as_slice(), &[0x10; 6]);
    // NPDU capacity is still independent of the newly raised BVLC capacity.
    transport.connection().unwrap().lock().await.max_apdu_length = 1476;
    server.send(Message::Binary(encoded(&large))).await.unwrap();
    server.send(Message::Binary(encoded(&small))).await.unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), receive.recv())
            .await
            .unwrap()
            .unwrap()
            .npdu,
        small.payload
    );
    transport.stop().await.unwrap();
}
