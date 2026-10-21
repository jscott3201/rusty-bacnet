//! RB-04 Busy/Available routing semantics: via-peer scope for explicit and
//! omitted network lists (Clauses 6.4.5/6.4.6, 6.6.3.6/6.6.3.7).
//!
//! A Busy with no network list asks that traffic be held back for every
//! network the router normally serves; an Available with no list releases
//! that hold for every network it serves.
//! The omitted scope is the via-peer set — learned routes egressing the
//! ingress port toward the immediate source MAC, never every route and never
//! none — shared by both arms through the table's `is_served_via_peer`.
//! Explicit lists intersect with that set; directly-attached paths are never
//! overridden by Busy/Available.
//!
//! The router runs on its engine through [`super::envelope_harness`]
//! (#1679); the table-level cases moved to `bacnet-router-core`'s
//! `tests/table_busy.rs` in #1688. Busy applies immediately with a 30s
//! deadline (no reject-style damping) while learning freshness
//! (`last_seen`) stays untouched. Operational note: Busy is a temporary
//! congestion signal that auto-clears; Unreachable is a permanent failure
//! that only Available (or fresh learning) lifts.

use std::sync::Arc;

use bacnet_encoding::npdu::{decode_npdu, NpduAddress};
use bacnet_npdu::NetworkMessageType;
use bacnet_types::MacAddr;
use bytes::Bytes;

use super::control_policy::ControlGate;
use super::envelope_harness::*;
use super::SendRequest;
use crate::router_table::Reachability;

const BUSY: NetworkMessageType = NetworkMessageType::ROUTER_BUSY_TO_NETWORK;
const AVAILABLE: NetworkMessageType = NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK;

/// Peer A serves 3000/3001 on port 1; used as the announcing ingress below.
const PEER_A: &[u8] = &[2];
/// Peer B shares port 1 but serves only 4000: same port, other peer.
const PEER_B: &[u8] = &[3];

fn busy_payload(nets: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nets.len() * 2);
    for net in nets {
        out.extend_from_slice(&net.to_be_bytes());
    }
    out
}

fn reachability(h: &Harness, net: u16) -> Option<Reachability> {
    h.table().effective_reachability(net, h.now())
}

#[tokio::test]
async fn omitted_busy_marks_whole_via_peer_set_only() {
    let mut h = Harness::busy_scope();
    let mut ctx = h.ctx(1, PEER_A, control_npdu(BUSY, &[]));
    ctx.data_attributes = attributes();
    h.handle(ctx).await;

    // Per-route change AND no-change on one topology.
    assert_eq!(reachability(&h, 3000), Some(Reachability::Busy));
    assert_eq!(reachability(&h, 3001), Some(Reachability::Busy));
    for net in [4000, 5000, 1000, 2000] {
        assert_eq!(
            reachability(&h, net),
            Some(Reachability::Reachable),
            "network {net} must stay reachable"
        );
    }

    // Propagation: verbatim empty payload to every other port, ingress
    // attributes carried, nothing reflected on the ingress port.
    assert!(h.drain(1).is_empty());
    let mut out = h.drain(0);
    assert_eq!(out.len(), 1);
    match out.pop().unwrap() {
        SendRequest::Broadcast {
            npdu,
            data_attributes,
            ..
        } => {
            let decoded = decode_npdu(npdu).unwrap();
            assert_eq!(decoded.message_type, Some(BUSY));
            assert!(decoded.payload.is_empty());
            assert_eq!(data_attributes, attributes());
        }
        SendRequest::Unicast { .. } => panic!("expected broadcast"),
    }
}

#[tokio::test]
async fn single_explicit_busy_marks_only_the_covered_route() {
    let mut h = Harness::busy_scope();
    h.handle(h.ctx(1, PEER_A, control_npdu(BUSY, &busy_payload(&[3000]))))
        .await;

    // Explicit [3000] marks 3000 but not 3001, although peer A serves both:
    // an explicit list is not the omitted whole-set scope.
    assert_eq!(reachability(&h, 3000), Some(Reachability::Busy));
    for net in [3001, 4000, 5000, 1000, 2000] {
        assert_eq!(
            reachability(&h, net),
            Some(Reachability::Reachable),
            "network {net} must stay reachable"
        );
    }
    assert_eq!(broadcast_data(h.drain(0)).len(), 1);
}

#[tokio::test]
async fn mixed_list_intersects_with_announcing_path() {
    let mut h = Harness::busy_scope();
    // 3001 is served via peer A; 4000 via peer B; 2000 is direct; 9999 is
    // unknown. Only 3001 may change, but propagation stays verbatim.
    let payload = busy_payload(&[3001, 4000, 2000, 9999]);
    h.handle(h.ctx(1, PEER_A, control_npdu(BUSY, &payload)))
        .await;

    assert_eq!(reachability(&h, 3001), Some(Reachability::Busy));
    for net in [3000, 4000, 5000, 1000, 2000] {
        assert_eq!(
            reachability(&h, net),
            Some(Reachability::Reachable),
            "network {net} must stay reachable"
        );
    }
    let rebroadcasts = broadcast_data(h.drain(0));
    assert_eq!(rebroadcasts.len(), 1);
    assert_eq!(
        decode_npdu(rebroadcasts[0].clone())
            .unwrap()
            .payload
            .as_ref(),
        payload.as_slice()
    );
}

#[tokio::test]
async fn available_reenables_before_expiry_and_omitted_clears_set() {
    let mut h = Harness::busy_scope();
    h.handle(h.ctx(1, PEER_A, control_npdu(BUSY, &busy_payload(&[3000]))))
        .await;
    assert_eq!(reachability(&h, 3000), Some(Reachability::Busy));

    // Re-enable before the 30s deadline: no waiting, state flips at once.
    h.handle(h.ctx(1, PEER_A, control_npdu(AVAILABLE, &busy_payload(&[3000]))))
        .await;
    assert_eq!(reachability(&h, 3000), Some(Reachability::Reachable));
    assert!(h.table().lookup(3000).unwrap().busy_until().is_none());

    // Omitted Available clears the whole via-peer set and nothing else.
    h.handle(h.ctx(1, PEER_A, control_npdu(BUSY, &[]))).await;
    assert_eq!(reachability(&h, 3001), Some(Reachability::Busy));
    h.handle(h.ctx(1, PEER_A, control_npdu(AVAILABLE, &[])))
        .await;
    for net in [3000, 3001, 4000, 5000, 1000, 2000] {
        assert_eq!(
            reachability(&h, net),
            Some(Reachability::Reachable),
            "network {net} must be reachable"
        );
    }
}

#[tokio::test]
async fn unreachable_is_permanent_until_in_scope_available() {
    let mut h = Harness::busy_scope();
    h.table().mark_unreachable(3000);
    assert_eq!(reachability(&h, 3000), Some(Reachability::Unreachable));

    // The busy sweep never lifts a permanent failure.
    let now = h.now();
    h.table().clear_expired_busy(now);
    assert_eq!(reachability(&h, 3000), Some(Reachability::Unreachable));

    // Out-of-scope Available (peer B serves 4000, not 3000) leaves it.
    h.handle(h.ctx(1, PEER_B, control_npdu(AVAILABLE, &busy_payload(&[3000]))))
        .await;
    assert_eq!(reachability(&h, 3000), Some(Reachability::Unreachable));

    // In-scope Available re-enables the flow for that DNET.
    h.handle(h.ctx(1, PEER_A, control_npdu(AVAILABLE, &busy_payload(&[3000]))))
        .await;
    assert_eq!(reachability(&h, 3000), Some(Reachability::Reachable));
}

#[tokio::test]
async fn busy_never_resurrects_unreachable_route() {
    // In-scope explicit and omitted Busy never lift it, while propagation
    // stays verbatim. The table-level case is the core's
    // `busy_never_revives_an_unreachable_route_and_available_does`.
    let mut h = Harness::busy_scope();
    h.table().mark_unreachable(3000);
    h.handle(h.ctx(1, PEER_A, control_npdu(BUSY, &busy_payload(&[3000]))))
        .await;
    assert_eq!(broadcast_data(h.drain(0)).len(), 1);
    h.handle(h.ctx(1, PEER_A, control_npdu(BUSY, &[]))).await;
    assert_eq!(broadcast_data(h.drain(0)).len(), 1);
    assert_eq!(reachability(&h, 3000), Some(Reachability::Unreachable));
    assert!(h.table().lookup(3000).unwrap().busy_until().is_none());
    let now = h.now();
    h.table().clear_expired_busy(now);
    assert_eq!(reachability(&h, 3000), Some(Reachability::Unreachable));
    // 3001 (served, Reachable) still follows the normal omitted-Busy path.
    assert_eq!(reachability(&h, 3001), Some(Reachability::Busy));
}

#[tokio::test]
async fn busy_marks_leave_learning_freshness_untouched() {
    let mut h = Harness::busy_scope();
    let before: Vec<(u16, _)> = {
        let table = h.table();
        [3000, 3001, 4000, 5000]
            .into_iter()
            .map(|net| (net, table.lookup(net).unwrap().last_seen()))
            .collect()
    };
    h.handle(h.ctx(1, PEER_A, control_npdu(BUSY, &[]))).await;
    h.handle(h.ctx(1, PEER_A, control_npdu(AVAILABLE, &[])))
        .await;
    let table = h.table();
    for (net, seen) in before {
        assert_eq!(
            table.lookup(net).unwrap().last_seen(),
            seen,
            "network {net} freshness must not move on Busy/Available"
        );
    }
}

#[tokio::test]
async fn queue_full_keeps_marks_bounded_and_local() {
    // Same topology as the shared fixture, but the rebroadcast target
    // (port 0) has a capacity-1 queue pre-filled to force a local drop.
    let mut h = Harness::with_capacities(Arc::new(ControlGate::permissive()), &[1, 16]);
    h.learned(3000, 1, PEER_A);
    h.learned(3001, 1, PEER_A);
    h.learned(4000, 1, PEER_B);
    h.learned(5000, 0, &[9]);
    h.txs[0]
        .try_send(
            bacnet_npdu::NetworkPriority::Normal,
            SendRequest::Broadcast {
                npdu: Bytes::from_static(&[0]),
                data_attributes: Vec::new(),

                reject_origin: None,
            },
        )
        .unwrap();

    h.handle(h.ctx(1, PEER_A, control_npdu(BUSY, &[]))).await;

    // Per-route state is correct despite the propagation drop; the drop is
    // bounded (no retry queue, no new admission path): port 0 still holds
    // only its filler, and the ingress port stays quiet.
    assert_eq!(reachability(&h, 3000), Some(Reachability::Busy));
    assert_eq!(reachability(&h, 3001), Some(Reachability::Busy));
    for net in [4000, 5000, 1000, 2000] {
        assert_eq!(
            reachability(&h, net),
            Some(Reachability::Reachable),
            "network {net} must stay reachable"
        );
    }
    h.rxs[0].try_recv().unwrap();
    h.assert_quiet();
}

#[tokio::test]
async fn directed_busy_is_routed_not_applied() {
    // RB-03 routing-first rule: a Busy addressed at another network is
    // forwarded toward its destination, never applied to the local table.
    let mut h = Harness::busy_scope();
    let mut npdu = control_npdu(BUSY, &busy_payload(&[3000]));
    npdu.destination = Some(NpduAddress {
        network: 5000,
        mac_address: MacAddr::from_slice(&[7]),
    });
    npdu.hop_count = 255;
    h.dispatch(h.ctx(1, PEER_A, npdu)).await;

    for net in [3000, 3001, 4000, 5000, 1000, 2000] {
        assert_eq!(
            reachability(&h, net),
            Some(Reachability::Reachable),
            "network {net} must stay reachable"
        );
    }
    assert!(h.drain(1).is_empty());
    let mut forwarded = h.drain(0);
    assert_eq!(forwarded.len(), 1);
    match forwarded.pop().unwrap() {
        SendRequest::Unicast { npdu, mac, .. } => {
            assert_eq!(mac.as_slice(), &[9]);
            let decoded = decode_npdu(npdu).unwrap();
            assert_eq!(decoded.destination.unwrap().network, 5000);
            assert_eq!(decoded.hop_count, 254);
        }
        SendRequest::Broadcast { .. } => panic!("expected unicast forward"),
    }
}
