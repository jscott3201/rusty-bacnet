//! RB-08 direct-accept provenance gaps (Refs #518, Refs #524 — baseline only).
//!
//! Pins two genuine gaps in the accept-side spoof-resistance story, with
//! loopback TLS fixtures only (in-test `rcgen` CA, no committed keys, no
//! site scans, no sleeps):
//! - pre-handshake NPDU bytes (post-TLS, pre-Connect-Request) close the
//!   connection with no `ReceivedNpdu` and therefore no verified value;
//! - a colliding-VMAC Connect-Request is NAKed
//!   (`COMMUNICATION`/`NODE_DUPLICATE_VMAC`) and closed with no delivery.
//! Verified minters remain exactly `sc/mod.rs:624` (hub relayed,
//! post-admission) and `sc_tls/direct_accept.rs:594` (direct, post-handshake),
//! both `pub(crate)`; nothing here mints provenance.

use super::{DirectAcceptConfig, DirectListener};
use crate::sc::WebSocketPort;
use crate::sc_frame::{
    decode_sc_bvlc_result, decode_sc_message, encode_sc_message, ScBvlcResult, ScFunction,
    ScMessage,
};
use crate::sc_tls::ScNodeTlsConfig;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bytes::{Bytes, BytesMut};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use std::net::SocketAddr;
use std::time::Duration;

const LISTENER_VMAC: [u8; 6] = [0xAA; 6];
const LISTENER_UUID: [u8; 16] = [9; 16];
const DIAL_UUID: [u8; 16] = [7; 16];
const NPDU: &[u8] = &[0x01, 0x00, 0x30];

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

    fn node_config(&self, sans: Vec<String>) -> ScNodeTlsConfig {
        let issuer = rcgen::Issuer::from_params(&self.params, &self.key);
        let params = rcgen::CertificateParams::new(sans).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = params.signed_by(&key, &issuer).unwrap();
        let chain: Vec<CertificateDer<'static>> =
            rustls::pki_types::pem::PemObject::pem_slice_iter(cert.pem().as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        ScNodeTlsConfig::from_der(
            vec![self.ca.clone()],
            chain,
            PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap()
    }
}

fn loopback_addr() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

async fn start_listener(
    ca: &TestCa,
) -> (
    DirectListener,
    tokio::sync::mpsc::Receiver<crate::port::ReceivedNpdu>,
) {
    let (chain, key) = {
        let issuer = rcgen::Issuer::from_params(&ca.params, &ca.key);
        let params =
            rcgen::CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = params.signed_by(&key, &issuer).unwrap();
        let chain: Vec<CertificateDer<'static>> =
            rustls::pki_types::pem::PemObject::pem_slice_iter(cert.pem().as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        (chain, key)
    };
    let tls = ScNodeTlsConfig::from_der(
        vec![ca.ca.clone()],
        chain,
        PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
    )
    .unwrap();
    let config = DirectAcceptConfig::new(loopback_addr(), LISTENER_VMAC, LISTENER_UUID, tls);
    DirectListener::start(config).await.unwrap()
}

fn direct_url(addr: &SocketAddr) -> String {
    format!("wss://localhost:{}/.bacnet/sc", addr.port())
}

async fn tls_dial(url: &str, tls: ScNodeTlsConfig) -> crate::sc_tls::TlsWebSocket {
    tokio::time::timeout(
        Duration::from_secs(5),
        crate::sc_tls::TlsWebSocket::connect_direct(url, tls),
    )
    .await
    .expect("direct dial timed out")
    .expect("direct dial must succeed")
}

#[tokio::test]
async fn pre_handshake_npdu_closes_without_delivery_or_verified_value() {
    let ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener(&ca).await;
    let url = direct_url(&listener.local_addr());
    let ws = tls_dial(&url, ca.node_config(vec!["node".into()])).await;

    // Encapsulated-NPDU as the first post-TLS frame: the handshake expects
    // Connect-Request, so this must close with no delivery (hence no verified
    // mint) and no NAK.
    let early = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x4242,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(NPDU.to_vec()),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &early);
    ws.send(&buf).await.unwrap();

    let closed = tokio::time::timeout(Duration::from_secs(5), ws.recv())
        .await
        .expect("pre-handshake close timed out");
    assert!(
        closed.is_err(),
        "pre-handshake NPDU must close without a response frame"
    );
    assert!(
        rx.try_recv().is_err(),
        "pre-handshake bytes must never deliver"
    );
    listener.stop().await;
}

#[tokio::test]
async fn colliding_vmac_connect_request_naks_duplicate_without_delivery() {
    let ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener(&ca).await;
    let url = direct_url(&listener.local_addr());
    let ws = tls_dial(&url, ca.node_config(vec!["node".into()])).await;

    // Well-formed Connect-Request that collides with the listener identity.
    let mut payload = Vec::with_capacity(26);
    payload.extend_from_slice(&LISTENER_VMAC);
    payload.extend_from_slice(&DIAL_UUID);
    payload.extend_from_slice(&1476u16.to_be_bytes());
    payload.extend_from_slice(&1476u16.to_be_bytes());
    let colliding = ScMessage {
        function: ScFunction::ConnectRequest,
        message_id: 0x1234,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &colliding);
    ws.send(&buf).await.unwrap();

    let nak_bytes = tokio::time::timeout(Duration::from_secs(5), ws.recv())
        .await
        .expect("duplicate-VMAC NAK timed out")
        .expect("listener must answer the colliding request");
    let nak = decode_sc_message(&nak_bytes).unwrap();
    assert_eq!(nak.function, ScFunction::Result);
    assert_eq!(nak.message_id, 0x1234);
    match decode_sc_bvlc_result(&nak).unwrap() {
        ScBvlcResult::Nak {
            error_class,
            error_code,
            ..
        } => {
            assert_eq!(error_class, ErrorClass::COMMUNICATION.to_raw());
            assert_eq!(error_code, ErrorCode::NODE_DUPLICATE_VMAC.to_raw());
        }
        other => panic!("expected duplicate-VMAC NAK, got {other:?}"),
    }
    assert!(rx.try_recv().is_err(), "colliding VMAC must never deliver");
    // The colliding handshake is refused: the peer observes close afterwards.
    let closed = tokio::time::timeout(Duration::from_secs(5), ws.recv())
        .await
        .expect("duplicate-VMAC close timed out");
    assert!(closed.is_err(), "colliding VMAC must close after the NAK");
    listener.stop().await;
}
