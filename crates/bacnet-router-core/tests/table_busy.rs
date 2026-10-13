//! Flow-control marks at the table: the scope of a Router-Busy or
//! Router-Available from one router, and busy deadlines. Ported from the
//! table-level cases of `bacnet-network`'s `router/busy_scope_tests.rs`,
//! plus the bulk marks for a message that lists no network.

mod support;

use bacnet_router_core::table::Reachability;
use support::{mac, port, secs, Hosted, BASE};

/// Peer A serves 3000 and 3001 on port 1.
const PEER_A: &[u8] = &[2];
/// Peer B shares port 1 but serves only 4000: same port, other router.
const PEER_B: &[u8] = &[3];

#[test]
fn selector_covers_only_learned_via_peer_routes() {
    let mut table = Hosted::new();
    table.add_direct(1000, port(0));
    table.add_direct(2000, port(1));
    table.add_learned(3001, port(1), mac(PEER_A), BASE);
    table.add_learned(3000, port(1), mac(PEER_A), BASE);
    table.add_learned(4000, port(1), mac(PEER_B), BASE);
    table.add_learned(5000, port(0), mac(&[9]), BASE);

    let served: Vec<u16> = table.routes_served_via_peer(port(1), PEER_A).collect();
    assert_eq!(served, [3000, 3001]);

    assert!(table.is_served_via_peer(3000, port(1), PEER_A));
    // Same port, other peer: served by a different router.
    assert!(!table.is_served_via_peer(4000, port(1), PEER_A));
    // Other port, even with the announcing MAC.
    assert!(!table.is_served_via_peer(5000, port(1), PEER_A));
    assert!(!table.is_served_via_peer(5000, port(0), PEER_A));
    // Directly attached paths are never covered.
    assert!(!table.is_served_via_peer(2000, port(1), PEER_A));
    assert!(!table.is_served_via_peer(1000, port(0), &[]));
    // Unknown networks have no route to mark.
    assert!(!table.is_served_via_peer(9999, port(1), PEER_A));
}

#[test]
fn busy_and_available_never_override_direct_routes() {
    let mut table = Hosted::new();
    table.add_direct(1000, port(0));
    table.mark_busy(1000, BASE + secs(30));
    assert_eq!(
        table.effective_reachability(1000, BASE),
        Some(Reachability::Reachable)
    );
    assert!(table.lookup(1000).unwrap().busy_until().is_none());
    table.mark_unreachable(1000);
    table.mark_available(1000);
    assert_eq!(
        table.effective_reachability(1000, BASE),
        Some(Reachability::Reachable)
    );
}

#[test]
fn busy_expiry_boundary_and_sweep() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(1), mac(PEER_A), BASE);
    table.mark_busy(3000, BASE + secs(30));
    assert_eq!(
        table.effective_reachability(3000, BASE),
        Some(Reachability::Busy)
    );
    // The exact deadline already reads Reachable inline.
    assert_eq!(
        table.effective_reachability(3000, BASE + secs(30)),
        Some(Reachability::Reachable)
    );
    // Past deadlines clear on the sweep and leave no residue.
    table.mark_busy(3000, BASE - secs(1));
    assert_eq!(table.clear_expired_busy(BASE), 1);
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.reachability(), Reachability::Reachable);
    assert!(entry.busy_until().is_none());
    assert_eq!(table.clear_expired_busy(BASE), 0);
}

#[test]
fn later_busy_update_extends_deadline() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(1), mac(PEER_A), BASE);
    table.mark_busy(3000, BASE + secs(30));
    table.mark_busy(3000, BASE + secs(31));
    assert_eq!(
        table.lookup(3000).unwrap().busy_until(),
        Some(BASE + secs(31))
    );
}

#[test]
fn busy_never_revives_an_unreachable_route_and_available_does() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(1), mac(PEER_A), BASE);
    table.mark_unreachable(3000);
    table.mark_busy(3000, BASE + secs(30));
    assert_eq!(
        table.effective_reachability(3000, BASE + secs(31)),
        Some(Reachability::Unreachable)
    );
    assert!(table.lookup(3000).unwrap().busy_until().is_none());
    table.mark_available(3000);
    assert_eq!(
        table.effective_reachability(3000, BASE),
        Some(Reachability::Reachable)
    );
}

/// Peer A's two routes, peer B's one, a route through another port with
/// peer A's MAC, and a direct route on port 1.
fn served_table() -> Hosted {
    let mut table = Hosted::new();
    table.add_direct(2000, port(1));
    table.add_learned(3000, port(1), mac(PEER_A), BASE);
    table.add_learned(3001, port(1), mac(PEER_A), BASE);
    table.add_learned(4000, port(1), mac(PEER_B), BASE);
    table.add_learned(5000, port(0), mac(PEER_A), BASE);
    table
}

#[test]
fn a_busy_with_no_list_marks_every_route_via_the_peer() {
    let mut table = served_table();
    table.touch(3000, BASE + secs(1));
    assert_eq!(
        table.mark_busy_via_peer(port(1), PEER_A, BASE + secs(30)),
        2
    );
    for network in [3000, 3001] {
        let route = table.lookup(network).unwrap();
        assert_eq!(route.reachability(), Reachability::Busy);
        assert_eq!(route.busy_until(), Some(BASE + secs(30)));
    }
    // Other peers, other ports and direct routes stay as they were.
    for network in [2000, 4000, 5000] {
        let route = table.lookup(network).unwrap();
        assert_eq!(route.reachability(), Reachability::Reachable);
        assert!(route.busy_until().is_none());
    }
    // Freshness is untouched.
    let route = table.lookup(3000).unwrap();
    assert_eq!(route.last_seen(), Some(BASE));
    assert_eq!(route.last_used(), Some(BASE + secs(1)));
    // A later Busy moves the deadline.
    assert_eq!(
        table.mark_busy_via_peer(port(1), PEER_A, BASE + secs(40)),
        2
    );
    assert_eq!(
        table.lookup(3001).unwrap().busy_until(),
        Some(BASE + secs(40))
    );
    // No route via an unknown peer.
    assert_eq!(table.mark_busy_via_peer(port(1), &[9], BASE), 0);
}

#[test]
fn a_bulk_busy_never_revives_an_unreachable_route() {
    let mut table = served_table();
    table.mark_unreachable(3001);
    assert_eq!(
        table.mark_busy_via_peer(port(1), PEER_A, BASE + secs(30)),
        1
    );
    let route = table.lookup(3001).unwrap();
    assert_eq!(route.reachability(), Reachability::Unreachable);
    assert!(route.busy_until().is_none());
    // Not even once the other route's busy mark lapses.
    assert_eq!(
        table.effective_reachability(3001, BASE + secs(31)),
        Some(Reachability::Unreachable)
    );
    assert_eq!(
        table.effective_reachability(3000, BASE + secs(30)),
        Some(Reachability::Reachable)
    );
}

#[test]
fn an_available_with_no_list_lifts_every_mark_via_the_peer() {
    let mut table = served_table();
    table.mark_busy_via_peer(port(1), PEER_A, BASE + secs(30));
    table.mark_unreachable(3001);
    table.mark_busy(4000, BASE + secs(30));
    assert_eq!(table.mark_available_via_peer(port(1), PEER_A), 2);
    for network in [3000, 3001] {
        let route = table.lookup(network).unwrap();
        assert_eq!(route.reachability(), Reachability::Reachable);
        assert!(route.busy_until().is_none());
    }
    // Peer B's route keeps its own mark, and the direct route is untouched.
    assert_eq!(
        table.lookup(4000).unwrap().reachability(),
        Reachability::Busy
    );
    assert!(table.lookup(2000).unwrap().is_direct());
    assert_eq!(table.mark_available_via_peer(port(0), PEER_B), 0);
}
