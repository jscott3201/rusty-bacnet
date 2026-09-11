use super::*;
use crate::port::TransportPort;
use crate::sc::{ScConnectionState, ScTransport};
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse, Request, Response,
};

struct HubSubprotocol;

impl Callback for HubSubprotocol {
    fn on_request(self, _: &Request, mut response: Response) -> Result<Response, ErrorResponse> {
        response.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            BACNET_SC_HUB_SUBPROTOCOL.parse().unwrap(),
        );
        Ok(response)
    }
}

#[tokio::test]
async fn mu_liveness_tls_wire_rejection_and_healthy_recovery() {
    tokio::time::timeout(Duration::from_secs(5), exercise())
        .await
        .unwrap();
}

async fn exercise() {
    let (node, server) = test_tls_pair();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://localhost:{}", listener.local_addr().unwrap().port());
    let peer = async {
        let (tcp, _) = listener.accept().await.unwrap();
        let tls = TlsAcceptor::from(server).accept(tcp).await.unwrap();
        assert_eq!(
            tls.get_ref().1.protocol_version(),
            Some(ProtocolVersion::TLSv1_3)
        );
        tokio_tungstenite::accept_hdr_async(tls, HubSubprotocol)
            .await
            .unwrap()
    };
    let (ws, mut peer) = tokio::join!(TlsWebSocket::connect(&url, node), peer);
    let mut transport = ScTransport::new(ws.unwrap(), [1; 6]).with_device_uuid([1; 16]);
    let accept = async {
        let request = peer.next().await.unwrap().unwrap().into_data();
        assert_eq!(&request[..4], &[6, 0, 0, 1]);
        let mut accept = vec![7, 0, 0, 1];
        accept.extend_from_slice(&[0x10; 6]);
        accept.extend_from_slice(&[0x33; 16]);
        accept.extend_from_slice(&[5, 0xC4, 5, 0xC4]);
        peer.send(Message::Binary(accept.into())).await.unwrap();
    };
    let (rx, ()) = tokio::join!(transport.start(), accept);
    let mut rx = rx.unwrap();
    for broadcast in [false, true] {
        let mut rejected = vec![1, if broadcast { 14 } else { 10 }, 0x22, 0x33];
        rejected.extend_from_slice(&[0x22; 6]);
        if broadcast {
            rejected.extend_from_slice(&[0xFF; 6]);
        }
        rejected.extend_from_slice(&[0xE2, 0, 0, 0x1F, 1, 0, 0x30]);
        peer.send(Message::Binary(rejected.into())).await.unwrap();
        if !broadcast {
            let mut nak = vec![0, 4, 0x22, 0x33];
            nak.extend_from_slice(&[0x22; 6]);
            nak.extend_from_slice(&[1, 1, 0xE2, 0, 7, 0, 0x92]);
            assert_eq!(peer.next().await.unwrap().unwrap().into_data(), nak);
        }
        peer.send(Message::Binary(vec![0x0A, 0, 0x33, 0x44, 0x42].into()))
            .await
            .unwrap();
        assert_eq!(
            peer.next().await.unwrap().unwrap().into_data().as_ref(),
            &[0, 0, 0x33, 0x44, 0x0A, 1, 0, 0, 7, 0, 7]
        );
        assert!(
            rx.try_recv().is_err(),
            "MU-rejected NPDU was delivered over TLS"
        );
    }
    let mut accepted = vec![1, 11, 0x44, 0x55];
    accepted.extend_from_slice(&[0x22; 6]);
    accepted.extend_from_slice(&[0x22, 0, 0, 0x62, 0, 1, 0xAA, 1, 0, 0x30]);
    peer.send(Message::Binary(accepted.into())).await.unwrap();
    let received = rx.recv().await.unwrap();
    assert_eq!(received.npdu.as_ref(), &[1, 0, 0x30]);
    assert_eq!(received.source_mac.as_slice(), &[0x22; 6]);
    assert_eq!(
        received.data_attributes,
        vec![crate::port::DataAttribute {
            option_type: 2,
            must_understand: true,
            data: vec![0xAA]
        }]
    );
    peer.send(Message::Binary(vec![0x0A, 0, 0x66, 0x77].into()))
        .await
        .unwrap();
    assert_eq!(
        peer.next().await.unwrap().unwrap().into_data().as_ref(),
        &[0x0B, 0, 0x66, 0x77]
    );
    assert_eq!(
        *transport.connection_state_changes().borrow(),
        ScConnectionState::Connected
    );
    transport.stop().await.unwrap();
}
