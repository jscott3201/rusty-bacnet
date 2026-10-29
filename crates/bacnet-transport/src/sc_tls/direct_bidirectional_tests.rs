//! Ordinary direct traffic in both directions, using real TLS sockets.
use super::*;
use crate::port::DirectResponseScope;
use crate::sc::LoopbackWebSocket;
use crate::sc_tls::TlsWebSocket;

#[tokio::test]
async fn accepted_ordinary_send_selects_established_socket_without_discovery() {
    let ca = TestCa::generate();
    let (ws, hub) = LoopbackWebSocket::pair();
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
    let (peer, mut conn) = dial_and_handshake(
        &direct_url(&listener.local_addr()),
        ca.node_config(vec!["peer".into()]),
    )
    .await;
    // Receipt through intake proves publication before ordinary selection.
    let mut bytes = BytesMut::new();
    encode_sc_message(
        &mut bytes,
        &conn.build_direct_encapsulated_npdu(NPDU, &[]).unwrap(),
    );
    peer.send(&bytes).await.unwrap();
    assert_eq!(rx.recv().await.unwrap().npdu.as_ref(), NPDU);
    transport.send_unicast(NPDU, &DIAL_VMAC).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        tokio::select! {
            direct = peer.recv() => {
                let msg = decode_sc_message(&direct.unwrap()).unwrap();
                assert_eq!(msg.function, ScFunction::EncapsulatedNpdu);
                assert_eq!(msg.payload.as_ref(), NPDU);
                assert_eq!((msg.originating_vmac, msg.destination_vmac), (None, None));
            }
            hub = hub.recv() => panic!("established direct send incorrectly reached Hub: {:?}", decode_sc_message(&hub.unwrap()).unwrap().function),
        }
    }).await.unwrap();
    let attribute = crate::port::DataAttribute {
        option_type: 3,
        must_understand: false,
        data: vec![7, 8],
    };
    transport
        .send_unicast_with_data_attributes(NPDU, &DIAL_VMAC, std::slice::from_ref(&attribute))
        .await
        .unwrap();
    let ordinary = decode_sc_message(&peer.recv().await.unwrap()).unwrap();
    assert_eq!(
        ordinary.data_options,
        vec![crate::sc_frame::ScOption {
            option_type: 3,
            must_understand: false,
            data: attribute.data
        }]
    );
    assert!(ordinary.originating_vmac.is_none() && ordinary.destination_vmac.is_none());
    transport.send_broadcast(NPDU).await.unwrap();
    let broadcast = decode_sc_message(&hub.recv().await.unwrap()).unwrap();
    assert_eq!(broadcast.destination_vmac, Some([0xff; 6]));
    assert_eq!(broadcast.payload.as_ref(), NPDU);
    assert!(futures_util::poll!(Box::pin(peer.recv()).as_mut()).is_pending());
    transport.stop().await.unwrap();
    listener.stop().await;
}

#[tokio::test]
async fn outbound_real_tls_npdu_reaches_transport_with_original_response() {
    let ca = TestCa::generate();
    let (mut remote, mut remote_rx) = start_listener(&ca, |c| c).await;
    let uri = direct_url(&remote.local_addr());
    let (ws, hub) = LoopbackWebSocket::pair();
    let tls = ca.node_config(vec!["caller".into()]);
    let mut transport = ScTransport::new(ws, DIAL_VMAC)
        .with_device_uuid(DIAL_UUID)
        .with_direct_tls(tls);
    let (started, ()) = tokio::join!(transport.start(), hub_accept(&hub, [0x33; 6]));
    let mut rx = started.unwrap();
    let discovery = async {
        let request = decode_sc_message(&hub.recv().await.unwrap()).unwrap();
        assert_eq!(request.function, ScFunction::AddressResolution);
        let ack = ScMessage {
            function: ScFunction::AddressResolutionAck,
            message_id: request.message_id,
            originating_vmac: Some(LISTENER_VMAC),
            destination_vmac: None,
            dest_options: vec![],
            data_options: vec![],
            payload: Bytes::from(uri),
        };
        let mut bytes = BytesMut::new();
        encode_sc_message(&mut bytes, &ack);
        hub.send(&bytes).await.unwrap();
    };
    let (sent, ()) = tokio::join!(transport.send_unicast(NPDU, &LISTENER_VMAC), discovery);
    sent.unwrap();
    let received = tokio::time::timeout(Duration::from_secs(3), remote_rx.recv())
        .await
        .unwrap()
        .unwrap();
    received
        .direct_response
        .unwrap()
        .send(&[1, 0, 0x44], &DirectResponseScope::default())
        .await
        .unwrap();
    let received = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("outbound application NPDU was discarded")
        .unwrap();
    assert_eq!(received.npdu.as_ref(), &[1, 0, 0x44]);
    let response = received
        .direct_response
        .expect("original outbound response capability");
    response
        .send(NPDU, &DirectResponseScope::default())
        .await
        .unwrap();
    assert_eq!(remote_rx.recv().await.unwrap().npdu.as_ref(), NPDU);
    transport.stop().await.unwrap();
    remote.stop().await;
}

#[tokio::test]
async fn accepted_direct_shared_fifo_saturation_never_uses_hub_and_cancelled_work_cannot_start() {
    let ca = TestCa::generate();
    let (ws, hub) = LoopbackWebSocket::pair();
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
    let (peer, mut conn) = dial_and_handshake(
        &direct_url(&listener.local_addr()),
        ca.node_config(vec!["peer".into()]),
    )
    .await;
    let mut bytes = BytesMut::new();
    encode_sc_message(
        &mut bytes,
        &conn.build_direct_encapsulated_npdu(NPDU, &[]).unwrap(),
    );
    peer.send(&bytes).await.unwrap();
    let response = rx.recv().await.unwrap().direct_response.unwrap();
    // Current-thread runtime: queue all work before permitting a worker poll.
    let mut pending: Vec<_> = (0..64)
        .map(|_| Box::pin(transport.send_unicast(NPDU, &DIAL_VMAC)))
        .collect();
    for send in &mut pending {
        assert!(futures_util::poll!(send.as_mut()).is_pending());
    }
    let error = transport.send_unicast(NPDU, &DIAL_VMAC).await.unwrap_err();
    assert!(
        matches!(error, bacnet_types::error::Error::Transport(e) if e.kind() == std::io::ErrorKind::WouldBlock)
    );
    let scope = DirectResponseScope::default();
    assert!(
        response.send(NPDU, &scope).await.is_err(),
        "ordinary occupancy also bounds original replies"
    );
    assert!(futures_util::poll!(Box::pin(hub.recv()).as_mut()).is_pending());
    drop(pending);
    // A fresh marker response is admitted once the sole writer has discarded
    // all cancelled requests. No guessed sleep establishes this boundary.
    let route = transport.direct_route_for_test(&DIAL_VMAC).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while route.send.capacity() != 64 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    response.send(&[1, 0, 0x55], &scope).await.unwrap();
    let frame = decode_sc_message(&peer.recv().await.unwrap()).unwrap();
    assert_eq!(
        frame.payload.as_ref(),
        &[1, 0, 0x55],
        "cancelled ordinary NPDUs must not precede marker"
    );
    assert!(futures_util::poll!(Box::pin(hub.recv()).as_mut()).is_pending());
    transport.stop().await.unwrap();
    listener.stop().await;
}

#[tokio::test]
async fn outbound_direct_frame_adapter_yields_every_ready_websocket_control() {
    use futures_util::StreamExt;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio_tungstenite::tungstenite::Message;
    let consumed = Arc::new(AtomicUsize::new(0));
    let count = consumed.clone();
    let messages = (0..16).map(|i| {
        Ok::<_, String>(if i % 2 == 0 {
            Message::Ping(vec![i].into())
        } else {
            Message::Pong(vec![i].into())
        })
    });
    let mut stream = futures_util::stream::iter(
        messages.chain([Ok(Message::Binary(vec![1].into()))]),
    )
    .inspect(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
    });
    for turn in 1..=16 {
        assert!(matches!(
            crate::sc_tls::direct_frame(&mut stream).await.unwrap(),
            crate::sc::direct_socket::DirectFrame::Control
        ));
        assert_eq!(
            consumed.load(Ordering::SeqCst),
            turn,
            "one physical control per scheduler turn"
        );
    }
    assert!(
        matches!(crate::sc_tls::direct_frame(&mut stream).await.unwrap(), crate::sc::direct_socket::DirectFrame::Binary(b) if b == [1])
    );
}

async fn start_outbound(
    ca: &TestCa,
) -> (
    ScTransport<LoopbackWebSocket>,
    tokio::sync::mpsc::Receiver<crate::port::ReceivedNpdu>,
    LoopbackWebSocket,
) {
    let (ws, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws, DIAL_VMAC)
        .with_device_uuid(DIAL_UUID)
        .with_direct_tls(ca.node_config(vec!["caller".into()]));
    let (started, ()) = tokio::join!(transport.start(), hub_accept(&hub, [0x33; 6]));
    (transport, started.unwrap(), hub)
}
async fn discover_send(
    transport: &ScTransport<LoopbackWebSocket>,
    hub: &LoopbackWebSocket,
    uri: &str,
) {
    let discovery = async {
        let request = decode_sc_message(&hub.recv().await.unwrap()).unwrap();
        assert_eq!(request.function, ScFunction::AddressResolution);
        let mut bytes = BytesMut::new();
        encode_sc_message(
            &mut bytes,
            &ScMessage {
                function: ScFunction::AddressResolutionAck,
                message_id: request.message_id,
                originating_vmac: Some(LISTENER_VMAC),
                destination_vmac: None,
                dest_options: vec![],
                data_options: vec![],
                payload: Bytes::copy_from_slice(uri.as_bytes()),
            },
        );
        hub.send(&bytes).await.unwrap();
    };
    let (sent, ()) = tokio::join!(transport.send_unicast(NPDU, &LISTENER_VMAC), discovery);
    sent.unwrap();
}

#[path = "direct_outbound_identity_tests.rs"]
mod identity_tests;

#[path = "direct_outbound_intake_tests.rs"]
mod intake_tests;

#[path = "direct_local_capacity_tests.rs"]
mod local_capacity_tests;
