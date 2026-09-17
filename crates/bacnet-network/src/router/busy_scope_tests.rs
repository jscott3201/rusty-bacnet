//! RB-04 Busy/Available routing semantics: via-peer scope for explicit and
//! omitted network lists (Clauses 6.4.5/6.4.6, 6.6.3.6/6.6.3.7).
//!
//! "If the 2-octet network numbers are omitted, it means the router wishes to
//! stop the flow of messages to all the networks it normally serves" (Busy;
//! Available re-enables "the flow of messages to all the networks it serves").
//! The omitted scope is the via-peer set — learned routes egressing the
//! ingress port toward the immediate source MAC, never every route and never
//! none — shared by both arms through [`RouterTable::is_served_via_peer`].
//! Explicit lists intersect with that set; directly-attached paths are never
//! overridden by Busy/Available.
//!
//! Timing uses table-seeded past/future deadlines only — no sleeps. Busy
//! applies immediately with a 30s deadline (no reject-style damping) while
//! learning freshness (`last_seen`) stays untouched; damping/corroboration
//! acceptance timing is unchanged. Operational note: Busy is a temporary
//! congestion signal that auto-clears; Unreachable is a permanent failure
//! that only Available (or fresh learning) lifts.

use std::time::{Duration, Instant};

use bacnet_encoding::npdu::NpduAddress;

use super::envelope_harness::*;
use super::*;

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

fn reachability(h: &Harness, net: u16) -> Option<ReachabilityStatus> {
    h.table.try_lock().unwrap().effective_reachability(net)
}

#[test]
fn selector_covers_only_learned_via_peer_routes() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);
    table.add_learned(3000, 1, MacAddr::from_slice(PEER_A));
    table.add_learned(3001, 1, MacAddr::from_slice(PEER_A));
    table.add_learned(4000, 1, MacAddr::from_slice(PEER_B));
    table.add_learned(5000, 0, MacAddr::from_slice(&[9]));

    let peer_a = MacAddr::from_slice(PEER_A);
    let mut served = table.routes_served_via_peer(1, &peer_a);
    served.sort_unstable();
    assert_eq!(served, vec![3000, 3001]);

    assert!(table.is_served_via_peer(3000, 1, &peer_a));
    // Same port, other peer: served by a different router.
    assert!(!table.is_served_via_peer(4000, 1, &peer_a));
    // Other port, even with the announcing MAC.
    assert!(!table.is_served_via_peer(5000, 1, &peer_a));
    assert!(!table.is_served_via_peer(5000, 0, &peer_a));
    // Directly-attached paths are never covered.
    assert!(!table.is_served_via_peer(2000, 1, &peer_a));
    assert!(!table.is_served_via_peer(1000, 0, &MacAddr::new()));
    // Unknown networks have no route to mark.
    assert!(!table.is_served_via_peer(9999, 1, &peer_a));
}

#[test]
fn busy_and_available_never_override_direct_routes() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.mark_busy(1000, Instant::now() + Duration::from_secs(30));
    assert_eq!(
        table.effective_reachability(1000),
        Some(ReachabilityStatus::Reachable)
    );
    assert!(table.lookup(1000).unwrap().busy_until.is_none());
    table.mark_unreachable(1000);
    table.mark_available(1000);
    assert_eq!(
        table.effective_reachability(1000),
        Some(ReachabilityStatus::Reachable)
    );
}

#[test]
fn busy_expiry_boundary_and_sweep() {
    let mut table = RouterTable::new();
    table.add_learned(3000, 1, MacAddr::from_slice(PEER_A));
    table.mark_busy(3000, Instant::now() + Duration::from_secs(30));
    assert_eq!(
        table.effective_reachability(3000),
        Some(ReachabilityStatus::Busy)
    );
    // Exact boundary (`now >= deadline`) already reads Reachable inline.
    table.mark_busy(3000, Instant::now());
    assert_eq!(
        table.effective_reachability(3000),
        Some(ReachabilityStatus::Reachable)
    );
    // Past deadlines clear on the sweep and leave no residue.
    table.mark_busy(3000, Instant::now() - Duration::from_secs(1));
    table.clear_expired_busy();
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.reachability, ReachabilityStatus::Reachable);
    assert!(entry.busy_until.is_none());
}

#[test]
fn later_busy_update_extends_deadline() {
    let mut table = RouterTable::new();
    table.add_learned(3000, 1, MacAddr::from_slice(PEER_A));
    let base = Instant::now();
    table.mark_busy(3000, base + Duration::from_secs(30));
    table.mark_busy(3000, base + Duration::from_secs(31));
    assert_eq!(
        table.lookup(3000).unwrap().busy_until,
        Some(base + Duration::from_secs(31))
    );
}

#[tokio::test]
async fn omitted_busy_marks_whole_via_peer_set_only() {
    let mut h = Harness::busy_scope();
    let mut ctx = h.ctx(1, PEER_A, control_npdu(BUSY, &[]));
    ctx.data_attributes = attributes();
    h.handle(ctx).await;

    // Per-route change AND no-change on one topology.
    assert_eq!(reachability(&h, 3000), Some(ReachabilityStatus::Busy));
    assert_eq!(reachability(&h, 3001), Some(ReachabilityStatus::Busy));
    for net in [4000, 5000, 1000, 2000] {
        assert_eq!(
            reachability(&h, net),
            Some(ReachabilityStatus::Reachable),
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
        } => {
            let decoded = decode_npdu(npdu).unwrap();
            assert_eq!(decoded.message_type, Some(BUSY.to_raw()));
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
    assert_eq!(reachability(&h, 3000), Some(ReachabilityStatus::Busy));
    for net in [3001, 4000, 5000, 1000, 2000] {
        assert_eq!(
            reachability(&h, net),
            Some(ReachabilityStatus::Reachable),
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

    assert_eq!(reachability(&h, 3001), Some(ReachabilityStatus::Busy));
    for net in [3000, 4000, 5000, 1000, 2000] {
        assert_eq!(
            reachability(&h, net),
            Some(ReachabilityStatus::Reachable),
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
    assert_eq!(reachability(&h, 3000), Some(ReachabilityStatus::Busy));

    // Re-enable before the 30s deadline: no waiting, state flips at once.
    h.handle(h.ctx(1, PEER_A, control_npdu(AVAILABLE, &busy_payload(&[3000]))))
        .await;
    assert_eq!(reachability(&h, 3000), Some(ReachabilityStatus::Reachable));
    assert!(h
        .table
        .try_lock()
        .unwrap()
        .lookup(3000)
        .unwrap()
        .busy_until
        .is_none());

    // Omitted Available clears the whole via-peer set and nothing else.
    h.handle(h.ctx(1, PEER_A, control_npdu(BUSY, &[]))).await;
    assert_eq!(reachability(&h, 3001), Some(ReachabilityStatus::Busy));
    h.handle(h.ctx(1, PEER_A, control_npdu(AVAILABLE, &[])))
        .await;
    for net in [3000, 3001, 4000, 5000, 1000, 2000] {
        assert_eq!(
            reachability(&h, net),
            Some(ReachabilityStatus::Reachable),
            "network {net} must be reachable"
        );
    }
}

#[tokio::test]
async fn unreachable_is_permanent_until_in_scope_available() {
    let mut h = Harness::busy_scope();
    h.table.try_lock().unwrap().mark_unreachable(3000);
    assert_eq!(
        reachability(&h, 3000),
        Some(ReachabilityStatus::Unreachable)
    );

    // The busy sweep never lifts a permanent failure.
    h.table.try_lock().unwrap().clear_expired_busy();
    assert_eq!(
        reachability(&h, 3000),
        Some(ReachabilityStatus::Unreachable)
    );

    // Out-of-scope Available (peer B serves 4000, not 3000) leaves it.
    h.handle(h.ctx(1, PEER_B, control_npdu(AVAILABLE, &busy_payload(&[3000]))))
        .await;
    assert_eq!(
        reachability(&h, 3000),
        Some(ReachabilityStatus::Unreachable)
    );

    // In-scope Available re-enables the flow for that DNET.
    h.handle(h.ctx(1, PEER_A, control_npdu(AVAILABLE, &busy_payload(&[3000]))))
        .await;
    assert_eq!(reachability(&h, 3000), Some(ReachabilityStatus::Reachable));
}

#[tokio::test]
async fn busy_never_resurrects_unreachable_route() {
    // Table level: (status, busy_until) untouched, so the sweep past the
    // deadline still finds a permanent failure — never an auto-cleared Busy.
    let mut table = RouterTable::new();
    table.add_learned(3000, 1, MacAddr::from_slice(PEER_A));
    table.mark_unreachable(3000);
    table.mark_busy(3000, Instant::now() + Duration::from_secs(30));
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.reachability, ReachabilityStatus::Unreachable);
    assert!(entry.busy_until.is_none());
    table.mark_busy(3000, Instant::now() - Duration::from_secs(1));
    table.clear_expired_busy();
    assert_eq!(
        table.effective_reachability(3000),
        Some(ReachabilityStatus::Unreachable)
    );

    // Handler level: in-scope explicit and omitted Busy never lift it, while
    // propagation stays verbatim.
    let mut h = Harness::busy_scope();
    h.table.try_lock().unwrap().mark_unreachable(3000);
    h.handle(h.ctx(1, PEER_A, control_npdu(BUSY, &busy_payload(&[3000]))))
        .await;
    assert_eq!(broadcast_data(h.drain(0)).len(), 1);
    h.handle(h.ctx(1, PEER_A, control_npdu(BUSY, &[]))).await;
    assert_eq!(broadcast_data(h.drain(0)).len(), 1);
    assert_eq!(
        reachability(&h, 3000),
        Some(ReachabilityStatus::Unreachable)
    );
    assert!(h
        .table
        .try_lock()
        .unwrap()
        .lookup(3000)
        .unwrap()
        .busy_until
        .is_none());
    h.table.try_lock().unwrap().clear_expired_busy();
    assert_eq!(
        reachability(&h, 3000),
        Some(ReachabilityStatus::Unreachable)
    );
    // 3001 (served, Reachable) still follows the normal omitted-Busy path.
    assert_eq!(reachability(&h, 3001), Some(ReachabilityStatus::Busy));
}

#[tokio::test]
async fn busy_marks_leave_learning_freshness_untouched() {
    let mut h = Harness::busy_scope();
    let before: Vec<(u16, _)> = {
        let table = h.table.try_lock().unwrap();
        [3000, 3001, 4000, 5000]
            .into_iter()
            .map(|net| (net, table.lookup(net).unwrap().last_seen))
            .collect()
    };
    h.handle(h.ctx(1, PEER_A, control_npdu(BUSY, &[]))).await;
    h.handle(h.ctx(1, PEER_A, control_npdu(AVAILABLE, &[])))
        .await;
    let table = h.table.try_lock().unwrap();
    for (net, seen) in before {
        assert_eq!(
            table.lookup(net).unwrap().last_seen,
            seen,
            "network {net} freshness must not move on Busy/Available"
        );
    }
}

#[tokio::test]
async fn queue_full_keeps_marks_bounded_and_local() {
    // Same topology as the shared fixture, but the rebroadcast target
    // (port 0) has a capacity-1 queue pre-filled to force a local drop.
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);
    table.add_learned(3000, 1, MacAddr::from_slice(PEER_A));
    table.add_learned(3001, 1, MacAddr::from_slice(PEER_A));
    table.add_learned(4000, 1, MacAddr::from_slice(PEER_B));
    table.add_learned(5000, 0, MacAddr::from_slice(&[9]));
    let table = Arc::new(Mutex::new(table));

    let (tx0, mut rx0) = mpsc::channel::<SendRequest>(1);
    let (tx1, mut rx1) = mpsc::channel::<SendRequest>(16);
    tx0.try_send(SendRequest::broadcast(Bytes::from_static(&[0])))
        .unwrap();

    let ctx = IngressContext::test_local(1, 2000, PEER_A, control_npdu(BUSY, &[]));
    handle_network_message(&table, &[tx0, tx1], &ctx).await;

    // Per-route state is correct despite the propagation drop; the drop is
    // bounded (no retry queue, no new admission path): port 0 still holds
    // only its filler, and the ingress port stays quiet.
    {
        let table = table.lock().await;
        assert_eq!(
            table.effective_reachability(3000),
            Some(ReachabilityStatus::Busy)
        );
        assert_eq!(
            table.effective_reachability(3001),
            Some(ReachabilityStatus::Busy)
        );
        for net in [4000, 5000, 1000, 2000] {
            assert_eq!(
                table.effective_reachability(net),
                Some(ReachabilityStatus::Reachable),
                "network {net} must stay reachable"
            );
        }
    }
    rx0.try_recv().unwrap();
    assert!(rx0.try_recv().is_err());
    assert!(rx1.try_recv().is_err());
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
            Some(ReachabilityStatus::Reachable),
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
