//! RB-11 listener-path fairness over real TLS peers (Refs #518).
//!
//! Loopback TLS fixtures only; all credentials are generated in-test with
//! `rcgen` and no keys or certificates are committed. Asserts the
//! per-origin quota caps a bursty peer while another peer progresses, and
//! that startup validation rejects bad limits before binding.

use super::{DirectAcceptConfig, DirectListener};
use crate::sc::npdu_admission::{DEFAULT_NPDU_PER_ORIGIN_LIMIT as QUOTA, SC_NPDU_QUEUE_CAPACITY};
use crate::sc::{ScConnection, WebSocketPort};
use crate::sc_frame::{decode_sc_message, encode_sc_message, ScFunction};
use crate::sc_tls::ScNodeTlsConfig;

use std::net::SocketAddr;
use std::time::Duration;

use bytes::BytesMut;
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

const LISTENER_VMAC: [u8; 6] = [0xAA; 6];
const LISTENER_UUID: [u8; 16] = [9; 16];
const PEER_A_VMAC: [u8; 6] = [0x22; 6];
const PEER_B_VMAC: [u8; 6] = [0x23; 6];
const PEER_UUID: [u8; 16] = [7; 16];

fn loopback_addr() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

async fn start_listener(
    ca: &TestCa,
) -> (
    DirectListener,
    tokio::sync::mpsc::Receiver<crate::port::ReceivedNpdu>,
) {
    start_listener_with(ca, |config| config).await
}

async fn start_listener_with(
    ca: &TestCa,
    configure: impl FnOnce(DirectAcceptConfig) -> DirectAcceptConfig,
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
    DirectListener::start(configure(config)).await.unwrap()
}

async fn dialed_peer(
    addr: &SocketAddr,
    ca: &TestCa,
    vmac: [u8; 6],
) -> (super::super::TlsWebSocket, ScConnection) {
    let url = format!("wss://localhost:{}/.bacnet/sc", addr.port());
    let ws = tokio::time::timeout(
        Duration::from_secs(5),
        super::super::TlsWebSocket::connect_direct(&url, ca.node_config(vec!["node".into()])),
    )
    .await
    .expect("direct dial timed out")
    .expect("direct dial must succeed");
    let mut uuid = PEER_UUID;
    uuid[..6].copy_from_slice(&vmac);
    let mut conn = ScConnection::new(vmac, uuid);
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
    assert!(conn.handle_connect_accept(&accept));
    (ws, conn)
}

async fn send_direct_npdu(
    ws: &super::super::TlsWebSocket,
    conn: &mut ScConnection,
    payload: &[u8],
) {
    let direct = conn
        .build_direct_encapsulated_npdu(payload, &[])
        .expect("direct build must succeed");
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &direct);
    ws.send(&buf).await.unwrap();
}

#[tokio::test]
async fn bursty_peer_capped_while_other_peer_progresses() {
    let ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener(&ca).await;
    let addr = listener.local_addr();
    let (ws_a, mut conn_a) = dialed_peer(&addr, &ca, PEER_A_VMAC).await;
    let (ws_b, mut conn_b) = dialed_peer(&addr, &ca, PEER_B_VMAC).await;
    // Bursty peer floods: only its quota survives, whatever the interleave
    // with the legitimate peer (distinct keys, room in the queue).
    for seq in 0..(QUOTA + 16) as u8 {
        send_direct_npdu(&ws_a, &mut conn_a, &[0x01, 0x00, 0xA0, seq]).await;
    }
    send_direct_npdu(&ws_b, &mut conn_b, &[0x01, 0x00, 0xB0, 0]).await;
    send_direct_npdu(&ws_b, &mut conn_b, &[0x01, 0x00, 0xB0, 1]).await;
    // Closing A proves its stream was fully processed (close lands last), so
    // all of its drops are counted before the assertions below.
    drop(ws_a);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if listener.active_connections() == 1 && rx.len() == QUOTA + 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("listener did not settle");
    let counts = listener.npdu_drop_counts();
    assert_eq!(counts.fairness_drops, 16);
    assert_eq!(counts.full_drops, 0);
    assert_eq!(counts.closed_drops, 0);
    let mut from_a = 0;
    let mut from_b = 0;
    for _ in 0..QUOTA + 2 {
        let received = rx.try_recv().expect("the queued NPDUs must drain");
        assert!(received.provenance.is_direct_peer());
        assert!(received.reply_tx.is_none());
        assert!(!received.link_layer_group);
        if received.source_mac.as_ref() == PEER_A_VMAC {
            from_a += 1;
        } else if received.source_mac.as_ref() == PEER_B_VMAC {
            from_b += 1;
        } else {
            panic!("unexpected source {:?}", received.source_mac);
        }
    }
    assert_eq!((from_a, from_b), (QUOTA, 2));
    drop(ws_b);
    listener.stop().await;
    // Counters survive stop.
    assert_eq!(listener.npdu_drop_counts().fairness_drops, 16);
}

#[tokio::test]
async fn the_listener_control_rate_limit_drops_a_peers_excess_control_messages() {
    let ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener_with(&ca, |config| {
        config.with_control_rate_limit(crate::sc::ScControlRateLimit::new(2, 1))
    })
    .await;
    let addr = listener.local_addr();
    let (ws, mut conn) = dialed_peer(&addr, &ca, PEER_A_VMAC).await;
    // Five route announcements, then one APDU.
    for _ in 0..5 {
        send_direct_npdu(&ws, &mut conn, &[0x01, 0x80, 0x01, 0x03, 0xe8]).await;
    }
    send_direct_npdu(&ws, &mut conn, &[0x01, 0x00, 0x10, 0x08]).await;
    let mut received = Vec::new();
    while received.len() < 3 {
        let npdu = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("listener made no progress")
            .unwrap();
        received.push(npdu.npdu[1] & 0x80 != 0);
    }
    assert_eq!(
        received,
        [true, true, false],
        "two control messages, then the APDU"
    );
    assert_eq!(listener.npdu_drop_counts().control_rate_drops, 3);
    drop(ws);
    listener.stop().await;
}

#[tokio::test]
async fn invalid_per_origin_limit_rejected_before_bind() {
    let ca = TestCa::generate();
    for limit in [0, SC_NPDU_QUEUE_CAPACITY + 1, usize::MAX] {
        let tls = ca.node_config(vec!["localhost".into(), "127.0.0.1".into()]);
        let config = DirectAcceptConfig::new(loopback_addr(), LISTENER_VMAC, LISTENER_UUID, tls)
            .with_npdu_per_origin_limit(limit);
        match DirectListener::start(config).await {
            Ok(_) => panic!("limit {limit} must be rejected before binding"),
            Err(err) => assert!(
                err.to_string().contains("per-origin limit"),
                "limit {limit}: {err}"
            ),
        }
    }
}
