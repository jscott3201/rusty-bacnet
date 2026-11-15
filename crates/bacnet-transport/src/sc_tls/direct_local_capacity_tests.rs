//! Independent wire limits for both TLS direct connection directions.
use super::*;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

fn data(payload: Vec<u8>) -> Bytes {
    let mut bytes = BytesMut::new();
    encode_sc_message(
        &mut bytes,
        &ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 30,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: vec![],
            data_options: vec![],
            payload: Bytes::from(payload),
        },
    );
    bytes.freeze()
}
async fn assert_intake(rx: &mut tokio::sync::mpsc::Receiver<crate::port::ReceivedNpdu>) {
    let first = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.npdu.as_ref(), vec![0x55; 1478]);
    let marker = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        marker.npdu.as_ref(),
        &[1, 0, 0x66],
        "1479 must not reach intake before the same-socket marker"
    );
    assert!(first.provenance.direct_sc_identity().is_some());
}

#[tokio::test]
async fn accepted_direct_advertises_1478_and_enforces_1478_1479() {
    let ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener(&ca, |c| c).await;
    let peer = TlsWebSocket::connect_direct(
        &direct_url(&listener.local_addr()),
        ca.node_config(vec!["peer".into()]),
    )
    .await
    .unwrap();
    let mut conn = ScConnection::new(DIAL_VMAC, DIAL_UUID);
    let mut wire = BytesMut::new();
    encode_sc_message(&mut wire, &conn.build_connect_request());
    peer.send(&wire).await.unwrap();
    let accept = decode_sc_message(&peer.recv().await.unwrap()).unwrap();
    assert_eq!(accept.function, ScFunction::ConnectAccept);
    assert_eq!(accept.payload.len(), 26);
    assert_eq!(
        u16::from_be_bytes([accept.payload[22], accept.payload[23]]),
        5705
    );
    assert_eq!(
        u16::from_be_bytes([accept.payload[24], accept.payload[25]]),
        1478
    );
    for payload in [vec![0x55; 1478], vec![0x77; 1479], vec![1, 0, 0x66]] {
        peer.send(&data(payload)).await.unwrap();
    }
    assert_intake(&mut rx).await;
    listener.stop().await;
}

#[tokio::test]
async fn outbound_direct_advertises_1478_and_enforces_1478_1479_with_smaller_peer() {
    let ca = TestCa::generate();
    let tls = ca.node_config(vec!["localhost".into()]);
    let listener = tokio::net::TcpListener::bind(loopback_addr())
        .await
        .unwrap();
    let uri = direct_url(&listener.local_addr().unwrap());
    let (release, done) = tokio::sync::oneshot::channel();
    let peer = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let tls_stream = tls.acceptor().accept(tcp).await.unwrap();
        let mut ws = tokio_tungstenite::accept_hdr_async(
            tls_stream,
            super::super::super::direct_subprotocol_response,
        )
        .await
        .unwrap();
        let Message::Binary(bytes) = ws.next().await.unwrap().unwrap() else {
            panic!("Connect expected")
        };
        let request = decode_sc_message(&bytes).unwrap();
        assert_eq!(request.function, ScFunction::ConnectRequest);
        assert_eq!(request.payload.len(), 26);
        assert_eq!(
            u16::from_be_bytes([request.payload[22], request.payload[23]]),
            5705
        );
        assert_eq!(
            u16::from_be_bytes([request.payload[24], request.payload[25]]),
            1478
        );
        let mut payload = Vec::from(LISTENER_VMAC);
        payload.extend_from_slice(&LISTENER_UUID);
        payload.extend_from_slice(&5705u16.to_be_bytes());
        payload.extend_from_slice(&480u16.to_be_bytes());
        let mut bytes = BytesMut::new();
        encode_sc_message(
            &mut bytes,
            &ScMessage {
                function: ScFunction::ConnectAccept,
                message_id: request.message_id,
                originating_vmac: None,
                destination_vmac: None,
                dest_options: vec![],
                data_options: vec![],
                payload: Bytes::from(payload),
            },
        );
        ws.send(Message::Binary(bytes.to_vec().into()))
            .await
            .unwrap();
        let Message::Binary(bytes) = ws.next().await.unwrap().unwrap() else {
            panic!("initial NPDU expected")
        };
        assert_eq!(decode_sc_message(&bytes).unwrap().payload.as_ref(), NPDU);
        for payload in [vec![0x55; 1478], vec![0x77; 1479], vec![1, 0, 0x66]] {
            ws.send(Message::Binary(data(payload).to_vec().into()))
                .await
                .unwrap();
        }
        done.await.unwrap();
    });
    let (transport, mut rx, hub) = start_outbound(&ca).await;
    assert_eq!(transport.local_receive_apdu_capacity(), 1476);
    discover_send(&transport, &hub, &uri).await;
    assert_intake(&mut rx).await;
    assert_eq!(transport.local_receive_apdu_capacity(), 1476);
    transport.stop().await.unwrap();
    release.send(()).unwrap();
    peer.await.unwrap();
}
