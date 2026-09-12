//! Comprehensive tests for discovery rate limiting, duplicate suppression,
//! and directed responses (Issue #534).

use std::sync::{Arc as StdArc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

use bacnet_encoding::apdu::{
    decode_apdu, encode_apdu, Apdu, ConfirmedRequest as ConfirmedRequestPdu,
    UnconfirmedRequest as UnconfirmedRequestPdu,
};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_services::who_has::{WhoHasObject, WhoHasRequest};
use bacnet_services::who_is::WhoIsRequest;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::{
    ConfirmedServiceChoice, NetworkPriority, ObjectType, PropertyIdentifier,
    UnconfirmedServiceChoice,
};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;

use super::*;

type CapturedUnicasts = StdArc<StdMutex<Vec<(Vec<u8>, Bytes)>>>;

#[derive(Clone)]
struct MockDiscoveryTransport {
    inbound_rx: StdArc<StdMutex<Option<mpsc::Receiver<ReceivedNpdu>>>>,
    unicasts: CapturedUnicasts,
    broadcasts: StdArc<StdMutex<Vec<Bytes>>>,
}

impl MockDiscoveryTransport {
    fn new() -> (Self, mpsc::Sender<ReceivedNpdu>) {
        let (tx, rx) = mpsc::channel(256);
        let transport = Self {
            inbound_rx: StdArc::new(StdMutex::new(Some(rx))),
            unicasts: StdArc::new(StdMutex::new(Vec::new())),
            broadcasts: StdArc::new(StdMutex::new(Vec::new())),
        };
        (transport, tx)
    }

    fn unicast_count(&self) -> usize {
        self.unicasts.lock().unwrap().len()
    }

    fn broadcast_count(&self) -> usize {
        self.broadcasts.lock().unwrap().len()
    }
}

impl TransportPort for MockDiscoveryTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.inbound_rx
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| Error::Encoding("MockDiscoveryTransport started twice".into()))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.unicasts
            .lock()
            .unwrap()
            .push((mac.to_vec(), Bytes::copy_from_slice(npdu)));
        Ok(())
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.broadcasts
            .lock()
            .unwrap()
            .push(Bytes::copy_from_slice(npdu));
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &[0x0A, 0x00, 0x00, 0x01]
    }
}

fn wrap_apdu(apdu: Bytes, source_mac: &[u8], routed: Option<(u16, &[u8])>) -> ReceivedNpdu {
    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: false,
        priority: NetworkPriority::NORMAL,
        destination: None,
        source: routed.map(|(net, mac)| NpduAddress {
            network: net,
            mac_address: MacAddr::from_slice(mac),
        }),
        hop_count: 255,
        payload: apdu,
        ..Npdu::default()
    };
    let mut raw_buf = BytesMut::new();
    encode_npdu(&mut raw_buf, &npdu).unwrap();
    ReceivedNpdu {
        npdu: raw_buf.freeze(),
        source_mac: MacAddr::from_slice(source_mac),
        link_layer_group: false,
        data_attributes: Vec::new(),
        reply_tx: None,
    }
}

fn build_who_is_npdu(
    low: Option<u32>,
    high: Option<u32>,
    source_mac: &[u8],
    routed: Option<(u16, &[u8])>,
) -> ReceivedNpdu {
    let mut apdu_buf = BytesMut::new();
    let who_is = WhoIsRequest {
        low_limit: low,
        high_limit: high,
    };
    let mut service_buf = BytesMut::new();
    who_is.encode(&mut service_buf);
    let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
        service_choice: UnconfirmedServiceChoice::WHO_IS,
        service_request: service_buf.freeze(),
    });
    encode_apdu(&mut apdu_buf, &pdu).unwrap();
    wrap_apdu(apdu_buf.freeze(), source_mac, routed)
}

fn build_who_has_npdu(
    object: WhoHasObject,
    low: Option<u32>,
    high: Option<u32>,
    source_mac: &[u8],
    routed: Option<(u16, &[u8])>,
) -> ReceivedNpdu {
    let who_has = WhoHasRequest {
        low_limit: low,
        high_limit: high,
        object,
    };
    let mut service_buf = BytesMut::new();
    who_has.encode(&mut service_buf).unwrap();
    let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
        service_choice: UnconfirmedServiceChoice::WHO_HAS,
        service_request: service_buf.freeze(),
    });
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &pdu).unwrap();
    wrap_apdu(apdu_buf.freeze(), source_mac, routed)
}

async fn spawn_test_server(
    policy: DiscoveryPolicy,
) -> (
    BACnetServer<MockDiscoveryTransport>,
    MockDiscoveryTransport,
    mpsc::Sender<ReceivedNpdu>,
) {
    let (transport, tx) = MockDiscoveryTransport::new();
    let transport_clone = transport.clone();

    let mut db = ObjectDatabase::new();
    let dev = DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "TestDevice".into(),
        vendor_id: 42,
        ..Default::default()
    })
    .unwrap();
    db.add(Box::new(dev)).unwrap();

    let ai = AnalogInputObject::new(1, "Zone Temp", 62).unwrap();
    db.add(Box::new(ai)).unwrap();

    let server = BACnetServer::generic_builder()
        .database(db)
        .transport(transport)
        .discovery_policy(policy)
        .build()
        .await
        .unwrap();

    (server, transport_clone, tx)
}

#[tokio::test]
async fn test_controlled_burst_from_one_source_throttled() {
    let policy = DiscoveryPolicy {
        max_responses_per_sec_per_source: 4,
        source_burst_capacity: 4,
        max_responses_per_sec_global: 64,
        global_burst_capacity: 64,
        coalesce_window: Duration::ZERO,
        prefer_directed_responses: true,
        ..Default::default()
    };
    let (mut server, transport, tx) = spawn_test_server(policy).await;
    let src = &[0x0A, 0x00, 0x00, 0x02];

    for _ in 0..10 {
        tx.send(build_who_is_npdu(None, None, src, None))
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    let c = server.discovery_counters();
    assert_eq!(c.who_is_received, 10);
    assert_eq!(c.i_am_sent, 4);
    assert_eq!(c.responses_throttled_source, 6);
    assert_eq!(c.responses_throttled_global, 0);
    assert_eq!(transport.unicast_count(), 4);

    server.stop().await.unwrap();
}

#[tokio::test]
async fn test_source_fairness_while_first_source_throttled() {
    let policy = DiscoveryPolicy {
        max_responses_per_sec_per_source: 3,
        source_burst_capacity: 3,
        max_responses_per_sec_global: 64,
        global_burst_capacity: 64,
        coalesce_window: Duration::ZERO,
        prefer_directed_responses: true,
        ..Default::default()
    };
    let (mut server, _transport, tx) = spawn_test_server(policy).await;
    let src1 = &[0x0A, 0x00, 0x00, 0x02];
    let src2 = &[0x0A, 0x00, 0x00, 0x03];

    for _ in 0..6 {
        tx.send(build_who_is_npdu(None, None, src1, None))
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(30)).await;

    for _ in 0..2 {
        tx.send(build_who_is_npdu(None, None, src2, None))
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(30)).await;

    let c = server.discovery_counters();
    assert_eq!(c.who_is_received, 8);
    assert_eq!(c.i_am_sent, 5); // 3 from src1, 2 from src2
    assert_eq!(c.responses_throttled_source, 3);
    assert_eq!(c.responses_throttled_global, 0);

    server.stop().await.unwrap();
}

#[tokio::test]
async fn test_reserved_capacity_preserved_under_global_load() {
    let reserved_mac = MacAddr::from_slice(&[0x0A, 0x00, 0x00, 0x99]);
    let policy = DiscoveryPolicy {
        global_burst_capacity: 5,
        max_responses_per_sec_global: 5,
        reserved_capacity: 2,
        reserved_sources: vec![reserved_mac.clone()],
        source_burst_capacity: 10,
        max_responses_per_sec_per_source: 10,
        coalesce_window: Duration::ZERO,
        prefer_directed_responses: true,
        ..Default::default()
    };
    let (mut server, _transport, tx) = spawn_test_server(policy).await;
    let unreserved = &[0x0A, 0x00, 0x00, 0x02];

    for _ in 0..5 {
        tx.send(build_who_is_npdu(None, None, unreserved, None))
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(30)).await;

    let c = server.discovery_counters();
    assert_eq!(c.i_am_sent, 3); // 5 - 2 reserved = 3
    assert_eq!(c.responses_throttled_global, 2);

    // Reserved source can consume from the reserved 2 tokens
    for _ in 0..2 {
        tx.send(build_who_is_npdu(None, None, reserved_mac.as_slice(), None))
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(30)).await;

    let c = server.discovery_counters();
    assert_eq!(c.i_am_sent, 5); // 3 + 2

    // Unreserved source is still throttled
    tx.send(build_who_is_npdu(None, None, unreserved, None))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    let c = server.discovery_counters();
    assert_eq!(c.responses_throttled_global, 3);

    server.stop().await.unwrap();
}

#[tokio::test]
async fn test_duplicate_scans_coalesced_within_window() {
    let policy = DiscoveryPolicy {
        coalesce_window: Duration::from_millis(200),
        prefer_directed_responses: true,
        ..Default::default()
    };
    let (mut server, _transport, tx) = spawn_test_server(policy).await;
    let src = &[0x0A, 0x00, 0x00, 0x02];

    // First Who-Is -> sent
    tx.send(build_who_is_npdu(None, None, src, None))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Duplicate Who-Is within 200ms -> coalesced
    tx.send(build_who_is_npdu(None, None, src, None))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;

    let c = server.discovery_counters();
    assert_eq!(c.who_is_received, 2);
    assert_eq!(c.i_am_sent, 1);
    assert_eq!(c.requests_coalesced, 1);

    // Who-Has for existing object by Name -> sent
    let who_has_name = WhoHasObject::Name("Zone Temp".into());
    tx.send(build_who_has_npdu(
        who_has_name.clone(),
        None,
        None,
        src,
        None,
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Duplicate Who-Has -> coalesced
    tx.send(build_who_has_npdu(who_has_name, None, None, src, None))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;

    let c = server.discovery_counters();
    assert_eq!(c.who_has_received, 2);
    assert_eq!(c.i_have_sent, 1);
    assert_eq!(c.requests_coalesced, 2);

    // Who-Has for non-existent object -> negative cache recorded
    let missing = WhoHasObject::Name("Missing Sensor".into());
    tx.send(build_who_has_npdu(missing.clone(), None, None, src, None))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Repeated query for non-existent object -> negative cache hit coalesced!
    tx.send(build_who_has_npdu(missing, None, None, src, None))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;

    let c = server.discovery_counters();
    assert_eq!(c.who_has_received, 4);
    assert_eq!(c.i_have_sent, 1);
    assert_eq!(c.requests_coalesced, 3);

    server.stop().await.unwrap();
}

#[tokio::test]
async fn test_confirmed_traffic_latency_bounded_during_discovery_flood() {
    let policy = DiscoveryPolicy {
        max_responses_per_sec_per_source: 2,
        source_burst_capacity: 2,
        coalesce_window: Duration::from_millis(100),
        ..Default::default()
    };
    let (mut server, transport, tx) = spawn_test_server(policy).await;
    let flood_src = &[0x0A, 0x00, 0x00, 0x02];

    for _ in 0..50 {
        tx.send(build_who_is_npdu(None, None, flood_src, None))
            .await
            .unwrap();
    }

    // Simultaneously send a ConfirmedReadProperty request
    let client_mac = &[0x0A, 0x00, 0x00, 0x05];
    let rp = ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new(bacnet_types::enums::ObjectType::ANALOG_INPUT, 1)
            .unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    };
    let mut s_buf = BytesMut::new();
    rp.encode(&mut s_buf);
    let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 1,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: s_buf.freeze(),
    });
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &pdu).unwrap();
    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: true,
        priority: NetworkPriority::NORMAL,
        destination: None,
        source: None,
        hop_count: 255,
        payload: apdu_buf.freeze(),
        ..Npdu::default()
    };
    let mut raw_buf = BytesMut::new();
    encode_npdu(&mut raw_buf, &npdu).unwrap();

    let t0 = std::time::Instant::now();
    tx.send(ReceivedNpdu {
        npdu: raw_buf.freeze(),
        source_mac: MacAddr::from_slice(client_mac),
        link_layer_group: false,
        data_attributes: Vec::new(),
        reply_tx: None,
    })
    .await
    .unwrap();

    // Poll until response arrives
    let mut found_ack = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(5)).await;
        let unicasts = transport.unicasts.lock().unwrap();
        for (dest, raw) in unicasts.iter() {
            if dest == client_mac {
                if let Ok(np) = decode_npdu(raw.clone()) {
                    if let Ok(Apdu::ComplexAck(ack)) = decode_apdu(np.payload) {
                        if ack.invoke_id == 1 {
                            found_ack = true;
                            break;
                        }
                    }
                }
            }
        }
        if found_ack {
            break;
        }
    }
    let elapsed = t0.elapsed();
    assert!(found_ack, "Confirmed response was not received");
    assert!(
        elapsed < Duration::from_millis(500),
        "Confirmed request delayed by discovery flood: took {elapsed:?}"
    );

    server.stop().await.unwrap();
}

#[tokio::test]
async fn test_who_is_range_and_who_has_matching_correctness() {
    let policy = DiscoveryPolicy::unlimited();
    let (mut server, _transport, tx) = spawn_test_server(policy).await;
    let src = &[0x0A, 0x00, 0x00, 0x02];

    // Device instance is 1234
    // Out of range Who-Is: 2000..=3000 -> no response
    tx.send(build_who_is_npdu(Some(2000), Some(3000), src, None))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(server.discovery_counters().i_am_sent, 0);

    // In-range Who-Is: 1000..=2000 -> responds
    tx.send(build_who_is_npdu(Some(1000), Some(2000), src, None))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(server.discovery_counters().i_am_sent, 1);

    // Who-Has by ID out of range device limits -> no response
    let ai_oid = ObjectIdentifier::new(bacnet_types::enums::ObjectType::ANALOG_INPUT, 1).unwrap();
    tx.send(build_who_has_npdu(
        WhoHasObject::Identifier(ai_oid),
        Some(2000),
        Some(3000),
        src,
        None,
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(server.discovery_counters().i_have_sent, 0);

    // Who-Has by ID matching -> responds with I-Have
    tx.send(build_who_has_npdu(
        WhoHasObject::Identifier(ai_oid),
        Some(1000),
        Some(2000),
        src,
        None,
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(server.discovery_counters().i_have_sent, 1);

    // Who-Has by Name matching -> responds with I-Have
    tx.send(build_who_has_npdu(
        WhoHasObject::Name("Zone Temp".into()),
        None,
        None,
        src,
        None,
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(server.discovery_counters().i_have_sent, 2);

    server.stop().await.unwrap();
}

#[tokio::test]
async fn test_directed_vs_broadcast_and_routed_npdu() {
    // Part A: prefer_directed_responses = true (default)
    let policy = DiscoveryPolicy {
        prefer_directed_responses: true,
        coalesce_window: Duration::ZERO,
        ..Default::default()
    };
    let (mut server, transport, tx) = spawn_test_server(policy).await;
    let local_src = &[0x0A, 0x00, 0x00, 0x02];

    // Local Who-Is produces directed unicast response
    tx.send(build_who_is_npdu(None, None, local_src, None))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(transport.unicast_count(), 1);
    assert_eq!(transport.broadcast_count(), 0);
    assert_eq!(server.discovery_counters().directed_responses_sent, 1);

    // Routed Who-Is produces routed unicast response
    let router_mac = &[0x0A, 0x00, 0x00, 0xFE];
    let remote_peer = &[0x11, 0x22];
    tx.send(build_who_is_npdu(
        None,
        None,
        router_mac,
        Some((200, remote_peer)),
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(transport.unicast_count(), 2);
    assert_eq!(server.discovery_counters().directed_responses_sent, 2);

    // Routed Who-Has produces routed unicast I-Have (verifying routed Who-Has bug fix!)
    let ai_oid = ObjectIdentifier::new(bacnet_types::enums::ObjectType::ANALOG_INPUT, 1).unwrap();
    tx.send(build_who_has_npdu(
        WhoHasObject::Identifier(ai_oid),
        None,
        None,
        router_mac,
        Some((200, remote_peer)),
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(transport.unicast_count(), 3);
    assert_eq!(transport.broadcast_count(), 0);
    assert_eq!(server.discovery_counters().directed_responses_sent, 3);
    assert_eq!(server.discovery_counters().i_have_sent, 1);

    server.stop().await.unwrap();

    // Part B: prefer_directed_responses = false
    let policy_bcast = DiscoveryPolicy {
        prefer_directed_responses: false,
        coalesce_window: Duration::ZERO,
        ..Default::default()
    };
    let (mut server2, transport2, tx2) = spawn_test_server(policy_bcast).await;

    // Local Who-Is with link_layer_group / broadcast produces broadcast response
    let mut req = build_who_is_npdu(None, None, local_src, None);
    req.link_layer_group = true;
    tx2.send(req).await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(transport2.broadcast_count(), 1);
    assert_eq!(transport2.unicast_count(), 0);
    assert_eq!(server2.discovery_counters().directed_responses_sent, 0);

    // Routed Who-Is still routes back unicast even when prefer_directed_responses is false!
    tx2.send(build_who_is_npdu(
        None,
        None,
        router_mac,
        Some((200, remote_peer)),
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(transport2.unicast_count(), 1);
    assert_eq!(server2.discovery_counters().directed_responses_sent, 1);

    server2.stop().await.unwrap();
}

fn mock_received(
    source_mac: &[u8],
    routed: Option<(u16, &[u8])>,
) -> bacnet_network::layer::ReceivedApdu {
    bacnet_network::layer::ReceivedApdu {
        apdu: Bytes::new(),
        source_mac: MacAddr::from_slice(source_mac),
        ingress_network: None,
        source_network: routed.map(|(net, mac)| NpduAddress {
            network: net,
            mac_address: MacAddr::from_slice(mac),
        }),
        link_layer_group: false,
        is_group: false,
        data_attributes: Vec::new(),
        reply_tx: None,
    }
}

fn encode_who_is_req(low: Option<u32>, high: Option<u32>) -> Vec<u8> {
    let mut buf = BytesMut::new();
    WhoIsRequest {
        low_limit: low,
        high_limit: high,
    }
    .encode(&mut buf);
    buf.to_vec()
}

#[test]
fn test_exhausted_source_throttled_when_max_tracked_sources_reached() {
    let policy = DiscoveryPolicy {
        source_burst_capacity: 1,
        max_responses_per_sec_per_source: 1,
        max_tracked_sources: 2,
        coalesce_window: Duration::ZERO,
        ..DiscoveryPolicy::default()
    };
    let limiter = DiscoveryLimiter::new(policy, Some(100));
    let req = encode_who_is_req(None, None);
    let now = Instant::now();
    let src_a = mock_received(&[0x0A, 0x00, 0x00, 0x01], None);
    let src_b = mock_received(&[0x0A, 0x00, 0x00, 0x02], None);

    assert_eq!(
        limiter.pre_check_who_is(&req, &src_a, now),
        PreCheckDecision::Admit
    );
    assert_eq!(
        limiter.pre_check_who_is(&req, &src_b, now),
        PreCheckDecision::Admit
    );
    assert_eq!(
        limiter.pre_check_who_is(&req, &src_a, now),
        PreCheckDecision::ThrottledSource
    );
    assert_eq!(limiter.counters().responses_throttled_source, 1);
}

#[test]
fn test_routed_who_is_byte_accounting_uses_remote_mac() {
    let policy = DiscoveryPolicy {
        max_bytes_per_sec_per_source: 100,
        source_burst_capacity: 10,
        max_responses_per_sec_per_source: 10,
        coalesce_window: Duration::ZERO,
        ..DiscoveryPolicy::default()
    };
    let limiter = DiscoveryLimiter::new(policy, Some(100));
    let req = encode_who_is_req(None, None);
    let now = Instant::now();
    let routed = mock_received(&[0x0A, 0x00, 0x00, 0xFE], Some((200, &[0x11, 0x22])));

    assert_eq!(
        limiter.pre_check_who_is(&req, &routed, now),
        PreCheckDecision::Admit
    );
    limiter.record_i_am_sent(
        34,
        true,
        &routed.source_mac,
        routed.source_network.as_ref(),
        now,
    );

    let byte_tokens = limiter
        .source_byte_tokens(&routed.source_mac, routed.source_network.as_ref())
        .expect("routed source state must exist");
    assert!(
        (byte_tokens - 66.0).abs() < 1e-4,
        "Expected 66 byte tokens, got {byte_tokens}"
    );
}

#[test]
fn test_independent_byte_tokens_enforced_when_response_count_unlimited() {
    let now = Instant::now();
    let src = mock_received(&[0x0A, 0x00, 0x00, 0x01], None);
    let req = encode_who_is_req(None, None);

    let check = |max_g, b_g, max_s, b_s| {
        let lim = DiscoveryLimiter::new(
            DiscoveryPolicy {
                max_responses_per_sec_global: max_g,
                max_bytes_per_sec_global: b_g,
                max_responses_per_sec_per_source: max_s,
                max_bytes_per_sec_per_source: b_s,
                global_burst_capacity: max_g,
                source_burst_capacity: max_s,
                reserved_capacity: 0,
                coalesce_window: Duration::ZERO,
                ..DiscoveryPolicy::default()
            },
            Some(100),
        );
        (
            lim.pre_check_who_is(&req, &src, now),
            lim.pre_check_who_is(&req, &src, now),
        )
    };

    // 1. Unlimited count, per-source byte limited
    assert_eq!(
        check(u32::MAX, usize::MAX, u32::MAX, 100),
        (PreCheckDecision::Admit, PreCheckDecision::ThrottledSource)
    );
    // 2. Unlimited count, global byte limited
    assert_eq!(
        check(u32::MAX, 100, u32::MAX, usize::MAX),
        (PreCheckDecision::Admit, PreCheckDecision::ThrottledGlobal)
    );
    // 3. Unlimited bytes, global count limited
    assert_eq!(
        check(1, usize::MAX, u32::MAX, usize::MAX),
        (PreCheckDecision::Admit, PreCheckDecision::ThrottledGlobal)
    );
}

#[test]
fn test_negative_who_has_cache_strictly_bounded_to_512() {
    let limiter = DiscoveryLimiter::new(
        DiscoveryPolicy {
            coalesce_window: Duration::from_secs(60),
            ..DiscoveryPolicy::default()
        },
        Some(100),
    );
    let base = Instant::now();

    for i in 0..600 {
        let target = WhoHasTarget::Id(ObjectIdentifier::new(ObjectType::ANALOG_INPUT, i).unwrap());
        limiter.record_negative_who_has(target, base + Duration::from_millis(i as u64));
    }

    assert_eq!(limiter.negative_who_has_count(), 512);

    let check_time = base + Duration::from_millis(600);
    let target_0 = WhoHasTarget::Id(ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 0).unwrap());
    assert!(!limiter.is_negative_who_has(&target_0, check_time));
    let target_599 =
        WhoHasTarget::Id(ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 599).unwrap());
    assert!(limiter.is_negative_who_has(&target_599, check_time));
}
