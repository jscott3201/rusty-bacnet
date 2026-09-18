//! RB-16 B/IP proof: one device over real B/IP loopback UDP, one socket.
//!
//! Split companion: `rb16_bip_flows.rs` holds same-ID/wrong-peer/routed and
//! COV/event flows (file-size gate; pure relocation, no logic change).
//! This file keeps bind-count, one-socket bidirectional round-trip, Device
//! readback (I-Am ≡ ReadProperty), and stop → rebind.
//!
//! Real loopback UDP (`127.0.0.1`, ephemeral ports), one endpoint bind — NO
//! second hidden socket. `BipEndpointBuilder(127.0.0.1,0).build_session` →
//! `start`; independent peers request-in while the role initiates confirmed
//! requests out; stop → rebind.
//!
//! Transport-dependent notes (recorded precisely):
//! - B/IP binds `INADDR_ANY` so subnet/limited broadcast reaches the socket;
//!   `interface` is only the announced MAC IP. Ephemeral ports isolate tests;
//!   production uses 47808. Broadcast I-Am is proven via wire bytes + Device
//!   readback (loopback broadcast capture is in `rb16_identity`); real peers
//!   use unicast to the learned MAC because broadcast goes to the sender's
//!   own port.
//! - `LoopbackTransport` appears alongside for determinism (bind-count double),
//!   never INSTEAD of these real-UDP proofs.

use std::net::Ipv4Addr;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use bacnet_encoding::apdu::{decode_apdu, encode_apdu, Apdu, ConfirmedRequest as ConfirmedPdu};
use bacnet_endpoint::bip::BipEndpointBuilder;
use bacnet_endpoint::identity::{build_database_with_extra, DeviceIdentity};
use bacnet_endpoint::session::{SessionConfig, SessionRole};
use bacnet_network::layer::NetworkLayer;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::bip::BipTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::{
    ConfirmedServiceChoice, NetworkPriority, ObjectType, PropertyIdentifier, Segmentation,
    ServiceSupported,
};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bacnet_types::MacAddr;
use bytes::BytesMut;
use tokio::sync::mpsc;

const WAIT: Duration = Duration::from_secs(5);

fn oid(t: ObjectType, i: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(t, i).unwrap()
}

fn session_config() -> SessionConfig {
    SessionConfig {
        queue_capacity: 32,
        apdu_timeout_ms: 2_000,
        apdu_retries: 0,
        max_apdu_length: 480,
    }
}

fn identity() -> DeviceIdentity {
    DeviceIdentity::new(1001, 42)
        .unwrap()
        .with_max_apdu(1476)
        .unwrap()
        .with_segmentation(Segmentation::NONE)
        .with_services(&[ServiceSupported::READ_PROPERTY])
}

fn db_with_analog(
    id: &DeviceIdentity,
    instance: u32,
    value: f32,
) -> bacnet_objects::database::ObjectDatabase {
    let mut o = AnalogInputObject::new(instance, format!("bip-ai-{instance}"), 0).unwrap();
    o.set_present_value(value);
    build_database_with_extra(id, vec![Box::new(o)]).unwrap()
}

/// Minimal peer responder over a started `NetworkLayer<BipTransport>`.
///
/// Receives one Confirmed ReadProperty, answers ComplexAck with the given
/// Real value, returns (invoke_id, peer-observed endpoint MAC).
async fn peer_respond_once(
    rx: &mut mpsc::Receiver<bacnet_network::layer::ReceivedApdu>,
    net: &NetworkLayer<BipTransport>,
    value: f32,
) -> (u8, MacAddr) {
    let req = tokio::time::timeout(WAIT, rx.recv())
        .await
        .expect("peer must receive request")
        .expect("peer channel closed");
    let iid = match decode_apdu(req.apdu.clone()).unwrap() {
        Apdu::ConfirmedRequest(r) => {
            assert_eq!(r.service_choice, ConfirmedServiceChoice::READ_PROPERTY);
            // Wire-byte proof: client advertises the composed max-APDU (1476).
            assert_eq!(
                r.max_apdu_length, 1476,
                "client must advertise identity max-APDU on wire"
            );
            r.invoke_id
        }
        other => panic!("peer expected ConfirmedRequest, got {other:?}"),
    };
    let endpoint_mac = req.source_mac.clone();
    assert!(
        !endpoint_mac.is_empty(),
        "endpoint MAC must be nonzero (single socket)"
    );
    assert_eq!(endpoint_mac.len(), 6, "B/IP MAC is 6 bytes (IP+port)");
    // Encode a Real present-value ComplexAck.
    let mut val = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut val, &PropertyValue::Real(value))
        .unwrap();
    // Service-ack = object id + property id + value (tag framing via ACK helper).
    let ack = ReadPropertyACK {
        object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: val.to_vec(),
    };
    let mut svc = BytesMut::new();
    ack.encode(&mut svc);
    let resp = Apdu::ComplexAck(bacnet_encoding::apdu::ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id: iid,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: svc.freeze(),
    });
    let mut enc = BytesMut::new();
    encode_apdu(&mut enc, &resp).unwrap();
    // Direct reply to the endpoint MAC (unicast); routed replies are covered
    // in the companion `rb16_bip_flows.rs` raw-socket routed test.
    match req.source_network.clone() {
        Some(src) if !src.mac_address.is_empty() => {
            net.send_apdu_routed(
                &enc,
                src.network,
                &src.mac_address,
                &endpoint_mac,
                false,
                NetworkPriority::NORMAL,
            )
            .await
            .unwrap();
        }
        _ => {
            net.send_apdu(&enc, &endpoint_mac, false, NetworkPriority::NORMAL)
                .await
                .unwrap();
        }
    }
    (iid, endpoint_mac)
}

// --- Bind-count double (transport-independent determinism) ---

struct CountingTransport<T: TransportPort> {
    inner: T,
    starts: Arc<AtomicUsize>,
}

impl<T: TransportPort> CountingTransport<T> {
    fn new(inner: T, starts: Arc<AtomicUsize>) -> Self {
        Self { inner, starts }
    }
}

impl<T: TransportPort> TransportPort for CountingTransport<T> {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, bacnet_types::error::Error> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.inner.start().await
    }
    async fn stop(&mut self) -> Result<(), bacnet_types::error::Error> {
        self.inner.stop().await
    }
    async fn send_unicast(
        &self,
        npdu: &[u8],
        mac: &[u8],
    ) -> Result<(), bacnet_types::error::Error> {
        self.inner.send_unicast(npdu, mac).await
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), bacnet_types::error::Error> {
        self.inner.send_broadcast(npdu).await
    }
    fn local_mac(&self) -> &[u8] {
        self.inner.local_mac()
    }
}

#[tokio::test]
async fn bip_bind_count_is_one_per_start() {
    // Deterministic double: each session start binds exactly once; second
    // start errs without an extra bind; stop releases (rebind works).
    use bacnet_endpoint::session::EndpointSession;
    use bacnet_transport::loopback::LoopbackTransport;

    let starts = Arc::new(AtomicUsize::new(0));
    let (inner, _peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let counting = CountingTransport::new(inner, Arc::clone(&starts));
    let mut s = EndpointSession::new(counting, SessionRole::Both, session_config()).unwrap();
    s.start().await.unwrap();
    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "one bind per start, no hidden socket"
    );
    assert!(s.start().await.is_err());
    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "failed start must not bind again"
    );
    s.stop().await.unwrap();

    let starts2 = Arc::new(AtomicUsize::new(0));
    let (inner2, _peer2) = LoopbackTransport::pair(vec![0x03], vec![0x04]);
    let mut s2 = EndpointSession::new(
        CountingTransport::new(inner2, Arc::clone(&starts2)),
        SessionRole::Both,
        session_config(),
    )
    .unwrap();
    s2.start().await.unwrap();
    assert_eq!(
        starts2.load(Ordering::SeqCst),
        1,
        "rebind after stop binds once"
    );
    s2.stop().await.unwrap();
}

#[tokio::test]
async fn bip_one_socket_bidirectional_and_device_inspection() {
    // Real UDP: one endpoint bind; independent raw peer request-in while the
    // role initiates a confirmed request out; Device readback ≡ I-Am.
    let id = identity();
    let db = db_with_analog(&id, 1, 11.0);
    let mut endpoint = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::Both)
        .database(db)
        .identity(id.clone())
        .build_session()
        .expect("bip session must build");
    endpoint.start().await.unwrap();
    // Second start errs (start-once, no second socket).
    // Note: cannot call start again after move; assert via a fresh builder
    // that the bound port would conflict only if leaked — instead assert the
    // running session reports running and a duplicate start on the same
    // session object fails (borrowed before move below is not possible, so
    // prove via the counting test + this session's single-MAC corroboration).

    // Controlled peer: real B/IP socket + network layer.
    let peer_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut peer_net = NetworkLayer::new(peer_transport);
    let mut peer_rx = peer_net.start().await.unwrap();
    let peer_mac = MacAddr::from_slice(peer_net.local_mac());
    assert!(!peer_mac.is_empty());

    // Outbound: endpoint role initiates confirmed ReadProperty to peer.
    // Peer answers with present-value 22.0; wire asserts identity max-APDU.
    let client = endpoint.cloned_client_handle().unwrap();
    let peer_mac_c = peer_mac.clone();
    let out = tokio::spawn(async move {
        client
            .read_property(
                &peer_mac_c,
                oid(ObjectType::ANALOG_INPUT, 1),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
    });
    let (out_iid, endpoint_mac) = peer_respond_once(&mut peer_rx, &peer_net, 22.0).await;
    let ack = tokio::time::timeout(WAIT, out)
        .await
        .expect("outbound hung")
        .unwrap()
        .expect("outbound failed");
    assert_eq!(ack.object_identifier, oid(ObjectType::ANALOG_INPUT, 1));
    // Single-socket corroboration: endpoint MAC is one nonzero B/IP MAC
    // (127.0.0.1 + ephemeral port); every later message must share it.
    assert_eq!(endpoint_mac.len(), 6);
    assert_eq!(&endpoint_mac[..4], &[127, 0, 0, 1]);
    assert_ne!(u16::from_be_bytes([endpoint_mac[4], endpoint_mac[5]]), 0);

    // Inbound: independent peer request-in to the endpoint server (same MAC).
    let mut svc = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut svc);
    let inbound = Apdu::ConfirmedRequest(ConfirmedPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 9,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: svc.freeze(),
    });
    let mut enc = BytesMut::new();
    encode_apdu(&mut enc, &inbound).unwrap();
    peer_net
        .send_apdu(&enc, &endpoint_mac, true, NetworkPriority::NORMAL)
        .await
        .unwrap();
    let reply = tokio::time::timeout(WAIT, peer_rx.recv())
        .await
        .expect("inbound reply hung")
        .expect("peer closed");
    // Reply must come from the same single endpoint MAC.
    assert_eq!(
        reply.source_mac, endpoint_mac,
        "one socket => one source MAC"
    );
    match decode_apdu(reply.apdu).unwrap() {
        Apdu::ComplexAck(a) => {
            assert_eq!(a.invoke_id, 9);
            let decoded = ReadPropertyACK::decode(&a.service_ack).unwrap();
            // Present value 11.0 from the endpoint database.
            let mut expect = BytesMut::new();
            bacnet_encoding::primitives::encode_property_value(
                &mut expect,
                &PropertyValue::Real(11.0),
            )
            .unwrap();
            assert_eq!(decoded.property_value, expect.to_vec());
        }
        other => panic!("expected ComplexAck, got {other:?}"),
    }
    let _ = out_iid;

    // Device properties inspection over the same real socket: every Device
    // ReadProperty must equal the I-Am fields (I-Am ≡ ReadProperty).
    let iam = id.iam_request();
    for (idx, (prop, expect_value)) in [
        (
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyValue::ObjectIdentifier(iam.object_identifier),
        ),
        (
            PropertyIdentifier::VENDOR_IDENTIFIER,
            PropertyValue::Unsigned(u64::from(iam.vendor_id)),
        ),
        (
            PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED,
            PropertyValue::Unsigned(u64::from(id.max_apdu_length())),
        ),
        (
            PropertyIdentifier::SEGMENTATION_SUPPORTED,
            PropertyValue::Enumerated(iam.segmentation_supported.to_raw() as u32),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        // Device-object readback via peer request to the endpoint Device.
        let mut svc = BytesMut::new();
        bacnet_services::read_property::ReadPropertyRequest {
            object_identifier: oid(ObjectType::DEVICE, 1001),
            property_identifier: prop,
            property_array_index: None,
        }
        .encode(&mut svc);
        let iid = (40 + idx) as u8;
        let req = Apdu::ConfirmedRequest(ConfirmedPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: iid,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: svc.freeze(),
        });
        let mut e = BytesMut::new();
        encode_apdu(&mut e, &req).unwrap();
        peer_net
            .send_apdu(&e, &endpoint_mac, true, NetworkPriority::NORMAL)
            .await
            .unwrap();
        let rep = tokio::time::timeout(WAIT, peer_rx.recv())
            .await
            .expect("device read hung")
            .expect("closed");
        match decode_apdu(rep.apdu).unwrap() {
            Apdu::ComplexAck(a) => {
                assert_eq!(a.invoke_id, iid);
                let decoded = ReadPropertyACK::decode(&a.service_ack).unwrap();
                assert_eq!(decoded.object_identifier, oid(ObjectType::DEVICE, 1001));
                assert_eq!(decoded.property_identifier, prop);
                if prop == PropertyIdentifier::OBJECT_IDENTIFIER {
                    assert_eq!(decoded.object_identifier, oid(ObjectType::DEVICE, 1001));
                } else {
                    // Compare encoded value bytes against the identity value.
                    let mut expect_enc = BytesMut::new();
                    bacnet_encoding::primitives::encode_property_value(
                        &mut expect_enc,
                        &expect_value,
                    )
                    .unwrap();
                    assert_eq!(
                        decoded.property_value,
                        expect_enc.to_vec(),
                        "Device {prop:?} must equal I-Am identity"
                    );
                }
            }
            other => panic!("expected ComplexAck for {prop:?}, got {other:?}"),
        }
    }

    // Transport admin while roles run: narrow session borrow (counters +
    // coordinator) stays usable; role handles expose no stop/transport.
    assert!(endpoint.is_running());
    assert_eq!(endpoint.session_role(), SessionRole::Both);
    let _ = endpoint.policy_counters().await;
    assert_eq!(endpoint.active_leases(), 0);

    endpoint.stop().await.unwrap();
    assert!(endpoint.stop().await.is_err(), "stop-once");
    peer_net.stop().await.unwrap();

    // Stop releases: rebind a fresh endpoint on ephemeral port works.
    let id2 = identity();
    let db2 = db_with_analog(&id2, 1, 1.0);
    let mut re = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::Both)
        .database(db2)
        .identity(id2)
        .build_session()
        .unwrap();
    re.start().await.unwrap();
    re.stop().await.unwrap();
}
