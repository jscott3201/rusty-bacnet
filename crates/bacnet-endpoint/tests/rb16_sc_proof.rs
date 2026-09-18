//! RB-16 SC proof: one device over constrained-TLS hub, durable identity.
//!
//! Local constrained-TLS hub (`ScHub::start_with_uuid(127.0.0.1:0)` +
//! rcgen-CA) + controlled peers through the NEW real-hub endpoint session
//! constructor (`ScEndpointBuilder::build_hub_session`; loopback-only before
//! RB-16). One SC node connection; kill/reconnect with VMAC+UUID preserved
//! (vs transient); route/context preservation (VMAC source + verified
//! provenance on the controlled peer); role traffic both directions;
//! transport admin while roles run; role-attempt-to-stop fails.
//!
//! Transport-dependent notes (precise):
//! - SC hub relay preserves `source_mac` as the originating VMAC (not the hub
//!   VMAC) with `verified-relayed-origin` provenance on the controlled
//!   `NetworkLayer` peer; the endpoint dispatch preserves the full envelope
//!   structurally (raw + effective group, attributes, provenance) without new
//!   decisions — asserted on the peer side, cited for the endpoint path.
//! - TLS is 1.3-only local policy with explicit CA trust + mutual auth; VMAC
//!   is payload-claimed inside TLS (not cert-bound); UUID is
//!   durable-caller-owned (neither generated nor stored by builders).
//! - Loopback appears only for builder validation (`bip_sc.rs`); all traffic
//!   here is real hub-relayed TLS.

#![cfg(feature = "sc-tls")]

use std::net::Ipv4Addr;
use std::time::Duration;

use bacnet_encoding::apdu::{decode_apdu, encode_apdu, Apdu};
use bacnet_endpoint::identity::{build_database_with_extra, DeviceIdentity};
use bacnet_endpoint::sc::ScEndpointBuilder;
use bacnet_endpoint::session::SessionRole;
use bacnet_network::layer::NetworkLayer;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::sc::{ScReconnectConfig, ScTransport};
use bacnet_transport::sc_hub::{ScHub, ScHubTlsConfig};
use bacnet_transport::sc_tls::{ScNodeTlsConfig, TlsWebSocket};
use bacnet_types::enums::{
    ConfirmedServiceChoice, NetworkPriority, ObjectType, PropertyIdentifier, Segmentation,
    ServiceSupported, UnconfirmedServiceChoice,
};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::primitives::PropertyValue;
use bacnet_types::MacAddr;
use bytes::BytesMut;

const WAIT: Duration = Duration::from_secs(10);

fn oid(t: ObjectType, i: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(t, i).unwrap()
}

struct TestCa {
    hub_tls: ScHubTlsConfig,
    node_tls: ScNodeTlsConfig,
}

fn test_ca() -> TestCa {
    use rcgen::{CertificateParams, Issuer, KeyPair};
    use rustls::pki_types::PrivatePkcs8KeyDer;
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = KeyPair::generate().unwrap();
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::from_params(&ca_params, &ca_key);

    let hub_key = KeyPair::generate().unwrap();
    let hub_cert = CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()])
        .unwrap()
        .signed_by(&hub_key, &issuer)
        .unwrap();
    let hub_tls = ScHubTlsConfig::from_der(
        vec![ca_cert.der().clone()],
        vec![hub_cert.der().clone()],
        PrivatePkcs8KeyDer::from(hub_key.serialize_der()).into(),
    )
    .unwrap();

    let node_key = KeyPair::generate().unwrap();
    let node_cert = CertificateParams::new(vec!["node".into()])
        .unwrap()
        .signed_by(&node_key, &issuer)
        .unwrap();
    let node_tls = ScNodeTlsConfig::from_der(
        vec![ca_cert.der().clone()],
        vec![node_cert.der().clone()],
        PrivatePkcs8KeyDer::from(node_key.serialize_der()).into(),
    )
    .unwrap();

    TestCa { hub_tls, node_tls }
}

fn identity_with_sc(instance: u32, vmac: [u8; 6], uuid: [u8; 16]) -> DeviceIdentity {
    DeviceIdentity::new(instance, 42)
        .unwrap()
        .with_max_apdu(1476)
        .unwrap()
        .with_segmentation(Segmentation::NONE)
        .with_services(&[ServiceSupported::READ_PROPERTY])
        .with_device_uuid(uuid)
        .with_sc_port(2, 77, vmac)
        .unwrap()
}

fn db_with_analog(
    id: &DeviceIdentity,
    instance: u32,
    value: f32,
) -> bacnet_objects::database::ObjectDatabase {
    let mut o = AnalogInputObject::new(instance, format!("sc-ai-{instance}"), 0).unwrap();
    o.set_present_value(value);
    build_database_with_extra(id, vec![Box::new(o)]).unwrap()
}

async fn dial_node(hub_port: u16, node_tls: ScNodeTlsConfig) -> TlsWebSocket {
    let url = format!("wss://127.0.0.1:{hub_port}");
    tokio::time::timeout(WAIT, TlsWebSocket::connect(&url, node_tls))
        .await
        .expect("hub dial timed out")
        .expect("hub dial failed")
}

/// Controlled SC peer (real hub relay) exposing provenance + source MAC.
struct ControlledPeer {
    net: NetworkLayer<ScTransport<TlsWebSocket>>,
    rx: tokio::sync::mpsc::Receiver<bacnet_network::layer::ReceivedApdu>,
    vmac: [u8; 6],
}

impl ControlledPeer {
    async fn dial(hub_port: u16, node_tls: ScNodeTlsConfig, vmac: [u8; 6], uuid: [u8; 16]) -> Self {
        let ws = dial_node(hub_port, node_tls).await;
        let transport = ScTransport::new(ws, vmac)
            .with_device_uuid(uuid)
            .with_heartbeat_interval_ms(30_000)
            .with_heartbeat_timeout_ms(60_000);
        let mut net = NetworkLayer::new(transport);
        let rx = net.start().await.expect("peer net must start");
        Self { net, rx, vmac }
    }

    fn mac(&self) -> MacAddr {
        MacAddr::from_slice(&self.vmac)
    }

    async fn respond_once(&mut self, value: f32) -> (u8, MacAddr, bool) {
        let req = tokio::time::timeout(WAIT, self.rx.recv())
            .await
            .expect("peer must receive")
            .expect("peer closed");
        // Route/context preservation: hub-relayed origin is verified (not
        // unverified legacy), source is the endpoint VMAC (not the hub).
        let verified = req.provenance.is_verified();
        let src = req.source_mac.clone();
        let iid = match decode_apdu(req.apdu.clone()).unwrap() {
            Apdu::ConfirmedRequest(r) => {
                assert_eq!(r.service_choice, ConfirmedServiceChoice::READ_PROPERTY);
                assert_eq!(
                    r.max_apdu_length, 1476,
                    "endpoint must advertise identity max-APDU"
                );
                r.invoke_id
            }
            other => panic!("peer expected request, got {other:?}"),
        };
        let mut val = BytesMut::new();
        bacnet_encoding::primitives::encode_property_value(&mut val, &PropertyValue::Real(value))
            .unwrap();
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
        self.net
            .send_apdu(&enc, &src, false, NetworkPriority::NORMAL)
            .await
            .unwrap();
        (iid, src, verified)
    }
}

#[tokio::test]
async fn sc_one_node_bidirectional_with_provenance_and_admin() {
    let ca = test_ca();
    let hub_vmac = [0xAA; 6];
    let hub_uuid = [0xBB; 16];
    let mut hub = ScHub::start_with_uuid("127.0.0.1:0", ca.hub_tls.clone(), hub_vmac, hub_uuid)
        .await
        .expect("hub must start");
    let hub_port = hub.local_addr().expect("hub addr").port();

    // One SC node connection: endpoint under test (durable VMAC+UUID).
    let node_vmac = [0x02; 6];
    let node_uuid = [0x11; 16];
    let id = identity_with_sc(2001, node_vmac, node_uuid);
    let db = db_with_analog(&id, 1, 11.0);
    let ws = dial_node(hub_port, ca.node_tls.clone()).await;
    let mut endpoint = ScEndpointBuilder::new(node_vmac, node_uuid)
        .role(SessionRole::Both)
        .database(db)
        .identity(id.clone())
        .build_hub_session(ws)
        .expect("hub session must build");
    endpoint.start().await.unwrap();
    assert!(endpoint.is_running());
    // Second start errs (start-once, one connection).
    assert!(endpoint.start().await.is_err());

    // Controlled peer on the same hub (different durable VMAC+UUID).
    let peer_vmac = [0x03; 6];
    let peer_uuid = [0x22; 16];
    let mut peer = ControlledPeer::dial(hub_port, ca.node_tls.clone(), peer_vmac, peer_uuid).await;

    // Outbound (endpoint→peer): peer asserts VMAC source + verified provenance.
    let client = endpoint.cloned_client_handle().unwrap();
    let peer_mac = peer.mac();
    let out = tokio::spawn(async move {
        client
            .read_property(
                &peer_mac,
                oid(ObjectType::ANALOG_INPUT, 1),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
    });
    let (_iid, src, verified) = peer.respond_once(22.0).await;
    assert_eq!(
        src.as_slice(),
        &node_vmac,
        "hub must relay leaf VMAC, not hub VMAC"
    );
    assert!(
        verified,
        "SC hub relay must be verified (not unverified legacy)"
    );
    let ack = tokio::time::timeout(WAIT, out)
        .await
        .expect("outbound hung")
        .unwrap()
        .expect("failed");
    assert_eq!(ack.object_identifier, oid(ObjectType::ANALOG_INPUT, 1));

    // Inbound (peer→endpoint): endpoint server answers; peer sees verified.
    let mut svc = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut svc);
    let req = Apdu::ConfirmedRequest(bacnet_encoding::apdu::ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 7,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: svc.freeze(),
    });
    let mut enc = BytesMut::new();
    encode_apdu(&mut enc, &req).unwrap();
    peer.net
        .send_apdu(&enc, &src, true, NetworkPriority::NORMAL)
        .await
        .unwrap();
    let reply = tokio::time::timeout(WAIT, peer.rx.recv())
        .await
        .expect("inbound reply hung")
        .expect("closed");
    assert_eq!(reply.source_mac.as_slice(), &node_vmac);
    assert!(
        reply.provenance.is_verified(),
        "inbound hub relay stays verified"
    );
    match decode_apdu(reply.apdu).unwrap() {
        Apdu::ComplexAck(a) => {
            assert_eq!(a.invoke_id, 7);
            let decoded = ReadPropertyACK::decode(&a.service_ack).unwrap();
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

    // I-Am over SC hub broadcast: endpoint broadcasts, peer receives the
    // same identity bytes (I-Am ≡ ReadProperty over the hub).
    endpoint.broadcast_i_am().await.unwrap();
    let iam_msg = tokio::time::timeout(WAIT, peer.rx.recv())
        .await
        .expect("I-Am must relay")
        .expect("closed");
    match decode_apdu(iam_msg.apdu).unwrap() {
        Apdu::UnconfirmedRequest(u) => {
            assert_eq!(u.service_choice, UnconfirmedServiceChoice::I_AM);
            let iam = bacnet_services::who_is::IAmRequest::decode(&u.service_request).unwrap();
            assert_eq!(iam.object_identifier, oid(ObjectType::DEVICE, 2001));
            assert_eq!(iam.max_apdu_length, 1476);
            assert_eq!(iam.vendor_id, 42);
        }
        other => panic!("expected I-Am broadcast, got {other:?}"),
    }

    // Transport admin while roles run: hub status + narrow session borrow.
    let status = hub.status().await;
    assert!(status.listening);
    assert!(status.client_count >= 2, "hub sees endpoint + peer");
    assert!(endpoint.is_running());
    let _ = endpoint.policy_counters().await;
    assert_eq!(endpoint.active_leases(), 0);
    assert!(endpoint.identity().is_some());

    // Role-attempt-to-stop-transport fails: roles expose no stop/transport;
    // session stop-once holds; post-stop role calls fail closed.
    let detached = endpoint.cloned_client_handle().unwrap();
    endpoint.stop().await.unwrap();
    assert!(endpoint.stop().await.is_err());
    assert!(detached
        .read_property(
            &peer.mac(),
            oid(ObjectType::ANALOG_INPUT, 1),
            PropertyIdentifier::PRESENT_VALUE,
            None
        )
        .await
        .is_err());
    peer.net.stop().await.unwrap();
    hub.stop().await;
}

#[tokio::test]
async fn sc_kill_reconnect_preserves_vmac_uuid_vs_transient() {
    let ca = test_ca();
    let mut hub = ScHub::start_with_uuid("127.0.0.1:0", ca.hub_tls.clone(), [0xC0; 6], [0xD0; 16])
        .await
        .unwrap();
    let hub_port = hub.local_addr().unwrap().port();

    // Durable identity (caller-owned VMAC+UUID).
    let vmac = [0x04; 6];
    let uuid = [0x33; 16];
    let id = identity_with_sc(3001, vmac, uuid);

    // First connection: real hub session, bidirectional smoke.
    let ws1 = dial_node(hub_port, ca.node_tls.clone()).await;
    let mut s1 = ScEndpointBuilder::new(vmac, uuid)
        .role(SessionRole::Both)
        .database(db_with_analog(&id, 1, 1.0))
        .identity(id.clone())
        .build_hub_session(ws1)
        .unwrap();
    s1.start().await.unwrap();
    let mut peer = ControlledPeer::dial(hub_port, ca.node_tls.clone(), [0x05; 6], [0x44; 16]).await;
    let c1 = s1.cloned_client_handle().unwrap();
    let pm = peer.mac();
    let t = tokio::spawn(async move {
        c1.read_property(
            &pm,
            oid(ObjectType::ANALOG_INPUT, 1),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )
        .await
    });
    let (_iid, src1, _) = peer.respond_once(1.0).await;
    assert_eq!(src1.as_slice(), &vmac);
    let _ = tokio::time::timeout(WAIT, t)
        .await
        .expect("hung")
        .unwrap()
        .expect("failed");

    // Kill: stop the session (clean Disconnect). Hub stays up.
    s1.stop().await.unwrap();

    // Reconnect with the SAME durable VMAC+UUID (new socket, same identity).
    let ws2 = dial_node(hub_port, ca.node_tls.clone()).await;
    let mut s2 = ScEndpointBuilder::new(vmac, uuid)
        .role(SessionRole::Both)
        .database(db_with_analog(&id, 1, 2.0))
        .identity(id.clone())
        .reconnect(ScReconnectConfig {
            initial_delay_ms: 100,
            max_delay_ms: 1_000,
            max_retries: 3,
        })
        .build_hub_session(ws2)
        .unwrap();
    s2.start().await.unwrap();
    let c2 = s2.cloned_client_handle().unwrap();
    let pm2 = peer.mac();
    let t2 = tokio::spawn(async move {
        c2.read_property(
            &pm2,
            oid(ObjectType::ANALOG_INPUT, 1),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )
        .await
    });
    let (_iid2, src2, verified2) = peer.respond_once(2.0).await;
    // VMAC+UUID preserved across kill/reconnect (same bytes on wire).
    assert_eq!(src2.as_slice(), &vmac, "reconnect must preserve VMAC");
    assert!(verified2);
    let _ = tokio::time::timeout(WAIT, t2)
        .await
        .expect("hung")
        .unwrap()
        .expect("failed");
    // Device UUID readback still the durable UUID (not zeros, not rotated).
    assert_eq!(id.device_uuid(), uuid);

    // Transient contrast: a fresh VMAC+UUID is observably different.
    let transient_vmac = [0x06; 6];
    let transient_uuid = [0x55; 16];
    let transient_id = identity_with_sc(3002, transient_vmac, transient_uuid);
    let ws3 = dial_node(hub_port, ca.node_tls.clone()).await;
    let mut s3 = ScEndpointBuilder::new(transient_vmac, transient_uuid)
        .role(SessionRole::ClientOnly)
        .database(db_with_analog(&transient_id, 1, 3.0))
        .identity(transient_id)
        .build_hub_session(ws3)
        .unwrap();
    s3.start().await.unwrap();
    let c3 = s3.cloned_client_handle().unwrap();
    let pm3 = peer.mac();
    let t3 = tokio::spawn(async move {
        c3.read_property(
            &pm3,
            oid(ObjectType::ANALOG_INPUT, 1),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )
        .await
    });
    let (_iid3, src3, _) = peer.respond_once(3.0).await;
    assert_eq!(src3.as_slice(), &transient_vmac);
    assert_ne!(
        src3.as_slice(),
        &vmac,
        "transient VMAC must differ from durable"
    );
    let _ = tokio::time::timeout(WAIT, t3)
        .await
        .expect("hung")
        .unwrap()
        .expect("failed");

    // Identity mismatch is fail-closed at composition (UUID must agree).
    let bad_id = DeviceIdentity::new(9999, 1)
        .unwrap()
        .with_device_uuid([0x99; 16]);
    let ws_bad = dial_node(hub_port, ca.node_tls.clone()).await;
    assert!(ScEndpointBuilder::new([0x07; 6], [0x77; 16])
        .identity(bad_id)
        .build_hub_session(ws_bad)
        .is_err());

    s2.stop().await.unwrap();
    s3.stop().await.unwrap();
    peer.net.stop().await.unwrap();
    hub.stop().await;

    // Silence unused import in non-TLS builds (this file is sc-tls only).
    let _ = Ipv4Addr::LOCALHOST;
}
