use super::*;
use crate::port::TransportPort;
use crate::sc::{ScConnectionState, ScTransport};

// Tungstenite's handshake callback requires its unboxed HTTP error response.
#[allow(clippy::result_large_err)]
async fn check_invalid_accept(recover: bool, field: std::ops::Range<usize>) {
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
        tokio_tungstenite::accept_hdr_async(
            tls,
            |_: &_, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                response.headers_mut().insert(
                    "Sec-WebSocket-Protocol",
                    BACNET_SC_HUB_SUBPROTOCOL.parse().unwrap(),
                );
                Ok(response)
            },
        )
        .await
        .unwrap()
    };
    let (ws, mut peer) = tokio::join!(TlsWebSocket::connect(&url, node), peer);
    let mut transport = ScTransport::new(ws.unwrap(), [0x02; 6])
        .with_device_uuid([0x12; 16])
        .with_connect_timeout_ms(500);
    let mut state = transport.connection_state_changes();
    let malformed_hub = async {
        let request = peer.next().await.unwrap().unwrap().into_data();
        assert_eq!(&request[..4], &[6, 0, 0, 1]);
        assert_eq!(&request[4..10], &[0x02; 6]);
        assert_eq!(&request[10..26], &[0x12; 16]);
        assert_eq!(*state.borrow_and_update(), ScConnectionState::Connecting);
        // Independently specified Connect-Accept bytes, not the product encoder.
        let mut accept = vec![
            7, 0, 0, 1, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0x20, 0, 0x10, 0,
        ];
        accept[10..26].fill(0xff); // nonzero UUID bits stay opaque
        let valid = accept.clone();
        accept[field].fill(0);
        for _ in 0..3 {
            peer.send(Message::Binary(accept.clone().into()))
                .await
                .unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(20), peer.next())
                    .await
                    .is_err(),
                "invalid Accept must not elicit any wire response or close"
            );
            assert!(
                !state.has_changed().unwrap(),
                "invalid Accept published state"
            );
        }
        if recover {
            peer.send(Message::Binary(valid.into())).await.unwrap();
        } else {
            // The native startup timeout drops the socket. No BVLC reply is legal.
            let end = peer.next().await;
            assert!(!matches!(end, Some(Ok(Message::Binary(_)))));
        }
    };
    let (started, ()) = tokio::join!(transport.start(), malformed_hub);
    let conn = transport.connection().unwrap().lock().await;
    assert_eq!(conn.local_vmac, [0x02; 6]);
    assert_eq!(conn.device_uuid, [0x12; 16]);
    if recover {
        let _rx = started.unwrap();
        assert_eq!(conn.state, ScConnectionState::Connected);
        assert_eq!(conn.hub_vmac, Some([0x22; 6]));
        assert_eq!(conn.hub_device_uuid, Some([0xff; 16]));
        assert_eq!(
            (conn.hub_max_bvlc_length, conn.hub_max_apdu_length),
            (8192, 4096)
        );
    } else {
        assert!(matches!(started, Err(Error::Timeout(d)) if d == Duration::from_millis(500)));
        assert_eq!(conn.state, ScConnectionState::Disconnected);
        assert_eq!(conn.hub_vmac, None);
        assert_eq!(conn.hub_device_uuid, None);
        assert_eq!(
            (conn.hub_max_bvlc_length, conn.hub_max_apdu_length),
            (1476, 1476)
        );
    }
    drop(conn);
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn nil_accept_tls_is_silent_until_later_valid_accept() {
    tokio::time::timeout(Duration::from_secs(5), check_invalid_accept(true, 10..26))
        .await
        .unwrap();
}

#[tokio::test]
async fn nil_accept_tls_expires_without_peer_identity_or_limits() {
    tokio::time::timeout(Duration::from_secs(5), check_invalid_accept(false, 10..26))
        .await
        .unwrap();
}

#[tokio::test]
async fn zero_limits_accept_tls_is_silent_until_later_valid_accept() {
    for field in [26..28, 28..30, 26..30] {
        tokio::time::timeout(Duration::from_secs(5), check_invalid_accept(true, field))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn zero_limits_accept_tls_expires_without_peer_identity_or_limits() {
    for field in [26..28, 28..30, 26..30] {
        tokio::time::timeout(Duration::from_secs(5), check_invalid_accept(false, field))
            .await
            .unwrap();
    }
}
