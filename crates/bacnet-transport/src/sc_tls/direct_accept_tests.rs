//! Accept-side matrix for the opt-in direct listener (Refs #615).
//!
//! Loopback TLS fixtures only; all credentials are generated in-test with
//! `rcgen` and no keys or certificates are committed.

use super::{DirectAcceptConfig, DirectListener};
use crate::port::TransportPort;
use crate::sc::{ScConnection, ScTransport, WebSocketPort};
use crate::sc_frame::{
    decode_sc_bvlc_result, decode_sc_message, encode_sc_message, ScBvlcResult, ScFunction,
    ScMessage,
};
use crate::sc_tls::ScNodeTlsConfig;

use std::net::SocketAddr;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

struct TestCa {
    params: rcgen::CertificateParams,
    key: rcgen::KeyPair,
    ca: CertificateDer<'static>,
}

impl TestCa {
    fn generate() -> Self {
        let mut params =
            rcgen::CertificateParams::new(Vec::<String>::new()).expect("empty SANs are valid");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = params.self_signed(&key).unwrap();
        Self {
            params,
            key,
            ca: cert.der().clone(),
        }
    }

    fn issue(&self, sans: Vec<String>) -> (Vec<CertificateDer<'static>>, rcgen::KeyPair) {
        let issuer = rcgen::Issuer::from_params(&self.params, &self.key);
        let params = rcgen::CertificateParams::new(sans).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = params.signed_by(&key, &issuer).unwrap();
        let chain: Vec<CertificateDer<'static>> =
            rustls::pki_types::pem::PemObject::pem_slice_iter(cert.pem().as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        (chain, key)
    }

    fn node_config(&self, sans: Vec<String>) -> ScNodeTlsConfig {
        let (chain, key) = self.issue(sans);
        ScNodeTlsConfig::from_der(
            vec![self.ca.clone()],
            chain,
            PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap()
    }
}

const LISTENER_VMAC: [u8; 6] = [0xAA; 6];
const LISTENER_UUID: [u8; 16] = [9; 16];
const DIAL_VMAC: [u8; 6] = [0x22; 6];
const DIAL_UUID: [u8; 16] = [7; 16];
const NPDU: &[u8] = &[0x01, 0x00, 0x30];

fn loopback_addr() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

async fn hub_accept(ws_hub: &crate::sc::LoopbackWebSocket, hub_vmac: [u8; 6]) {
    let data = ws_hub.recv().await.unwrap();
    let req = decode_sc_message(&data).unwrap();
    assert_eq!(req.function, ScFunction::ConnectRequest);
    let mut accept_payload = Vec::with_capacity(26);
    accept_payload.extend_from_slice(&hub_vmac);
    accept_payload.extend_from_slice(&[0x33; 16]);
    accept_payload.extend_from_slice(&1476u16.to_be_bytes());
    accept_payload.extend_from_slice(&1476u16.to_be_bytes());
    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: req.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(accept_payload),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &accept);
    ws_hub.send(&buf).await.unwrap();
}

async fn start_listener(
    ca: &TestCa,
    configure: impl FnOnce(DirectAcceptConfig) -> DirectAcceptConfig,
) -> (
    DirectListener,
    tokio::sync::mpsc::Receiver<crate::port::ReceivedNpdu>,
) {
    let (chain, key) = ca.issue(vec!["localhost".into(), "127.0.0.1".into()]);
    let tls = ScNodeTlsConfig::from_der(
        vec![ca.ca.clone()],
        chain,
        PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
    )
    .unwrap();
    let config = configure(DirectAcceptConfig::new(
        loopback_addr(),
        LISTENER_VMAC,
        LISTENER_UUID,
        tls,
    ));
    DirectListener::start(config).await.unwrap()
}

fn direct_url(addr: &SocketAddr) -> String {
    format!("wss://localhost:{}/.bacnet/sc", addr.port())
}

async fn dial_and_handshake(
    url: &str,
    tls: ScNodeTlsConfig,
) -> (super::super::TlsWebSocket, ScConnection) {
    let ws = tokio::time::timeout(
        Duration::from_secs(5),
        super::super::TlsWebSocket::connect_direct(url, tls),
    )
    .await
    .expect("direct dial timed out")
    .expect("direct dial must succeed");
    let mut conn = ScConnection::new(DIAL_VMAC, DIAL_UUID);
    let request = conn.build_connect_request();
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &request);
    ws.send(&buf).await.unwrap();
    let accept_bytes = tokio::time::timeout(Duration::from_secs(5), ws.recv())
        .await
        .expect("accept timed out")
        .unwrap();
    let accept = decode_sc_message(&accept_bytes).unwrap();
    assert_eq!(accept.function, ScFunction::ConnectAccept);
    assert_eq!(accept.message_id, request.message_id);
    assert!(conn.handle_connect_accept(&accept));
    (ws, conn)
}

#[tokio::test]
async fn accept_valid_handshake_delivers_npdu() {
    let ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener(&ca, |c| c).await;
    let url = direct_url(&listener.local_addr());
    let (ws, mut conn) = dial_and_handshake(&url, ca.node_config(vec!["node".into()])).await;
    let direct = conn
        .build_direct_encapsulated_npdu(NPDU, &[])
        .expect("direct build must succeed");
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &direct);
    ws.send(&buf).await.unwrap();
    let received = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("NPDU timed out")
        .expect("listener closed");
    assert_eq!(received.npdu.as_ref(), NPDU);
    assert_eq!(received.source_mac.as_ref(), &DIAL_VMAC);
    assert!(!received.link_layer_group);
    assert!(received.data_attributes.is_empty());
    listener.stop().await;
}

#[tokio::test]
async fn accept_bad_identity_naks_and_closes_without_delivery() {
    let ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener(&ca, |c| c).await;
    let url = direct_url(&listener.local_addr());
    let ws = tokio::time::timeout(
        Duration::from_secs(5),
        super::super::TlsWebSocket::connect_direct(&url, ca.node_config(vec!["node".into()])),
    )
    .await
    .expect("dial timed out")
    .expect("dial must succeed");
    let mut payload = Vec::with_capacity(26);
    payload.extend_from_slice(&[0; 6]);
    payload.extend_from_slice(&DIAL_UUID);
    payload.extend_from_slice(&1476u16.to_be_bytes());
    payload.extend_from_slice(&1476u16.to_be_bytes());
    let bad = ScMessage {
        function: ScFunction::ConnectRequest,
        message_id: 0x1234,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &bad);
    ws.send(&buf).await.unwrap();
    let nak_bytes = tokio::time::timeout(Duration::from_secs(5), ws.recv())
        .await
        .expect("NAK timed out")
        .unwrap();
    let nak = decode_sc_message(&nak_bytes).unwrap();
    assert_eq!(nak.function, ScFunction::Result);
    assert_eq!(nak.message_id, 0x1234);
    let result = decode_sc_bvlc_result(&nak).unwrap();
    assert!(matches!(result, ScBvlcResult::Nak { .. }));
    let closed = tokio::time::timeout(Duration::from_secs(5), ws.recv()).await;
    match closed {
        Ok(Err(_)) => {}
        Ok(Ok(_)) => panic!("bad identity must close without further frames"),
        Err(_) => panic!("bad identity close timed out"),
    }
    assert!(rx.try_recv().is_err());
    listener.stop().await;
}

#[tokio::test]
async fn accept_zero_limits_naks_and_closes_without_delivery() {
    let ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener(&ca, |c| c).await;
    let url = direct_url(&listener.local_addr());
    let ws = tokio::time::timeout(
        Duration::from_secs(5),
        super::super::TlsWebSocket::connect_direct(&url, ca.node_config(vec!["node".into()])),
    )
    .await
    .expect("dial timed out")
    .expect("dial must succeed");
    let mut payload = Vec::with_capacity(26);
    payload.extend_from_slice(&DIAL_VMAC);
    payload.extend_from_slice(&DIAL_UUID);
    payload.extend_from_slice(&0u16.to_be_bytes());
    payload.extend_from_slice(&1476u16.to_be_bytes());
    let bad = ScMessage {
        function: ScFunction::ConnectRequest,
        message_id: 0x2345,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &bad);
    ws.send(&buf).await.unwrap();
    let nak_bytes = tokio::time::timeout(Duration::from_secs(5), ws.recv())
        .await
        .expect("NAK timed out")
        .unwrap();
    let nak = decode_sc_message(&nak_bytes).unwrap();
    assert_eq!(nak.function, ScFunction::Result);
    assert!(matches!(
        decode_sc_bvlc_result(&nak).unwrap(),
        ScBvlcResult::Nak { .. }
    ));
    assert!(rx.try_recv().is_err());
    listener.stop().await;
}

#[tokio::test]
async fn accept_hub_subprotocol_is_cleanly_refused() {
    let ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener(&ca, |c| c).await;
    let url = direct_url(&listener.local_addr());
    let refused =
        super::super::TlsWebSocket::connect(&url, ca.node_config(vec!["node".into()])).await;
    match refused {
        Ok(_) => panic!("hub subprotocol must be refused"),
        Err(err) => assert!(err.to_string().contains("WebSocket")),
    }
    assert!(rx.try_recv().is_err());
    assert_eq!(listener.active_connections(), 0);
    listener.stop().await;
}

#[tokio::test]
async fn accept_untrusted_client_cert_fails_tls_before_bacnet() {
    let server_ca = TestCa::generate();
    let client_ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener(&server_ca, |c| c).await;
    let url = direct_url(&listener.local_addr());
    let refused = super::super::TlsWebSocket::connect_direct(
        &url,
        client_ca.node_config(vec!["node".into()]),
    )
    .await;
    match refused {
        Ok(_) => panic!("untrusted client must fail TLS"),
        Err(err) => {
            assert!(err.to_string().contains("TLS") || err.to_string().contains("WebSocket"))
        }
    }
    assert!(rx.try_recv().is_err());
    listener.stop().await;
}

#[tokio::test]
async fn accept_idle_timeout_cleans_up_without_delivery() {
    let ca = TestCa::generate();
    let (mut listener, mut rx) =
        start_listener(&ca, |c| c.with_idle_timeout(Duration::from_millis(120))).await;
    let url = direct_url(&listener.local_addr());
    let (_ws, _conn) = dial_and_handshake(&url, ca.node_config(vec!["node".into()])).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if listener.active_connections() == 0 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "idle close timed out"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(rx.try_recv().is_err());
    listener.stop().await;
}

#[tokio::test]
async fn accept_close_cleans_up_active_count() {
    let ca = TestCa::generate();
    let (mut listener, _rx) = start_listener(&ca, |c| c).await;
    let url = direct_url(&listener.local_addr());
    let (ws, _conn) = dial_and_handshake(&url, ca.node_config(vec!["node".into()])).await;
    assert_eq!(listener.active_connections(), 1);
    drop(ws);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if listener.active_connections() == 0 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "close cleanup timed out"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    listener.stop().await;
}

#[tokio::test]
async fn accept_rejects_reserved_local_identity_before_bind() {
    let ca = TestCa::generate();
    let tls = ca.node_config(vec!["node".into()]);
    for vmac in [[0; 6], [0xFF; 6]] {
        let config = DirectAcceptConfig::new(loopback_addr(), vmac, LISTENER_UUID, tls.clone());
        assert!(DirectListener::start(config).await.is_err());
    }
    let config = DirectAcceptConfig::new(loopback_addr(), LISTENER_VMAC, [0; 16], tls);
    assert!(DirectListener::start(config).await.is_err());
}

#[tokio::test]
async fn accept_hub_transport_path_is_untouched() {
    let (client, hub) = crate::sc::LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(client, [0x01; 6]).with_device_uuid([1; 16]);
    let hub_task = tokio::spawn(async move {
        hub_accept(&hub, [0x10; 6]).await;
        hub
    });
    let _rx = transport.start().await.unwrap();
    let hub = hub_task.await.unwrap();
    transport.send_unicast(NPDU, &[0x22; 6]).await.unwrap();
    let bytes = tokio::time::timeout(Duration::from_secs(2), hub.recv())
        .await
        .expect("hub recv timed out")
        .unwrap();
    let msg = decode_sc_message(&bytes).unwrap();
    assert_eq!(msg.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(msg.destination_vmac, Some([0x22; 6]));
    transport.stop().await.unwrap();
}
