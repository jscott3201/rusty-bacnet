//! Accept-side matrix for the opt-in direct listener (Refs #615).
//!
//! Loopback TLS fixtures only; all credentials are generated in-test with
//! `rcgen` and no keys or certificates are committed.

use super::{DirectAcceptConfig, DirectListener};
use crate::port::TransportPort;
use crate::sc::{ScConnection, ScTransport, WebSocketPort};
use crate::sc_frame::{
    decode_sc_bvlc_result, decode_sc_message, encode_sc_message, ScBvlcResult, ScFunction,
    ScMessage, ScOption,
};
use crate::sc_tls::ScNodeTlsConfig;
use bacnet_types::enums::{ErrorClass, ErrorCode};

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

fn direct_npdu_wire(
    message_id: u16,
    destination_vmac: Option<[u8; 6]>,
    dest_options: Vec<ScOption>,
) -> Vec<u8> {
    let msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id,
        originating_vmac: None,
        destination_vmac,
        dest_options,
        data_options: Vec::new(),
        payload: Bytes::from(NPDU.to_vec()),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &msg);
    buf.to_vec()
}

#[tokio::test]
async fn accept_mu_destination_option_naks_without_delivery() {
    let ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener(&ca, |c| c).await;
    let url = direct_url(&listener.local_addr());
    let (ws, _conn) = dial_and_handshake(&url, ca.node_config(vec!["node".into()])).await;
    // MU option with Header Data so the NAK must echo the wire marker verbatim.
    let wire = direct_npdu_wire(
        0x5151,
        None,
        vec![ScOption {
            option_type: 2,
            must_understand: true,
            data: vec![0xAA],
        }],
    );
    ws.send(&wire).await.unwrap();
    let nak_bytes = tokio::time::timeout(Duration::from_secs(5), ws.recv())
        .await
        .expect("MU NAK timed out")
        .unwrap();
    let nak = decode_sc_message(&nak_bytes).unwrap();
    assert_eq!(nak.function, ScFunction::Result);
    assert_eq!(nak.message_id, 0x5151);
    assert_eq!(nak.originating_vmac, None);
    assert_eq!(nak.destination_vmac, None);
    assert_eq!(
        decode_sc_bvlc_result(&nak).unwrap(),
        ScBvlcResult::Nak {
            result_for: ScFunction::EncapsulatedNpdu,
            error_header_marker: 0x62,
            error_class: ErrorClass::COMMUNICATION.to_raw(),
            error_code: ErrorCode::HEADER_NOT_UNDERSTOOD.to_raw(),
            error_details: String::new(),
        }
    );
    assert!(rx.try_recv().is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .is_err(),
        "MU destination option must never deliver"
    );
    assert_eq!(listener.active_connections(), 1);
    listener.stop().await;
}

#[tokio::test]
async fn accept_non_mu_destination_option_still_delivers() {
    let ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener(&ca, |c| c).await;
    let url = direct_url(&listener.local_addr());
    let (ws, _conn) = dial_and_handshake(&url, ca.node_config(vec!["node".into()])).await;
    let wire = direct_npdu_wire(
        0x5252,
        None,
        vec![ScOption {
            option_type: 31,
            must_understand: false,
            data: vec![0x12, 0x34],
        }],
    );
    ws.send(&wire).await.unwrap();
    let received = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("non-MU NPDU timed out")
        .expect("listener closed");
    assert_eq!(received.npdu.as_ref(), NPDU);
    assert!(
        tokio::time::timeout(Duration::from_millis(200), ws.recv())
            .await
            .is_err(),
        "non-MU destination option must not NAK"
    );
    listener.stop().await;
}

#[tokio::test]
async fn accept_broadcast_mu_destination_option_drops_without_nak() {
    let ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener(&ca, |c| c).await;
    let url = direct_url(&listener.local_addr());
    let (ws, _conn) = dial_and_handshake(&url, ca.node_config(vec!["node".into()])).await;
    let wire = direct_npdu_wire(
        0x5353,
        Some([0xFF; 6]),
        vec![ScOption {
            option_type: 31,
            must_understand: true,
            data: vec![0x12, 0x34, 0x56],
        }],
    );
    ws.send(&wire).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .is_err(),
        "broadcast MU destination option must never deliver"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), ws.recv())
            .await
            .is_err(),
        "broadcast MU destination option must not NAK"
    );
    assert_eq!(listener.active_connections(), 1);
    listener.stop().await;
}

async fn registered_listener(
    ca: &TestCa,
) -> (
    ScTransport<crate::sc::LoopbackWebSocket>,
    DirectListener,
    tokio::sync::mpsc::Receiver<crate::port::ReceivedNpdu>,
    crate::sc::LoopbackWebSocket,
) {
    let (client, hub) = crate::sc::LoopbackWebSocket::pair();
    let config = DirectAcceptConfig::new(
        loopback_addr(),
        LISTENER_VMAC,
        LISTENER_UUID,
        ca.node_config(vec!["localhost".into()]),
    );
    let (mut transport, listener) = ScTransport::new(client, LISTENER_VMAC)
        .with_device_uuid(LISTENER_UUID)
        .with_direct_listener(config)
        .await
        .unwrap();
    let (rx, ()) = tokio::join!(transport.start(), hub_accept(&hub, [0x10; 6]));
    (transport, listener, rx.unwrap(), hub)
}

async fn assert_accept_direct(hub: &crate::sc::LoopbackWebSocket, expected: u8) {
    // Independent AB.2.8.1 wire vector: no addresses/options, fresh ID,
    // primary-hub status, live accept flag, unchanged 5705/1476 maxima.
    hub.send(&[5, 0, 0x22, 0x34]).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(5), hub.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply.len(), 10);
    assert_ne!(&reply[2..4], &[0x22, 0x34]);
    assert_eq!(
        reply,
        [4, 0, reply[2], reply[3], 1, expected, 0x16, 0x49, 0x05, 0xC4]
    );
}

#[tokio::test]
async fn accept_registered_bit_is_one_while_listening_and_npdu_reaches_transport() {
    let ca = TestCa::generate();
    let (mut transport, mut listener, mut rx, hub) = registered_listener(&ca).await;
    assert_accept_direct(&hub, 1).await;
    let (ws, _) = dial_and_handshake(
        &direct_url(&listener.local_addr()),
        ca.node_config(vec!["node".into()]),
    )
    .await;
    // Direct NPDU with one non-MU proprietary Data Option. No address fields.
    ws.send(&[1, 1, 0x12, 0x34, 0x3F, 0, 3, 0x12, 0x34, 0x56, 1, 0, 0x30])
        .await
        .unwrap();
    let received = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.npdu.as_ref(), NPDU);
    assert_eq!(received.source_mac.as_ref(), &DIAL_VMAC);
    assert!(!received.link_layer_group);
    assert!(received.reply_tx.is_none());
    assert_eq!(received.data_attributes.len(), 1);
    assert_eq!(received.data_attributes[0].option_type, 31);
    assert!(!received.data_attributes[0].must_understand);
    assert_eq!(received.data_attributes[0].data, [0x12, 0x34, 0x56]);
    listener.stop().await;
    transport.stop().await.unwrap();
}

async fn registered_listener_reverts_to_zero(drop_listener: bool) {
    let ca = TestCa::generate();
    let (mut transport, listener, mut rx, hub) = registered_listener(&ca).await;
    let addr = listener.local_addr();
    let mut listener = Some(listener);
    assert_accept_direct(&hub, 1).await;
    if drop_listener {
        drop(listener.take());
    } else {
        listener.as_mut().unwrap().stop().await;
    }
    // Preserve the existing one-second solicited-reply gate, not a new timer.
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_accept_direct(&hub, 0).await;
    assert!(tokio::net::TcpStream::connect(addr).await.is_err());
    // Closed direct intake must neither terminate nor starve the hub intake.
    hub.send(&[
        1, 8, 0x12, 0x35, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 1, 0, 0x40,
    ])
    .await
    .unwrap();
    let received = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.npdu.as_ref(), &[1, 0, 0x40]);
    assert_eq!(received.source_mac.as_ref(), &[0x33; 6]);
    drop(listener);
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn accept_registered_bit_reverts_after_stop() {
    registered_listener_reverts_to_zero(false).await;
}

#[tokio::test]
async fn accept_registered_bit_reverts_after_drop() {
    registered_listener_reverts_to_zero(true).await;
}

#[tokio::test]
async fn accept_registered_bit_reverts_if_accept_task_is_cancelled() {
    let ca = TestCa::generate();
    let (mut transport, mut listener, _rx, hub) = registered_listener(&ca).await;
    assert_accept_direct(&hub, 1).await;
    listener.accept_task.as_ref().unwrap().abort();
    let mut stopped = listener.shutdown_status();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !*stopped.borrow_and_update() {
            stopped.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_accept_direct(&hub, 0).await;
    listener.stop().await;
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn accept_registered_bit_is_zero_without_application_intake_or_matching_identity() {
    let ca = TestCa::generate();
    let (mut transport, mut listener, rx, hub) = registered_listener(&ca).await;
    transport.connection().unwrap().lock().await.local_vmac = [0xBB; 6];
    assert_accept_direct(&hub, 0).await;
    transport.connection().unwrap().lock().await.local_vmac = LISTENER_VMAC;
    drop(rx);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_accept_direct(&hub, 0).await;
    listener.stop().await;
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn accept_registration_rejects_identity_mismatch_before_binding() {
    let ca = TestCa::generate();
    let occupied = tokio::net::TcpListener::bind(loopback_addr())
        .await
        .unwrap();
    let config = DirectAcceptConfig::new(
        occupied.local_addr().unwrap(),
        LISTENER_VMAC,
        LISTENER_UUID,
        ca.node_config(vec!["localhost".into()]),
    );
    let (client, _hub) = crate::sc::LoopbackWebSocket::pair();
    let result = ScTransport::new(client, DIAL_VMAC)
        .with_device_uuid(DIAL_UUID)
        .with_direct_listener(config)
        .await;
    assert!(
        matches!(result, Err(ref error) if error.to_string().contains("identity does not match"))
    );
}
