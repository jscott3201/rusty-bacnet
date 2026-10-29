//! Built-in outbound evidence, resumption, replacement and retirement.
use super::*;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn outbound_verified_leaf_full_and_resumed_tls_have_new_incarnations() {
    let ca = TestCa::generate();
    let (chain, key) = ca.issue(vec!["localhost".into()]);
    let expected: [u8; 32] =
        aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, chain[0].as_ref())
            .as_ref()
            .try_into()
            .unwrap();
    let server = ScNodeTlsConfig::from_der(
        vec![ca.ca.clone()],
        chain,
        PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind(loopback_addr())
        .await
        .unwrap();
    let uri = direct_url(&listener.local_addr().unwrap());
    let (command, mut commands) = tokio::sync::mpsc::channel::<()>(1);
    let (events, mut kinds) = tokio::sync::mpsc::channel(2);
    let task = tokio::spawn(async move {
        for _ in 0..2 {
            let (tcp, _) = listener.accept().await.unwrap();
            let tls = server.acceptor().accept(tcp).await.unwrap();
            let kind = tls.get_ref().1.handshake_kind().unwrap();
            let mut ws = tokio_tungstenite::accept_hdr_async(
                tls,
                super::super::super::direct_subprotocol_response,
            )
            .await
            .unwrap();
            let Message::Binary(bytes) = ws.next().await.unwrap().unwrap() else {
                panic!("Connect expected")
            };
            let request = decode_sc_message(&bytes).unwrap();
            let config = DirectAcceptConfig::new(
                loopback_addr(),
                LISTENER_VMAC,
                LISTENER_UUID,
                server.clone(),
            );
            let mut bytes = BytesMut::new();
            encode_sc_message(
                &mut bytes,
                &super::super::super::build_connect_accept(request.message_id, &config),
            );
            ws.send(Message::Binary(bytes.to_vec().into()))
                .await
                .unwrap();
            let Message::Binary(request) = ws.next().await.unwrap().unwrap() else {
                panic!("NPDU expected")
            };
            assert_eq!(decode_sc_message(&request).unwrap().payload.as_ref(), NPDU);
            // Tickets are processed before the next connect. Ping/Pong are real
            // frame events followed by a binary NPDU which reaches transport.
            ws.send(Message::Ping(vec![7].into())).await.unwrap();
            ws.send(Message::Pong(vec![8].into())).await.unwrap();
            ws.send(Message::Binary(request)).await.unwrap();
            events.send(kind).await.unwrap();
            commands.recv().await.unwrap();
            ws.send(Message::Close(None)).await.unwrap();
        }
    });
    let (mut transport, mut rx, hub) = start_outbound(&ca).await;
    discover_send(&transport, &hub, &uri).await;
    let mut identities = Vec::new();
    for index in 0..2 {
        if index == 1 {
            transport.send_unicast(NPDU, &LISTENER_VMAC).await.unwrap();
        }
        let incoming = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let identity = incoming.provenance.direct_sc_identity().unwrap();
        assert_eq!(identity.leaf_sha256(), expected);
        // The dialled peer's verified leaf rides along, after a resumed
        // handshake too, and is the certificate the identity fingerprints.
        let leaf = incoming.provenance.leaf_certificate().unwrap();
        assert_eq!(
            aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, leaf.der()).as_ref(),
            expected
        );
        assert_eq!(
            incoming.direct_response.as_ref().unwrap().identity(),
            identity
        );
        assert_eq!(format!("{identity:?}"), "DirectScIdentity { .. }");
        identities.push(identity);
        assert_eq!(
            kinds.recv().await.unwrap(),
            if index == 0 {
                rustls::HandshakeKind::Full
            } else {
                rustls::HandshakeKind::Resumed
            }
        );
        command.send(()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while transport.direct_route_for_test(&LISTENER_VMAC).is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            incoming
                .direct_response
                .unwrap()
                .send(NPDU, &DirectResponseScope::default())
                .await
                .is_err()
        );
    }
    assert_ne!(identities[0].incarnation(), identities[1].incarnation());
    task.await.unwrap();
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn outbound_admitted_identity_survives_accepted_replacement_but_reply_never_retargets() {
    let ca = TestCa::generate();
    let (mut remote, mut remote_rx) = start_listener(&ca, |c| c).await;
    let (ws, hub) = LoopbackWebSocket::pair();
    let config = DirectAcceptConfig::new(
        loopback_addr(),
        DIAL_VMAC,
        DIAL_UUID,
        ca.node_config(vec!["localhost".into()]),
    );
    let (mut transport, mut local) = ScTransport::new(ws, DIAL_VMAC)
        .with_device_uuid(DIAL_UUID)
        .with_direct_tls(ca.node_config(vec!["caller".into()]))
        .with_direct_listener(config)
        .await
        .unwrap();
    let (started, ()) = tokio::join!(transport.start(), hub_accept(&hub, [0x33; 6]));
    let mut rx = started.unwrap();
    discover_send(&transport, &hub, &direct_url(&remote.local_addr())).await;
    let remote_request = remote_rx.recv().await.unwrap();
    remote_request
        .direct_response
        .unwrap()
        .send(&[1, 0, 0x41], &DirectResponseScope::default())
        .await
        .unwrap();
    let admitted_a = rx.recv().await.unwrap();
    let a_identity = admitted_a.provenance.direct_sc_identity().unwrap();
    // B claims exactly A's UUID/VMAC under a different same-CA leaf.
    let b = TlsWebSocket::connect_direct(
        &direct_url(&local.local_addr()),
        ca.node_config(vec!["B".into()]),
    )
    .await
    .unwrap();
    let mut connection = ScConnection::new(LISTENER_VMAC, LISTENER_UUID);
    let mut wire = BytesMut::new();
    encode_sc_message(&mut wire, &connection.build_connect_request());
    b.send(&wire).await.unwrap();
    assert_eq!(
        decode_sc_message(&b.recv().await.unwrap())
            .unwrap()
            .function,
        ScFunction::ConnectAccept
    );
    wire.clear();
    encode_sc_message(
        &mut wire,
        &connection
            .build_direct_encapsulated_npdu(&[1, 0, 0x42], &[])
            .unwrap(),
    );
    b.send(&wire).await.unwrap();
    let admitted_b = rx.recv().await.unwrap();
    let b_identity = admitted_b.provenance.direct_sc_identity().unwrap();
    assert_ne!(a_identity.leaf_sha256(), b_identity.leaf_sha256());
    assert_ne!(a_identity.incarnation(), b_identity.incarnation());
    assert_eq!(admitted_a.provenance.direct_sc_identity(), Some(a_identity));
    assert!(
        admitted_a
            .direct_response
            .unwrap()
            .send(&[1, 0, 0x43], &DirectResponseScope::default())
            .await
            .is_err()
    );
    admitted_b
        .direct_response
        .unwrap()
        .send(&[1, 0, 0x44], &DirectResponseScope::default())
        .await
        .unwrap();
    assert_eq!(
        decode_sc_message(&b.recv().await.unwrap())
            .unwrap()
            .payload
            .as_ref(),
        &[1, 0, 0x44]
    );
    assert!(futures_util::poll!(Box::pin(hub.recv()).as_mut()).is_pending());
    // Discovery disable retires outbound owners, not the accepted B route.
    transport = transport.with_direct_discovery(false);
    transport
        .send_unicast(&[1, 0, 0x45], &LISTENER_VMAC)
        .await
        .unwrap();
    assert_eq!(
        decode_sc_message(&b.recv().await.unwrap())
            .unwrap()
            .payload
            .as_ref(),
        &[1, 0, 0x45]
    );
    transport.stop().await.unwrap();
    local.stop().await;
    remote.stop().await;
}

#[tokio::test]
async fn outbound_stop_abort_drop_and_discovery_disable_seal_queued_original_replies() {
    for mode in 0..4 {
        let ca = TestCa::generate();
        let (mut remote, mut remote_rx) = start_listener(&ca, |c| c).await;
        let (mut transport, mut rx, hub) = start_outbound(&ca).await;
        discover_send(&transport, &hub, &direct_url(&remote.local_addr())).await;
        remote_rx
            .recv()
            .await
            .unwrap()
            .direct_response
            .unwrap()
            .send(NPDU, &DirectResponseScope::default())
            .await
            .unwrap();
        let capability = rx.recv().await.unwrap().direct_response.unwrap();
        let scope = DirectResponseScope::default();
        capability.send(NPDU, &scope).await.unwrap();
        assert_eq!(remote_rx.recv().await.unwrap().npdu.as_ref(), NPDU);
        let mut queued = Box::pin(capability.send(&[1, 0, 0x66], &scope));
        assert!(futures_util::poll!(&mut queued).is_pending());
        match mode {
            0 => {
                transport.stop().await.unwrap();
                transport.stop().await.unwrap();
            }
            1 => {
                transport.abort();
                transport.abort();
            }
            2 => drop(transport),
            _ => {
                transport = transport.with_direct_discovery(false);
                transport.stop().await.unwrap();
            }
        }
        assert!(queued.await.is_err());
        assert!(capability.send(NPDU, &scope).await.is_err());
        tokio::time::timeout(Duration::from_secs(3), async {
            while remote.active_connections() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            remote_rx.try_recv().is_err(),
            "queued response must not outlive teardown"
        );
        remote.stop().await;
    }
}
