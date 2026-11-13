//! Real TLS accepted sockets with an independently retained registered listener.
use super::*;
use crate::port::DirectResponseScope;

#[tokio::test]
async fn direct_response_registered_transport_stop_abort_drop_seal_retained_listener() {
    for mode in 0..3 {
        let ca = TestCa::generate();
        let (ws, hub) = crate::sc::LoopbackWebSocket::pair();
        let config = DirectAcceptConfig::new(
            loopback_addr(),
            LISTENER_VMAC,
            LISTENER_UUID,
            ca.node_config(vec!["localhost".into()]),
        );
        let (mut transport, mut listener) = ScTransport::new(ws, LISTENER_VMAC)
            .with_device_uuid(LISTENER_UUID)
            .with_direct_listener(config)
            .await
            .unwrap();
        let (started, ()) = tokio::join!(transport.start(), hub_accept(&hub, [0x33; 6]));
        let mut rx = started.unwrap();
        let (peer, mut connection) = dial_and_handshake(
            &direct_url(&listener.local_addr()),
            ca.node_config(vec!["peer".into()]),
        )
        .await;
        let mut bytes = BytesMut::new();
        encode_sc_message(
            &mut bytes,
            &connection
                .build_direct_encapsulated_npdu(NPDU, &[])
                .unwrap(),
        );
        peer.send(&bytes).await.unwrap();
        let ingress = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let route = ingress.direct_response.unwrap();
        let scope = DirectResponseScope::default();
        route.send(NPDU, &scope).await.unwrap();
        let reply = decode_sc_message(&peer.recv().await.unwrap()).unwrap();
        assert_eq!(reply.function, ScFunction::EncapsulatedNpdu);
        assert_eq!(reply.payload.as_ref(), NPDU);
        let mut queued = Box::pin(route.send(&[1, 0, 0x44], &scope));
        assert!(futures_util::poll!(&mut queued).is_pending());
        // Current-thread scheduling: queue exists, but the sole writer has not
        // polled it. Every teardown seals before its first possible yield.
        match mode {
            0 => {
                transport.stop().await.unwrap();
                transport.stop().await.unwrap();
            }
            1 => {
                transport.abort();
                transport.abort();
            }
            _ => drop(transport),
        }
        assert!(*listener.shutdown_status().borrow());
        listener.stop().await; // retained handle joins the shared teardown
        assert_eq!(listener.active_connections(), 0);
        assert!(listener.membership.current_generations().is_empty());
        assert!(queued.await.is_err());
        assert!(route.send(NPDU, &scope).await.is_err());
        // The queued response cannot pass teardown: the peer reads only
        // the orderly goodbye (#2520).
        super::direct_close_tests::assert_goodbye(&peer).await;
        listener.stop().await;
    }
}

#[tokio::test]
async fn direct_response_real_tls_ping_pong_preserves_handshake_and_response() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener(&ca, |c| c).await;
    let peer = crate::sc_tls::TlsWebSocket::connect_direct(
        &direct_url(&listener.local_addr()),
        ca.node_config(vec!["peer".into()]),
    )
    .await
    .unwrap();
    let mut connection = ScConnection::new(DIAL_VMAC, DIAL_UUID);
    let mut bytes = BytesMut::new();
    encode_sc_message(&mut bytes, &connection.build_connect_request());
    for message in [
        Message::Ping(vec![1].into()),
        Message::Pong(vec![2].into()),
        Message::Binary(bytes.to_vec().into()),
    ] {
        peer.write.lock().await.send(message).await.unwrap();
    }
    // Actual tungstenite TLS frames establish that Ping handling still emits
    // Pong and that controls before Connect do not become handshake data.
    let mut got_pong = false;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match peer.read.lock().await.next().await.unwrap().unwrap() {
                Message::Pong(data) => {
                    assert_eq!(data.as_ref(), &[1]);
                    got_pong = true;
                }
                Message::Binary(data) => {
                    assert_eq!(
                        decode_sc_message(&data).unwrap().function,
                        ScFunction::ConnectAccept
                    );
                    break;
                }
                other => panic!("unexpected handshake frame: {other:?}"),
            }
        }
    })
    .await
    .unwrap();
    assert!(got_pong);
    bytes.clear();
    encode_sc_message(
        &mut bytes,
        &connection
            .build_direct_encapsulated_npdu(NPDU, &[])
            .unwrap(),
    );
    for message in [
        Message::Ping(vec![3].into()),
        Message::Pong(vec![4].into()),
        Message::Binary(bytes.to_vec().into()),
    ] {
        peer.write.lock().await.send(message).await.unwrap();
    }
    let received = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.npdu.as_ref(), NPDU);
    let scope = DirectResponseScope::default();
    received
        .direct_response
        .unwrap()
        .send(NPDU, &scope)
        .await
        .unwrap();
    let mut got_pong = false;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match peer.read.lock().await.next().await.unwrap().unwrap() {
                Message::Pong(data) => {
                    assert_eq!(data.as_ref(), &[3]);
                    got_pong = true;
                }
                Message::Binary(data) => {
                    let response = decode_sc_message(&data).unwrap();
                    assert_eq!(response.function, ScFunction::EncapsulatedNpdu);
                    assert_eq!(response.payload.as_ref(), NPDU);
                    break;
                }
                other => panic!("unexpected response frame: {other:?}"),
            }
        }
    })
    .await
    .unwrap();
    assert!(got_pong);
    listener.stop().await;
}
