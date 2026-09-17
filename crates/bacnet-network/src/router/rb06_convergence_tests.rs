//! RB-06: freshness split, standard vs hardened convergence, bounded discovery.
//!
//! Clauses 6.6.3.1 (reject only when the network cannot be found in the table
//! or through Who-Is), 6.6.3.2 (each new I-Am represents a configuration
//! modification; last message takes precedence), 6.6.3.3 (replace MAC/port on
//! difference) and 6.5 (attempt Who-Is when the next router is unknown).
//! In-memory only, deterministic time via table/tracker injection, no sleeps.

use super::envelope_harness::{broadcast_data, control_npdu, Harness};
use super::*;
use crate::router_table::ConvergenceMode;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::port::TransportProvenance;
use std::time::Instant;

fn learned_at(port: usize, now: Instant) -> RouterTable {
    let mut table = RouterTable::new();
    table.add_learned_at(3000, port, MacAddr::from_slice(&[1]), now);
    table
}

// --- Freshness split ---

#[test]
fn touch_updates_used_only_and_expiry_keys_off_seen() {
    let base = Instant::now();
    let mut table = learned_at(0, base);
    assert!(table.lookup(3000).unwrap().last_used.is_none());
    for i in 1..=100 {
        table.touch_used_at(3000, base + Duration::from_secs(i));
    }
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.last_seen, Some(base));
    assert_eq!(entry.last_used, Some(base + Duration::from_secs(100)));
    // Sustained traffic did not slide expiry: 300s rescue still fires.
    let purged = table.purge_stale_at(base + Duration::from_secs(301), Duration::from_secs(300));
    assert_eq!(purged, vec![3000]);
    assert!(table.lookup(3000).is_none());
}

#[test]
fn expiry_without_traffic_fires_on_seen_and_directs_never_expire() {
    let base = Instant::now();
    let mut table = RouterTable::new();
    table.add_learned_at(3000, 0, MacAddr::from_slice(&[1]), base);
    table.add_direct(1000, 0);
    table.touch_used_at(1000, base + Duration::from_secs(10));
    assert!(table.lookup(1000).unwrap().last_seen.is_none());
    let purged = table.purge_stale_at(base + Duration::from_secs(301), Duration::from_secs(300));
    assert_eq!(purged, vec![3000]);
    assert!(table.lookup(1000).is_some());
}

#[test]
fn same_port_refresh_updates_seen_preserves_used() {
    let base = Instant::now();
    let mut table = learned_at(0, base);
    table.touch_used_at(3000, base + Duration::from_secs(10));
    assert!(table.apply_learning_claim(
        3000,
        0,
        MacAddr::from_slice(&[2]),
        base + Duration::from_secs(20)
    ));
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.last_seen, Some(base + Duration::from_secs(20)));
    assert_eq!(entry.last_used, Some(base + Duration::from_secs(10)));
    assert_eq!(entry.next_hop_mac.as_slice(), &[2]);
}

#[test]
fn busy_available_touch_neither_timestamp() {
    let base = Instant::now();
    let mut table = learned_at(0, base);
    table.touch_used_at(3000, base + Duration::from_secs(5));
    table.mark_busy(3000, base + Duration::from_secs(35));
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.last_seen, Some(base));
    assert_eq!(entry.last_used, Some(base + Duration::from_secs(5)));
    table.mark_available(3000);
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.last_seen, Some(base));
    assert_eq!(entry.last_used, Some(base + Duration::from_secs(5)));
}

// --- Convergence modes ---

#[test]
fn standard_converges_on_single_claim_under_sustained_traffic() {
    let base = Instant::now();
    let mut table = learned_at(0, base);
    assert_eq!(table.convergence(), ConvergenceMode::Standard);
    for i in 1..=50 {
        table.touch_used_at(3000, base + Duration::from_secs(i));
    }
    assert!(table.apply_learning_claim(
        3000,
        1,
        MacAddr::from_slice(&[2]),
        base + Duration::from_secs(51)
    ));
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.port_index, 1);
    assert_eq!(entry.next_hop_mac.as_slice(), &[2]);
    assert_eq!(entry.last_seen, Some(base + Duration::from_secs(51)));
    assert!(entry.last_used.is_none());
    assert_eq!(table.pending_count(), 0);
    assert_eq!(table.claim_snapshot().pending_started, 0);
}

#[test]
fn hardened_holds_then_reaps_then_recovers() {
    let base = Instant::now();
    let mut table = RouterTable::new_hardened();
    table.add_learned_at(3000, 0, MacAddr::from_slice(&[1]), base);
    assert!(!table.apply_learning_claim(3000, 1, MacAddr::from_slice(&[2]), base));
    assert_eq!(table.lookup(3000).unwrap().port_index, 0);
    assert_eq!(table.pending_count(), 1);
    // Idle reaper clears the slot so a single-shot advertiser does not linger.
    let expired = table.expire_pending_at(base + Duration::from_secs(61));
    assert_eq!(expired, vec![3000]);
    assert_eq!(table.pending_count(), 0);
    assert_eq!(table.claim_snapshot().pending_expired, 1);
    // Fresh challenger pends again, immediate repeat corroborates.
    assert!(!table.apply_learning_claim(
        3000,
        1,
        MacAddr::from_slice(&[2]),
        base + Duration::from_secs(62)
    ));
    assert!(table.apply_learning_claim(
        3000,
        1,
        MacAddr::from_slice(&[2]),
        base + Duration::from_secs(63)
    ));
    assert_eq!(table.lookup(3000).unwrap().port_index, 1);
    assert_eq!(table.claim_snapshot().corroborated_applied, 1);
}

#[test]
fn hardened_alternation_never_converges_robustness_only() {
    // Cyclic alternation is a bounded-robustness check, never a supported
    // topology claim: the slot resets each time and the old route forwards.
    let base = Instant::now();
    let mut table = RouterTable::new_hardened();
    table.add_learned_at(3000, 0, MacAddr::from_slice(&[1]), base);
    for i in 0..10 {
        assert!(!table.apply_learning_claim(
            3000,
            1 + (i % 2) as usize,
            MacAddr::from_slice(&[2]),
            base + Duration::from_secs(i)
        ));
        assert_eq!(table.lookup(3000).unwrap().port_index, 0);
    }
    assert_eq!(table.pending_count(), 1);
    assert_eq!(table.claim_snapshot().pending_started, 10);
}

// --- Discovery tracker bounds ---

#[test]
fn discovery_coalesces_rate_limits_expires_and_caps() {
    let base = Instant::now();
    let mut disc = DiscoveryTracker::default();
    assert!(disc.should_solicit_at(3000, base));
    assert!(!disc.should_solicit_at(3000, base + Duration::from_secs(1)));
    assert_eq!(disc.len(), 1);
    assert!(disc.should_solicit_at(3000, base + Duration::from_secs(5)));
    assert!(!disc.should_solicit_at(0, base));
    assert!(!disc.should_solicit_at(0xFFFF, base));
    for net in 4000..(4000 + 255) {
        assert!(disc.should_solicit_at(net, base));
    }
    assert_eq!(disc.len(), 256);
    assert!(!disc.should_solicit_at(9999, base));
    let expired = disc.expire_at(base + Duration::from_secs(36));
    assert_eq!(expired.len(), 256);
    assert_eq!(disc.len(), 0);
    assert!(disc.should_solicit_at(9999, base + Duration::from_secs(37)));
    disc.cancel();
    assert!(disc.is_cancelled());
    assert_eq!(disc.len(), 0);
    assert!(!disc.should_solicit_at(3000, base + Duration::from_secs(100)));
}

// --- Router harness: directed unknown solicits, coalesces, learns ---

#[tokio::test]
async fn directed_unknown_solicits_once_then_rejects_twice_coalesced() {
    let mut h = Harness::two_port();
    for _ in 0..2 {
        let mut npdu = control_npdu(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[0x27, 0x0f]);
        npdu.destination = Some(NpduAddress {
            network: 9999,
            mac_address: MacAddr::from_slice(&[9]),
        });
        npdu.hop_count = 255;
        h.dispatch(h.ctx(0, &[7], npdu)).await;
    }
    let solicited = broadcast_data(h.drain(1));
    assert_eq!(solicited.len(), 1);
    let who_is = decode_npdu(solicited[0].clone()).unwrap();
    assert_eq!(
        who_is.message_type,
        Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw())
    );
    assert_eq!(who_is.payload.as_ref(), &[0x27, 0x0f]);
    let rejects = h.drain(0);
    assert_eq!(rejects.len(), 2);
    for req in rejects {
        match req {
            SendRequest::Unicast { npdu, mac, .. } => {
                assert_eq!(mac.as_slice(), &[7]);
                let decoded = decode_npdu(npdu).unwrap();
                assert_eq!(
                    decoded.message_type,
                    Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw())
                );
                assert_eq!(
                    decoded.payload.as_ref(),
                    &[
                        RejectMessageReason::NOT_DIRECTLY_CONNECTED.to_raw(),
                        0x27,
                        0x0f
                    ]
                );
            }
            SendRequest::Broadcast { .. } => panic!("expected unicast reject"),
        }
    }
}

#[tokio::test]
async fn directed_unknown_discovered_via_answer_routes_next_packet() {
    let mut h = Harness::two_port();
    let mut npdu = control_npdu(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[0x0b, 0xb8]);
    npdu.destination = Some(NpduAddress {
        network: 3000,
        mac_address: MacAddr::from_slice(&[9]),
    });
    npdu.hop_count = 255;
    h.dispatch(h.ctx(0, &[7], npdu)).await;
    assert_eq!(broadcast_data(h.drain(1)).len(), 1);
    assert_eq!(h.drain(0).len(), 1);
    assert!(h.table.lock().await.lookup(3000).is_none());
    // Answer installs the route (standard last-wins, no corroboration wait).
    h.handle(h.ctx(
        1,
        &[2],
        control_npdu(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0x0b, 0xb8]),
    ))
    .await;
    // Consume the unconditional I-Am rebroadcast before the next dispatch.
    for port in 0..2 {
        for req in h.drain(port) {
            match req {
                SendRequest::Broadcast { npdu, .. } => {
                    assert_eq!(
                        decode_npdu(npdu).unwrap().message_type,
                        Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw())
                    );
                }
                SendRequest::Unicast { .. } => panic!("expected only I-Am rebroadcast"),
            }
        }
    }
    assert!(h.table.lock().await.lookup(3000).is_some());
    assert_eq!(h.table.lock().await.lookup(3000).unwrap().port_index, 1);
    // Next packet to the now-known network forwards instead of rejecting.
    let mut npdu = control_npdu(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[0x0b, 0xb8]);
    npdu.destination = Some(NpduAddress {
        network: 3000,
        mac_address: MacAddr::from_slice(&[9]),
    });
    npdu.hop_count = 255;
    h.dispatch(h.ctx(0, &[7], npdu)).await;
    assert!(h.drain(0).is_empty());
    match h.drain(1).pop().expect("expected forward") {
        SendRequest::Unicast { mac, .. } => assert_eq!(mac.as_slice(), &[2]),
        SendRequest::Broadcast { .. } => panic!("expected unicast forward"),
    }
}

// --- Real router: APDU unknown path + stop cancellation ---

struct MemTransport {
    incoming: Option<mpsc::Receiver<ReceivedNpdu>>,
    outgoing: mpsc::Sender<SendRequest>,
    mac: [u8; 1],
}

impl bacnet_transport::port::TransportPort for MemTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, bacnet_types::error::Error> {
        self.incoming
            .take()
            .ok_or_else(|| bacnet_types::error::Error::Encoding("started".into()))
    }
    async fn stop(&mut self) -> Result<(), bacnet_types::error::Error> {
        Ok(())
    }
    async fn send_unicast(
        &self,
        npdu: &[u8],
        mac: &[u8],
    ) -> Result<(), bacnet_types::error::Error> {
        self.send_unicast_with_data_attributes(npdu, mac, &[]).await
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), bacnet_types::error::Error> {
        self.send_broadcast_with_data_attributes(npdu, &[]).await
    }
    async fn send_unicast_with_data_attributes(
        &self,
        npdu: &[u8],
        mac: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), bacnet_types::error::Error> {
        self.outgoing
            .try_send(SendRequest::Unicast {
                npdu: Bytes::copy_from_slice(npdu),
                mac: MacAddr::from_slice(mac),
                data_attributes: data_attributes.to_vec(),
            })
            .map_err(|_| bacnet_types::error::Error::Encoding("wire full".into()))
    }
    async fn send_broadcast_with_data_attributes(
        &self,
        npdu: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), bacnet_types::error::Error> {
        self.outgoing
            .try_send(SendRequest::Broadcast {
                npdu: Bytes::copy_from_slice(npdu),
                data_attributes: data_attributes.to_vec(),
            })
            .map_err(|_| bacnet_types::error::Error::Encoding("wire full".into()))
    }
    fn local_mac(&self) -> &[u8] {
        &self.mac
    }
}

fn apdu_to(net: u16) -> Bytes {
    let npdu = Npdu {
        destination: Some(NpduAddress {
            network: net,
            mac_address: MacAddr::from_slice(&[9]),
        }),
        hop_count: 255,
        payload: Bytes::from_static(&[0x10, 0x20]),
        ..Npdu::default()
    };
    let mut buf = BytesMut::new();
    encode_npdu(&mut buf, &npdu).unwrap();
    buf.freeze()
}

fn ingress(npdu: Bytes) -> ReceivedNpdu {
    ReceivedNpdu {
        npdu,
        source_mac: MacAddr::from_slice(&[7]),
        link_layer_group: true,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

#[tokio::test]
async fn apdu_unknown_solicits_rejects_then_forwards_after_answer() {
    let (tx0, rx0) = mpsc::channel(16);
    let (out0, mut wire0) = mpsc::channel(16);
    let (tx1, _rx1) = mpsc::channel(16);
    let (out1, mut wire1) = mpsc::channel(16);
    let ports = vec![
        RouterPort {
            transport: MemTransport {
                incoming: Some(rx0),
                outgoing: out0,
                mac: [1],
            },
            network_number: 1000,
        },
        RouterPort {
            transport: MemTransport {
                incoming: Some(_rx1),
                outgoing: out1,
                mac: [2],
            },
            network_number: 2000,
        },
    ];
    let (mut router, _local) = BACnetRouter::start(ports).await.unwrap();
    for _ in [wire0.try_recv(), wire1.try_recv()] {}
    tx0.send(ingress(apdu_to(3000))).await.unwrap();
    // Unknown APDU: one Who-Is out the other port, one retryable reject back.
    let mut saw_who_is = false;
    let mut saw_reject = false;
    for _ in 0..4 {
        tokio::task::yield_now().await;
        while let Ok(req) = wire1.try_recv() {
            if let SendRequest::Broadcast { npdu, .. } = req {
                let decoded = decode_npdu(npdu).unwrap();
                if decoded.message_type
                    == Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw())
                    && decoded.payload.as_ref() == &[0x0b, 0xb8]
                {
                    saw_who_is = true;
                }
            }
        }
        while let Ok(req) = wire0.try_recv() {
            if let SendRequest::Unicast { npdu, mac, .. } = req {
                let decoded = decode_npdu(npdu).unwrap();
                if decoded.message_type
                    == Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw())
                    && decoded.payload.as_ref()
                        == &[
                            RejectMessageReason::NOT_DIRECTLY_CONNECTED.to_raw(),
                            0x0b,
                            0xb8,
                        ]
                    && mac.as_slice() == &[7]
                {
                    saw_reject = true;
                }
            }
        }
        if saw_who_is && saw_reject {
            break;
        }
    }
    assert!(saw_who_is, "expected bounded Who-Is solicitation");
    assert!(saw_reject, "expected honest retryable reject");
    // Answer installs the route; the retry forwards without a reject.
    {
        let mut tbl = router.table().lock().await;
        tbl.apply_learning_claim(3000, 1, MacAddr::from_slice(&[2]), Instant::now());
        tbl.record_learned();
    }
    tx0.send(ingress(apdu_to(3000))).await.unwrap();
    let mut forwarded = false;
    let mut rejected = false;
    for _ in 0..20 {
        tokio::task::yield_now().await;
        while let Ok(req) = wire1.try_recv() {
            if let SendRequest::Unicast { mac, .. } = req {
                if mac.as_slice() == &[2] {
                    forwarded = true;
                }
            }
        }
        while let Ok(req) = wire0.try_recv() {
            if let SendRequest::Unicast { npdu, .. } = req {
                if decode_npdu(npdu).unwrap().message_type
                    == Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw())
                {
                    rejected = true;
                }
            }
        }
        if forwarded {
            break;
        }
    }
    assert!(forwarded, "expected forward after discovery");
    assert!(!rejected, "no reject once the route is known");
    router.stop().await;
}

#[tokio::test]
async fn stop_cancels_pending_discovery() {
    let (tx0, rx0) = mpsc::channel(16);
    let (out0, mut wire0) = mpsc::channel(16);
    let (tx1, _rx1) = mpsc::channel(16);
    let (out1, _wire1) = mpsc::channel(16);
    let ports = vec![
        RouterPort {
            transport: MemTransport {
                incoming: Some(rx0),
                outgoing: out0,
                mac: [1],
            },
            network_number: 1000,
        },
        RouterPort {
            transport: MemTransport {
                incoming: Some(_rx1),
                outgoing: out1,
                mac: [2],
            },
            network_number: 2000,
        },
    ];
    let (mut router, _local) = BACnetRouter::start(ports).await.unwrap();
    for _ in [wire0.try_recv()] {}
    tx0.send(ingress(apdu_to(4000))).await.unwrap();
    for _ in 0..10 {
        tokio::task::yield_now().await;
        if router.discovery.lock().await.len() == 1 {
            break;
        }
    }
    assert_eq!(router.discovery.lock().await.len(), 1);
    router.stop().await;
    let disc = router.discovery.lock().await;
    assert!(disc.is_cancelled());
    assert_eq!(disc.len(), 0);
}
