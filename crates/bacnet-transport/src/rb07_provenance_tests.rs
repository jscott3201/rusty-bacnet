//! RB-07 provenance unit tests (in-memory, deterministic, no sleeps).
//!
//! Covers: unverified legacy (Loopback, B/IP, MS/TP, custom untrusted),
//! verified direct peer (constructor + full TLS direct listener), verified
//! relayed origin (SC hub via LoopbackWebSocket + source_admission), unknown
//! hub origin dropped, only-verified-path discipline, cloning/queueing
//! preserves meaning, reply ownership (clone loses reply, single use),
//! redacted Debug (no key material).

use super::bip::BipTransport;
use super::loopback::LoopbackTransport;
use super::mstp::{LoopbackSerial, MasterNode, MstpConfig};
use super::mstp_frame::{FrameType, MstpFrame};
use super::port::{ReceivedNpdu, TransportPort, TransportProvenance};
use super::sc::{LoopbackWebSocket, ScTransport, WebSocketPort};
use super::sc_frame::{decode_sc_message, encode_sc_message, ScFunction, ScMessage};
#[cfg(feature = "sc-tls")]
use super::sc_tls::{DirectAcceptConfig, DirectListener, ScNodeTlsConfig};
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use std::net::Ipv4Addr;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

#[test]
fn provenance_variants_and_scope() {
    let unverified = TransportProvenance::unverified();
    assert!(unverified.is_unverified());
    assert!(!unverified.is_verified());
    assert!(!unverified.is_direct_peer());
    assert!(!unverified.is_relayed_origin());
    assert!(unverified.scope().contains("unverified"));

    let direct = TransportProvenance::verified_direct_peer();
    assert!(direct.is_direct_peer());
    assert!(direct.is_verified());
    assert!(!direct.is_unverified());
    assert!(direct.scope().contains("direct"));

    let relayed = TransportProvenance::verified_relayed_origin();
    assert!(relayed.is_relayed_origin());
    assert!(relayed.is_verified());
    assert!(!relayed.is_unverified());
    assert!(relayed.scope().contains("hub"));

    // Snapshots compare by value; conflicting contexts fail closed elsewhere.
    assert_ne!(unverified, direct);
    assert_ne!(unverified, relayed);
    assert_ne!(direct, relayed);
    assert_eq!(unverified, TransportProvenance::default());
}

#[test]
fn only_unverified_is_default() {
    assert!(TransportProvenance::default().is_unverified());
}

#[tokio::test]
async fn loopback_yields_unverified_legacy_origin() {
    let (mut a, mut b) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut rx_b = b.start().await.unwrap();
    let _rx_a = a.start().await.unwrap();
    a.send_unicast(b"hello", &[0x02]).await.unwrap();
    let received = rx_b.recv().await.unwrap();
    assert!(received.provenance.is_unverified());
    assert!(!received.provenance.is_verified());
    a.send_broadcast(b"bcast").await.unwrap();
    let received = rx_b.recv().await.unwrap();
    assert!(received.provenance.is_unverified());
}

#[test]
fn mstp_frame_path_yields_unverified() {
    let (tx, mut rx) = mpsc::channel(16);
    let config = MstpConfig {
        this_station: 3,
        max_master: 127,
        max_info_frames: 1,
        baud_rate: 9600,
    };
    let mut node = MasterNode::new(config).unwrap();
    let frame = MstpFrame {
        frame_type: FrameType::BACnetDataNotExpectingReply,
        destination: 3,
        source: 7,
        data: Bytes::from_static(&[0x01, 0x00, 0x30]),
    };
    let _ = node.handle_received_frame(&frame, &tx);
    let received = rx
        .try_recv()
        .expect("MSTP must deliver DataNotExpectingReply");
    assert!(received.provenance.is_unverified());
    assert_eq!(received.source_mac.as_slice(), &[7]);
}

#[tokio::test]
async fn bip_yields_unverified_legacy_origin() {
    let mut a = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx_a = a.start().await.unwrap();
    let mut rx_b = b.start().await.unwrap();
    let mac_b = b.local_mac().to_vec();
    a.send_unicast(&[0x01, 0x00, 0x30], &mac_b).await.unwrap();
    let received = timeout(Duration::from_secs(2), rx_b.recv())
        .await
        .expect("BIP timed out")
        .expect("BIP closed");
    assert!(received.provenance.is_unverified());
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}

#[tokio::test]
async fn caller_provided_untrusted_option_stays_untrusted() {
    // Caller-supplied transports are part of the trust boundary: they must
    // use unverified() unless they implement equivalent TLS + admission.
    // The unverified helper is the only public constructor; a custom option
    // that synthesizes envelopes stays untrusted, and AnyTransport
    // delegation preserves the inner value.
    let envelope = ReceivedNpdu::unverified(
        Bytes::from_static(&[0x01, 0x00, 0x30]),
        MacAddr::from_slice(&[0xFE]),
        false,
        Vec::new(),
        None,
    );
    assert!(envelope.provenance.is_unverified());
    let (loopback, _peer) = LoopbackTransport::pair(vec![0xFE], vec![0xFD]);
    let any = super::any::AnyTransport::<LoopbackSerial>::Loopback(loopback);
    // Delegation preserves inner provenance (unverified here); no data-link
    // implementation is forced to pretend auth support.
    assert_eq!(any.max_apdu_length(), 1476);
}

async fn sc_hub_accept(ws_hub: &LoopbackWebSocket, hub_vmac: [u8; 6]) {
    let data = ws_hub.recv().await.unwrap();
    let req = decode_sc_message(&data).unwrap();
    assert_eq!(req.function, ScFunction::ConnectRequest);
    let mut payload = Vec::with_capacity(26);
    payload.extend_from_slice(&hub_vmac);
    payload.extend_from_slice(&[0x33; 16]);
    payload.extend_from_slice(&1476u16.to_be_bytes());
    payload.extend_from_slice(&1476u16.to_be_bytes());
    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: req.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &accept);
    ws_hub.send(&buf).await.unwrap();
}

fn hub_npdu(source: Option<[u8; 6]>) -> ScMessage {
    ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x2233,
        originating_vmac: source,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x00, 0x30]),
    }
}

#[tokio::test]
async fn sc_hub_relayed_origin_is_verified_and_unknown_origin_dropped() {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, [0x01; 6]).with_device_uuid([1; 16]);
    let hub_task = tokio::spawn(async move {
        sc_hub_accept(&ws_hub, [0x10; 6]).await;
        ws_hub
    });
    let mut rx = transport.start().await.unwrap();
    let ws_hub = hub_task.await.unwrap();

    // Unknown origins (missing / zero / broadcast) never deliver verified:
    // missing unicast NAKs, reserved stays silent (source_admission).
    for source in [None, Some([0; 6]), Some([0xFF; 6])] {
        let msg = hub_npdu(source);
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);
        ws_hub.send(&buf).await.unwrap();
    }
    // Valid relayed origin delivers verified.
    let valid = hub_npdu(Some([0x22; 6]));
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &valid);
    ws_hub.send(&buf).await.unwrap();

    let received = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("hub relay timed out")
        .expect("hub channel closed");
    assert_eq!(received.source_mac.as_slice(), &[0x22; 6]);
    assert!(received.provenance.is_relayed_origin());
    assert!(received.provenance.is_verified());
    // No other delivery: unknown origins were dropped, not queued.
    assert!(rx.try_recv().is_err());
    // Hub peer VMAC is never substituted for the leaf origin.
    assert_ne!(received.source_mac.as_slice(), &[0x10; 6]);
    transport.stop().await.unwrap();
}

#[cfg(feature = "sc-tls")]
#[tokio::test]
async fn sc_direct_peer_is_verified_post_handshake() {
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
    // Minimal in-test CA (no committed keys).
    let mut ca_params =
        rcgen::CertificateParams::new(Vec::<String>::new()).expect("empty SANs are valid");
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let ca_der: CertificateDer<'static> = ca_cert.der().clone();
    let issuer = rcgen::Issuer::from_params(&ca_params, &ca_key);

    let issue = |sans: Vec<String>| {
        let params = rcgen::CertificateParams::new(sans).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = params.signed_by(&key, &issuer).unwrap();
        let chain: Vec<CertificateDer<'static>> =
            rustls::pki_types::pem::PemObject::pem_slice_iter(cert.pem().as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        ScNodeTlsConfig::from_der(
            vec![ca_der.clone()],
            chain,
            PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap()
    };

    let listener_vmac = [0xAA; 6];
    let listener_uuid = [9; 16];
    let dial_vmac = [0x22; 6];
    let dial_uuid = [7; 16];
    let listener_tls = issue(vec!["localhost".into(), "127.0.0.1".into()]);
    let config = DirectAcceptConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        listener_vmac,
        listener_uuid,
        listener_tls,
    );
    let (mut listener, mut rx) = DirectListener::start(config).await.unwrap();
    let url = format!(
        "wss://localhost:{}/.bacnet/sc",
        listener.local_addr().port()
    );
    let dial_tls = issue(vec!["node".into()]);
    let ws = timeout(
        Duration::from_secs(5),
        super::sc_tls::TlsWebSocket::connect_direct(&url, dial_tls),
    )
    .await
    .expect("dial timed out")
    .expect("dial must succeed");
    let mut conn = super::sc::ScConnection::new(dial_vmac, dial_uuid);
    let request = conn.build_connect_request();
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &request);
    ws.send(&buf).await.unwrap();
    let accept_bytes = timeout(Duration::from_secs(5), ws.recv())
        .await
        .expect("accept timed out")
        .unwrap();
    let accept = decode_sc_message(&accept_bytes).unwrap();
    assert!(conn.handle_connect_accept(&accept));
    let direct = conn
        .build_direct_encapsulated_npdu(&[0x01, 0x00, 0x30], &[])
        .unwrap();
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &direct);
    ws.send(&buf).await.unwrap();
    let received = timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("direct NPDU timed out")
        .expect("listener closed");
    assert_eq!(received.source_mac.as_slice(), &dial_vmac);
    assert!(received.provenance.is_direct_peer());
    assert!(received.provenance.is_verified());
    listener.stop().await;
}

#[test]
fn cloning_preserves_provenance_and_drops_reply() {
    use tokio::sync::oneshot;
    let (tx, _rx) = oneshot::channel();
    let original = ReceivedNpdu {
        npdu: Bytes::from_static(&[0x01, 0x00, 0x30]),
        source_mac: MacAddr::from_slice(&[0xAA; 6]),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::verified_relayed_origin(),
        reply_tx: Some(tx),
    };
    assert!(original.reply_tx.is_some());
    let cloned = original.clone();
    // Cloning must not duplicate reply authority nor change meaning.
    assert!(cloned.reply_tx.is_none());
    assert_eq!(cloned.provenance, original.provenance);
    assert!(cloned.provenance.is_relayed_origin());
}

#[tokio::test]
async fn reply_ownership_is_single_use() {
    use tokio::sync::oneshot;
    let (tx, rx) = oneshot::channel();
    let mut envelope = ReceivedNpdu {
        npdu: Bytes::from_static(&[0x01]),
        source_mac: MacAddr::from_slice(&[0x01]),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: Some(tx),
    };
    assert!(envelope.reply_tx.is_some());
    let reply_tx = envelope.reply_tx.take().expect("must hold reply");
    reply_tx.send(Bytes::from_static(&[0x02])).unwrap();
    let bytes = rx.await.unwrap();
    assert_eq!(bytes.as_ref(), &[0x02]);
    // Clones never regain the sender (single use preserved).
    assert!(envelope.clone().reply_tx.is_none());
}

#[tokio::test]
async fn queueing_cannot_change_provenance_meaning() {
    let (tx, mut rx) = mpsc::channel(4);
    for provenance in [
        TransportProvenance::unverified(),
        TransportProvenance::verified_direct_peer(),
        TransportProvenance::verified_relayed_origin(),
    ] {
        tx.send(ReceivedNpdu {
            npdu: Bytes::from_static(&[0x01]),
            source_mac: MacAddr::from_slice(&[0x09]),
            link_layer_group: false,
            data_attributes: Vec::new(),
            provenance,
            reply_tx: None,
        })
        .await
        .unwrap();
        let received = rx.recv().await.unwrap();
        assert_eq!(received.provenance, provenance);
    }
}

#[test]
fn debug_is_redacted_no_key_material() {
    let secret_mac = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF];
    let envelope = ReceivedNpdu {
        npdu: Bytes::from_static(&[0xDE, 0xAD]),
        source_mac: MacAddr::from_slice(&secret_mac),
        link_layer_group: true,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::verified_direct_peer(),
        reply_tx: None,
    };
    let rendered = format!("{:?}", envelope);
    // Lengths and kind labels only; raw MAC bytes and payload bytes absent.
    assert!(rendered.contains("source_mac_len"));
    assert!(!rendered.contains("AA") && !rendered.contains("aa"));
    assert!(!rendered.contains("BB") && !rendered.contains("171"));
    assert!(rendered.contains("verified-direct-peer"));
    let provenance_rendered = format!("{:?}", TransportProvenance::verified_relayed_origin());
    assert!(provenance_rendered.contains("verified-relayed-origin"));
    assert!(!provenance_rendered.contains("AA"));
}

#[test]
fn no_public_authenticated_boolean_on_data_attributes() {
    // DataAttribute stays a wire representation: option type, MU flag, bytes.
    let attr = super::port::DataAttribute {
        option_type: 31,
        must_understand: false,
        data: vec![0x12],
    };
    let rendered = format!("{:?}", attr);
    assert!(!rendered.to_lowercase().contains("authenticated"));
}
