//! RB-05 routing-table management tests: Initialize-Routing-Table update and
//! query semantics plus Init-Ack claim separation (135-2020 6.4.7–6.4.8,
//! 6.6.3.8–6.6.3.9).
//!
//! In-memory only, no sleeps. Wire assertions decode reply bytes with
//! [`decode_npdu`](bacnet_encoding::npdu::decode_npdu) and re-walk the entry
//! octets by hand, never reusing the implementation's parser.

use super::envelope_harness::*;
use super::*;
use crate::router_table::RoutingClaimSnapshot;

/// Independent-peer entry walk: count + `DNET(2) + Port ID(1) + Port Info(N)`
/// with no trailing bytes. Panics on any envelope deviation.
fn peer_entries(payload: &[u8]) -> Vec<(u16, u8)> {
    let count = usize::from(payload[0]);
    let mut entries = Vec::with_capacity(count);
    let mut offset = 1usize;
    for _ in 0..count {
        assert!(offset + 4 <= payload.len(), "peer: truncated entry");
        let net = u16::from_be_bytes([payload[offset], payload[offset + 1]]);
        let port_id = payload[offset + 2];
        let info_len = usize::from(payload[offset + 3]);
        offset += 4 + info_len;
        assert!(offset <= payload.len(), "peer: truncated Port Info");
        entries.push((net, port_id));
    }
    assert_eq!(offset, payload.len(), "peer: trailing bytes");
    entries
}

fn unicast_payloads(requests: Vec<SendRequest>) -> Vec<Bytes> {
    requests
        .into_iter()
        .map(|req| match req {
            SendRequest::Unicast { npdu, .. } => npdu,
            SendRequest::Broadcast { .. } => panic!("expected unicast"),
        })
        .collect()
}

#[test]
fn wire_port_id_mapping_is_stable_nonzero_and_checked() {
    assert_eq!(RouterTable::wire_port_id(0), Some(1));
    assert_eq!(RouterTable::wire_port_id(1), Some(2));
    assert_eq!(RouterTable::port_index_for_wire_id(1, 2), Some(0));
    assert_eq!(RouterTable::port_index_for_wire_id(2, 2), Some(1));
    // Port ID 0 is the purge trigger, never a real port.
    assert_eq!(RouterTable::port_index_for_wire_id(0, 2), None);
    // Past the local port count names no local port.
    assert_eq!(RouterTable::port_index_for_wire_id(3, 2), None);
    assert_eq!(RouterTable::port_index_for_wire_id(255, 2), None);
}

#[tokio::test]
async fn zero_entry_query_leaves_table_untouched() {
    let mut h = Harness::two_port();
    let before: Vec<(u16, usize, bool)> = {
        let tbl = h.table.lock().await;
        let mut rows: Vec<(u16, usize, bool)> = tbl
            .sorted_networks()
            .iter()
            .map(|net| {
                let entry = tbl.lookup(*net).unwrap();
                (*net, entry.port_index, entry.directly_connected)
            })
            .collect();
        rows.sort_unstable();
        rows
    };
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::INITIALIZE_ROUTING_TABLE, &[0]),
    ))
    .await;
    assert!(h.drain(1).is_empty());
    let replies = unicast_payloads(h.drain(0));
    assert_eq!(replies.len(), 1);
    let decoded = decode_npdu(replies[0].clone()).unwrap();
    assert_eq!(
        decoded.message_type,
        Some(NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK.to_raw())
    );
    assert_eq!(peer_entries(&decoded.payload), vec![(1000, 1), (2000, 2)]);
    let after: Vec<(u16, usize, bool)> = {
        let tbl = h.table.lock().await;
        let mut rows: Vec<(u16, usize, bool)> = tbl
            .sorted_networks()
            .iter()
            .map(|net| {
                let entry = tbl.lookup(*net).unwrap();
                (*net, entry.port_index, entry.directly_connected)
            })
            .collect();
        rows.sort_unstable();
        rows
    };
    assert_eq!(before, after);
}

#[tokio::test]
async fn empty_table_query_is_one_count_zero_ack() {
    let mut h = Harness::with_table(RouterTable::new());
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::INITIALIZE_ROUTING_TABLE, &[0]),
    ))
    .await;
    let replies = unicast_payloads(h.drain(0));
    assert_eq!(replies.len(), 1);
    let decoded = decode_npdu(replies[0].clone()).unwrap();
    assert_eq!(decoded.payload.as_ref(), &[0]);
}

#[tokio::test]
async fn update_installs_learned_route_on_mapped_port() {
    let mut h = Harness::two_port();
    h.handle(h.ctx(
        0,
        &[9],
        control_npdu(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE,
            &[1, 0x0f, 0xa0, 2, 0],
        ),
    ))
    .await;
    let replies = unicast_payloads(h.drain(0));
    assert_eq!(replies.len(), 1);
    let decoded = decode_npdu(replies[0].clone()).unwrap();
    assert_eq!(
        decoded.message_type,
        Some(NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK.to_raw())
    );
    assert!(decoded.payload.is_empty());
    let table = h.table.lock().await;
    let route = table.lookup(4000).unwrap();
    assert_eq!(route.port_index, 1);
    assert!(!route.directly_connected);
    assert_eq!(route.next_hop_mac.as_slice(), &[9]);
    // Configured attachments are untouched.
    assert!(table.lookup(1000).unwrap().directly_connected);
    assert!(table.lookup(2000).unwrap().directly_connected);
    assert_eq!(table.claim_snapshot(), RoutingClaimSnapshot::default());
}

#[tokio::test]
async fn purge_removes_learned_but_preserves_direct() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);
    table.add_learned(3000, 0, MacAddr::from_slice(&[1]));
    let mut h = Harness::with_table(table);
    // Purge learned 3000 and direct 1000: only the learned entry goes.
    h.handle(h.ctx(
        1,
        &[2],
        control_npdu(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE,
            &[2, 0x0b, 0xb8, 0, 0, 0x03, 0xe8, 0, 0],
        ),
    ))
    .await;
    let replies = unicast_payloads(h.drain(1));
    assert_eq!(replies.len(), 1);
    assert!(decode_npdu(replies[0].clone()).unwrap().payload.is_empty());
    let table = h.table.lock().await;
    assert!(table.lookup(3000).is_none());
    let direct = table.lookup(1000).unwrap();
    assert!(direct.directly_connected);
    assert_eq!(direct.port_index, 0);
}

#[tokio::test]
async fn replacement_rewrites_learned_mapping_in_one_message() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);
    table.add_learned(3000, 0, MacAddr::from_slice(&[1]));
    let mut h = Harness::with_table(table);
    // No corroboration round-trip: one management write moves the route.
    h.handle(h.ctx(
        0,
        &[9],
        control_npdu(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE,
            &[1, 0x0b, 0xb8, 2, 0],
        ),
    ))
    .await;
    assert_eq!(unicast_payloads(h.drain(0)).len(), 1);
    let table = h.table.lock().await;
    let route = table.lookup(3000).unwrap();
    assert_eq!(route.port_index, 1);
    assert!(!route.directly_connected);
    assert_eq!(route.next_hop_mac.as_slice(), &[9]);
    assert_eq!(table.claim_snapshot().learned_ok, 0);
}

#[tokio::test]
async fn intra_message_order_is_last_wins() {
    let mut h = Harness::two_port();
    h.handle(h.ctx(
        0,
        &[9],
        control_npdu(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE,
            &[
                4, 0x0f, 0xa0, 2, 0, 0x0f, 0xa0, 0, 0, 0x13, 0x88, 0, 0, 0x13, 0x88, 1, 0,
            ],
        ),
    ))
    .await;
    assert_eq!(unicast_payloads(h.drain(0)).len(), 1);
    let table = h.table.lock().await;
    // Added then purged: absent.
    assert!(table.lookup(4000).is_none());
    // Purged (absent no-op) then added via Port ID 1: learned on port 0.
    let route = table.lookup(5000).unwrap();
    assert_eq!(route.port_index, 0);
    assert!(!route.directly_connected);
}

#[tokio::test]
async fn truncated_port_info_rejects_whole_message() {
    for payload in [
        // Port Info Length 5 with one octet present.
        vec![1, 0x0b, 0xb8, 1, 5, 0xaa],
        // Entry cut mid-header.
        vec![1, 0x0b, 0xb8],
        // Valid entry plus trailing garbage.
        vec![1, 0x0b, 0xb8, 1, 0, 0xff],
        // Valid entry plus a truncated second entry.
        vec![2, 0x0b, 0xb8, 1, 0, 0x0f],
    ] {
        let mut h = Harness::two_port();
        h.handle(h.ctx(
            0,
            &[9],
            control_npdu(NetworkMessageType::INITIALIZE_ROUTING_TABLE, &payload),
        ))
        .await;
        h.assert_quiet();
        let table = h.table.lock().await;
        assert_eq!(table.len(), 2);
        assert!(table.lookup(3000).is_none());
        assert_eq!(table.claim_snapshot(), RoutingClaimSnapshot::default());
    }
}

#[tokio::test]
async fn unknown_port_and_reserved_networks_skip_rest_applies() {
    let mut h = Harness::two_port();
    h.handle(h.ctx(
        0,
        &[9],
        control_npdu(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE,
            &[
                4, 0x1b, 0x58, 9, 0, // 7000 via unknown Port ID 9: skipped.
                0x1b, 0x59, 2, 0, // 7001 via Port ID 2: learned on port 1.
                0x00, 0x00, 1, 0, // Reserved DNET 0: skipped.
                0xff, 0xff, 1, 0, // Reserved DNET 0xFFFF: skipped.
            ],
        ),
    ))
    .await;
    // Well-formed message: the update ACK still goes out (empty).
    let replies = unicast_payloads(h.drain(0));
    assert_eq!(replies.len(), 1);
    assert!(decode_npdu(replies[0].clone()).unwrap().payload.is_empty());
    let table = h.table.lock().await;
    assert!(table.lookup(7000).is_none());
    assert_eq!(table.lookup(7001).unwrap().port_index, 1);
    assert_eq!(table.len(), 3);
}

#[tokio::test]
async fn reserved_entry_does_not_consume_cap_or_hide_suffix() {
    // Full table (256 learned): a leading reserved-DNET entry with a valid
    // Port ID must skip before the route-cap check, so the replacement
    // behind it still applies and the cap counter stays truthful.
    let mut table = RouterTable::new();
    for net in 1..=256u16 {
        table.add_learned(net, 0, MacAddr::from_slice(&[1]));
    }
    let mut h = Harness::with_table(table);
    h.handle(h.ctx(
        0,
        &[9],
        control_npdu(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE,
            &[2, 0x00, 0x00, 2, 0, 0x00, 0x01, 2, 0],
        ),
    ))
    .await;
    // Well-formed update: the empty success ACK still goes out.
    let replies = unicast_payloads(h.drain(0));
    assert_eq!(replies.len(), 1);
    assert!(decode_npdu(replies[0].clone()).unwrap().payload.is_empty());
    let table = h.table.lock().await;
    assert_eq!(table.len(), 256);
    let route = table.lookup(1).unwrap();
    assert_eq!(route.port_index, 1);
    assert!(!route.directly_connected);
    assert_eq!(route.next_hop_mac.as_slice(), &[9]);
    assert_eq!(table.claim_snapshot(), RoutingClaimSnapshot::default());
}

#[tokio::test]
async fn large_table_query_splits_into_bounded_acks() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);
    for net in 10..310u16 {
        let port = usize::from(net % 2);
        table.add_learned(net, port, MacAddr::from_slice(&[port as u8]));
    }
    let expected_total = table.len();
    assert_eq!(expected_total, 302);
    let mut h = Harness::with_table(table);
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::INITIALIZE_ROUTING_TABLE, &[0]),
    ))
    .await;
    assert!(h.drain(1).is_empty());
    let replies = unicast_payloads(h.drain(0));
    assert!(replies.len() > 1, "302 entries must not fit one ACK");
    let mut stream: Vec<(u16, u8)> = Vec::with_capacity(expected_total);
    for raw in &replies {
        // Egress bound holds on the encoded reply itself.
        assert!(
            raw.len() <= 501,
            "reply of {} octets exceeds 501",
            raw.len()
        );
        let decoded = decode_npdu(raw.clone()).unwrap();
        assert_eq!(
            decoded.message_type,
            Some(NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK.to_raw())
        );
        assert!(decoded.payload.len() >= 1);
        assert!(decoded.payload[0] as usize <= 255);
        let entries = peer_entries(&decoded.payload);
        assert_eq!(entries.len(), usize::from(decoded.payload[0]));
        stream.extend_from_slice(&entries);
    }
    // Completeness: every route exactly once, ascending, correct wire Port ID.
    assert_eq!(stream.len(), expected_total);
    let mut sorted = stream.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), expected_total);
    assert_eq!(stream, sorted);
    assert!(stream
        .iter()
        .all(|(_, port_id)| *port_id == 1 || *port_id == 2));
    assert!(stream.contains(&(1000, 1)));
    assert!(stream.contains(&(2000, 2)));
    // Even learned nets sit on port 0 (wire 1), odd on port 1 (wire 2).
    assert!(stream.contains(&(10, 1)));
    assert!(stream.contains(&(11, 2)));
}

#[tokio::test]
async fn full_queue_drops_acks_without_mutation_or_panic() {
    // Query with a saturated ingress queue: no ACK leaves, table unchanged.
    let mut h = Harness::two_port();
    for _ in 0..16 {
        h.txs[0]
            .try_send(SendRequest::Broadcast {
                npdu: Bytes::new(),
                data_attributes: Vec::new(),
            })
            .unwrap();
    }
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::INITIALIZE_ROUTING_TABLE, &[0]),
    ))
    .await;
    let drained = h.drain(0);
    assert_eq!(drained.len(), 16);
    for req in drained {
        match req {
            SendRequest::Broadcast { npdu, .. } => assert!(npdu.is_empty()),
            SendRequest::Unicast { .. } => panic!("no ACK fits a full queue"),
        }
    }
    assert_eq!(h.table.lock().await.len(), 2);

    // Update with a saturated queue still applies, ACK dropped (bounded).
    let mut h = Harness::two_port();
    for _ in 0..16 {
        h.txs[0]
            .try_send(SendRequest::Broadcast {
                npdu: Bytes::new(),
                data_attributes: Vec::new(),
            })
            .unwrap();
    }
    h.handle(h.ctx(
        0,
        &[9],
        control_npdu(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE,
            &[1, 0x0b, 0xb8, 2, 0],
        ),
    ))
    .await;
    assert!(h.table.lock().await.lookup(3000).is_some());
    assert_eq!(h.drain(0).len(), 16);
}

#[tokio::test]
async fn ack_claims_learn_via_ingress_and_never_reconfigure() {
    // The ACK's Port IDs are the sender's local numbering: they must not
    // steer our table, and direct routes must survive ACK traffic.
    // Hardened gate: first cross-port claim pends, repeat corroborates.
    let mut table = RouterTable::new_hardened();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);
    table.add_learned(3000, 0, MacAddr::from_slice(&[1]));
    let mut h = Harness::with_table(table);
    // First cross-port claim for 3000: held pending, old route forwards.
    h.handle(h.ctx(
        1,
        &[2],
        control_npdu(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
            &[2, 0x03, 0xe8, 2, 0, 0x0b, 0xb8, 2, 0],
        ),
    ))
    .await;
    h.assert_quiet();
    {
        let table = h.table.lock().await;
        let direct = table.lookup(1000).unwrap();
        assert!(direct.directly_connected);
        assert_eq!(direct.port_index, 0);
        assert_eq!(table.lookup(3000).unwrap().port_index, 0);
    }
    // Repeat corroborates: 3000 moves via the ingress port (1), never via
    // the sender's Port ID; the direct entry is still intact.
    h.handle(h.ctx(
        1,
        &[2],
        control_npdu(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
            &[2, 0x03, 0xe8, 2, 0, 0x0b, 0xb8, 2, 0],
        ),
    ))
    .await;
    h.assert_quiet();
    let table = h.table.lock().await;
    let moved = table.lookup(3000).unwrap();
    assert_eq!(moved.port_index, 1);
    assert!(!moved.directly_connected);
    assert_eq!(moved.next_hop_mac.as_slice(), &[2]);
    assert!(table.lookup(1000).unwrap().directly_connected);
    assert_eq!(
        table.claim_snapshot(),
        RoutingClaimSnapshot {
            learned_ok: 1,
            pending_started: 1,
            corroborated_applied: 1,
            ..Default::default()
        }
    );
}

#[tokio::test]
async fn update_ack_round_trip_is_stable() {
    // A query answer re-applied as an update restores the same table: the
    // wire Port IDs are stable and nonzero in both directions.
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);
    table.add_learned(3000, 1, MacAddr::from_slice(&[5]));
    let mut h = Harness::with_table(table);
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::INITIALIZE_ROUTING_TABLE, &[0]),
    ))
    .await;
    let replies = unicast_payloads(h.drain(0));
    assert_eq!(replies.len(), 1);
    let answer = decode_npdu(replies[0].clone()).unwrap().payload.to_vec();
    assert_eq!(peer_entries(&answer), vec![(1000, 1), (2000, 2), (3000, 2)]);
    // Purge the learned entry, then restore it from the captured answer.
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(
            NetworkMessageType::INITIALIZE_ROUTING_TABLE,
            &[1, 0x0b, 0xb8, 0, 0],
        ),
    ))
    .await;
    assert!(h.table.lock().await.lookup(3000).is_none());
    h.handle(h.ctx(
        0,
        &[7],
        control_npdu(NetworkMessageType::INITIALIZE_ROUTING_TABLE, &answer),
    ))
    .await;
    let table = h.table.lock().await;
    // Learned entries restore as learned on the mapped ports; the direct
    // entries the answer re-states are refused, staying exactly as before.
    let route = table.lookup(3000).unwrap();
    assert_eq!(route.port_index, 1);
    assert!(!route.directly_connected);
    assert!(table.lookup(1000).unwrap().directly_connected);
    assert!(table.lookup(2000).unwrap().directly_connected);
}
