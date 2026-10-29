//! The routing table's routes: direct and learned, lookup, listing, marks,
//! the freshness split and aging. Ported from `bacnet-network`'s
//! `router_table_tests.rs` and the freshness cases of
//! `router/rb06_convergence_tests.rs`, plus storage cases of the core's own.

mod support;

use std::collections::BTreeMap;

use bacnet_router_core::PortId;
use bacnet_router_core::npdu::Mac;
use bacnet_router_core::npdu::RejectMessageReason;
use bacnet_router_core::table::{LearnRefusal, Learned, Reachability, RejectEffect, Table};
use proptest::prelude::*;
use support::{BASE, Hosted, NANO, TIMING, mac, port, purge, secs};

#[test]
fn add_direct_and_lookup() {
    let mut table = Hosted::new();
    assert!(table.add_direct(1000, port(0)));

    let entry = table.lookup(1000).unwrap();
    assert!(entry.is_direct());
    assert_eq!(entry.port(), port(0));
    assert!(entry.next_hop().is_empty());
}

#[test]
fn add_learned_route() {
    let mut table = Hosted::new();
    let next_hop = mac(&[192, 168, 1, 100, 0xBA, 0xC0]);
    assert!(table.add_learned(2000, port(0), next_hop, BASE));

    let entry = table.lookup(2000).unwrap();
    assert!(!entry.is_direct());
    assert_eq!(entry.port(), port(0));
    assert_eq!(*entry.next_hop(), next_hop);
}

#[test]
fn lookup_unknown_returns_none() {
    let table = Hosted::new();
    assert!(table.lookup(9999).is_none());
}

#[test]
fn remove_route() {
    let mut table = Hosted::new();
    table.add_direct(1000, port(0));
    assert_eq!(table.len(), 1);

    let removed = table.remove(1000);
    assert!(removed.is_some());
    assert!(table.is_empty());
}

#[test]
fn networks_on_port() {
    let mut table = Hosted::new();
    table.add_direct(1000, port(0));
    table.add_direct(2000, port(1));
    table.add_learned(3000, port(0), mac(&[1, 2, 3]), BASE);

    let port0: Vec<u16> = table.networks_on_port(port(0)).collect();
    assert_eq!(port0, [1000, 3000]);

    let port1: Vec<u16> = table.networks_on_port(port(1)).collect();
    assert_eq!(port1, [2000]);
}

#[test]
fn list_all_networks() {
    let mut table = Hosted::new();
    table.add_direct(300, port(2));
    table.add_direct(100, port(0));
    table.add_direct(200, port(1));

    // Ascending, whatever the order they came in.
    assert_eq!(table.networks().collect::<Vec<_>>(), [100, 200, 300]);
}

#[test]
fn learned_route_does_not_override_direct() {
    let mut table = Hosted::new();
    table.add_direct(1000, port(0));

    let entry = table.lookup(1000).unwrap();
    assert!(entry.is_direct());
    assert_eq!(entry.port(), port(0));

    // add_learned does not overwrite a direct route.
    assert!(!table.add_learned(1000, port(1), mac(&[10, 0, 1, 1]), BASE));

    let entry = table.lookup(1000).unwrap();
    assert!(entry.is_direct());
    assert_eq!(entry.port(), port(0));
    assert!(entry.next_hop().is_empty());
}

#[test]
fn add_learned_overwrites_existing_learned() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(0), mac(&[10, 0, 1, 1]), BASE);

    let entry = table.lookup(3000).unwrap();
    assert!(!entry.is_direct());
    assert_eq!(entry.next_hop().as_slice(), &[10, 0, 1, 1]);

    table.add_learned(3000, port(1), mac(&[10, 0, 2, 1]), BASE);

    let entry = table.lookup(3000).unwrap();
    assert!(!entry.is_direct());
    assert_eq!(entry.port(), port(1));
    assert_eq!(entry.next_hop().as_slice(), &[10, 0, 2, 1]);
}

#[test]
fn lookup_unknown_network_returns_none() {
    let mut table = Hosted::new();
    table.add_direct(1000, port(0));
    table.add_direct(2000, port(1));

    assert!(table.lookup(9999).is_none());
}

#[test]
fn purge_stale_routes() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(0), mac(&[1, 2, 3]), BASE);
    assert_eq!(purge(&mut table, BASE + NANO, secs(0)), [3000]);
    assert!(table.lookup(3000).is_none());
}

#[test]
fn direct_routes_never_expire() {
    let mut table = Hosted::new();
    table.add_direct(1000, port(0));
    assert!(purge(&mut table, BASE + secs(1_000_000), secs(0)).is_empty());
    assert!(table.lookup(1000).is_some());
}

#[test]
fn touch_refreshes_timestamp() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(0), mac(&[1, 2, 3]), BASE);
    table.touch(3000, BASE);
    assert!(purge(&mut table, BASE, secs(3600)).is_empty());
    assert!(table.lookup(3000).is_some());
}

#[test]
fn learned_route_has_last_seen() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(0), mac(&[1, 2, 3]), BASE);
    assert_eq!(table.lookup(3000).unwrap().last_seen(), Some(BASE));
}

#[test]
fn direct_route_has_no_last_seen() {
    let mut table = Hosted::new();
    table.add_direct(1000, port(0));
    assert!(table.lookup(1000).unwrap().last_seen().is_none());
}

#[test]
fn networks_not_on_port_excludes_requesting_port() {
    let mut table = Hosted::new();
    table.add_direct(1000, port(0));
    table.add_direct(2000, port(1));
    table.add_learned(3000, port(1), mac(&[10, 0, 1, 1]), BASE);
    table.add_learned(4000, port(0), mac(&[10, 0, 2, 1]), BASE);

    let nets: Vec<u16> = table.networks_not_on_port(port(0)).collect();
    assert_eq!(nets, [2000, 3000]);

    let nets: Vec<u16> = table.networks_not_on_port(port(1)).collect();
    assert_eq!(nets, [1000, 4000]);
}

#[test]
fn add_learned_flap_inserts_new_route() {
    let mut table = Hosted::new();
    let result = table.learn(3000, port(0), mac(&[10, 0, 1, 1]), BASE);
    assert_eq!(result, Learned::Applied);
    assert_eq!(table.lookup(3000).unwrap().port(), port(0));
}

#[test]
fn add_learned_flap_refreshes_same_port() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(0), mac(&[10, 0, 1, 1]), BASE);
    let result = table.learn(3000, port(0), mac(&[10, 0, 1, 2]), BASE);
    assert_eq!(result, Learned::Applied);
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.next_hop().as_slice(), &[10, 0, 1, 2]);
}

#[test]
fn add_learned_flap_always_updates_different_port() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(0), mac(&[10, 0, 1, 1]), BASE);
    // Clause 6.6.3.2: the last claim wins, whichever port it came on.
    let result = table.learn(3000, port(1), mac(&[10, 0, 2, 1]), BASE);
    assert_eq!(
        result,
        Learned::Moved {
            from: port(0),
            flaps: 1
        }
    );
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.port(), port(1));
    assert_eq!(entry.next_hop().as_slice(), &[10, 0, 2, 1]);
}

#[test]
fn add_learned_flap_increments_flap_count() {
    let mut table = Hosted::new();
    let _ = table.learn(3000, port(0), mac(&[10, 0, 1, 1]), BASE);
    let _ = table.learn(3000, port(1), mac(&[10, 0, 2, 1]), BASE);
    assert_eq!(table.lookup(3000).unwrap().flap_count(), 1);
    let result = table.learn(3000, port(0), mac(&[10, 0, 1, 1]), BASE);
    assert_eq!(table.lookup(3000).unwrap().flap_count(), 2);
    assert!(result.applied() && !result.flapping());
    // The third move within the window is reported.
    assert!(table.learn(3000, port(1), Mac::EMPTY, BASE).flapping());
    assert_eq!(table.claim_counters().flap_warned, 1);
}

#[test]
fn add_learned_flap_rejects_direct_route() {
    let mut table = Hosted::new();
    table.add_direct(1000, port(0));
    let result = table.learn(1000, port(1), mac(&[10, 0, 2, 1]), BASE);
    assert_eq!(result, Learned::Refused(LearnRefusal::Direct));
    assert!(table.lookup(1000).unwrap().is_direct());
}

#[test]
fn the_flap_window_is_sixty_seconds_exclusive() {
    // A move just inside 60 s of the last counts toward the warning.
    let mut table = Hosted::new();
    let _ = table.learn(3000, port(0), Mac::EMPTY, BASE);
    let _ = table.learn(3000, port(1), Mac::EMPTY, BASE);
    let inside = table.learn(3000, port(0), Mac::EMPTY, BASE + secs(60) - NANO);
    assert_eq!(
        inside,
        Learned::Moved {
            from: port(1),
            flaps: 2
        }
    );
    // One exactly 60 s after starts the count again.
    let mut table = Hosted::new();
    let _ = table.learn(3000, port(0), Mac::EMPTY, BASE);
    let _ = table.learn(3000, port(1), Mac::EMPTY, BASE);
    let outside = table.learn(3000, port(0), Mac::EMPTY, BASE + secs(60));
    assert_eq!(
        outside,
        Learned::Moved {
            from: port(1),
            flaps: 1
        }
    );
    assert_eq!(table.lookup(3000).unwrap().flap_count(), 1);
}

#[test]
fn mark_busy_sets_reachability_and_deadline() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(0), mac(&[1, 2, 3]), BASE);
    let deadline = BASE + secs(30);
    table.mark_busy(3000, deadline);
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.reachability(), Reachability::Busy);
    assert_eq!(entry.busy_until(), Some(deadline));
}

#[test]
fn mark_available_clears_busy() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(0), mac(&[1, 2, 3]), BASE);
    table.mark_busy(3000, BASE + secs(30));
    table.mark_available(3000);
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.reachability(), Reachability::Reachable);
    assert!(entry.busy_until().is_none());
}

#[test]
fn mark_unreachable_keeps_entry() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(0), mac(&[1, 2, 3]), BASE);
    table.mark_unreachable(3000);
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.reachability(), Reachability::Unreachable);
}

#[test]
fn mark_unreachable_does_not_affect_direct_routes() {
    let mut table = Hosted::new();
    table.add_direct(1000, port(0));
    table.mark_unreachable(1000);
    let entry = table.lookup(1000).unwrap();
    assert_eq!(entry.reachability(), Reachability::Reachable);
}

#[test]
fn clear_expired_busy_clears_elapsed_deadlines() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(0), mac(&[1, 2, 3]), BASE);
    table.mark_busy(3000, BASE - secs(1));
    assert_eq!(table.clear_expired_busy(BASE), 1);
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.reachability(), Reachability::Reachable);
    assert!(entry.busy_until().is_none());
}

#[test]
fn clear_expired_busy_clears_a_deadline_of_exactly_now() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(0), mac(&[1]), BASE);
    table.mark_busy(3000, BASE);
    // Just before the deadline the mark stays; at it, the sweep clears it.
    assert_eq!(table.clear_expired_busy(BASE - NANO), 0);
    assert_eq!(
        table.lookup(3000).unwrap().reachability(),
        Reachability::Busy
    );
    assert_eq!(table.clear_expired_busy(BASE), 1);
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.reachability(), Reachability::Reachable);
    assert!(entry.busy_until().is_none());
}

#[test]
fn effective_reachability_checks_deadline_inline() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(0), mac(&[1, 2, 3]), BASE);
    table.mark_busy(3000, BASE - secs(1));
    assert_eq!(
        table.effective_reachability(3000, BASE),
        Some(Reachability::Reachable)
    );
}

#[test]
fn effective_reachability_returns_busy_when_deadline_not_elapsed() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(0), mac(&[1, 2, 3]), BASE);
    table.mark_busy(3000, BASE + secs(30));
    assert_eq!(
        table.effective_reachability(3000, BASE),
        Some(Reachability::Busy)
    );
}

// --- The freshness split (rb06_convergence_tests.rs) ---

#[test]
fn touch_updates_used_only_and_expiry_keys_off_seen() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(0), mac(&[1]), BASE);
    assert!(table.lookup(3000).unwrap().last_used().is_none());
    for i in 1..=100 {
        table.touch(3000, BASE + secs(i));
    }
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.last_seen(), Some(BASE));
    assert_eq!(entry.last_used(), Some(BASE + secs(100)));
    // Sustained traffic did not slide expiry: the 300 s rescue still fires.
    assert_eq!(purge(&mut table, BASE + secs(301), secs(300)), [3000]);
    assert!(table.lookup(3000).is_none());
}

#[test]
fn expiry_without_traffic_fires_on_seen_and_directs_never_expire() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(0), mac(&[1]), BASE);
    table.add_direct(1000, port(0));
    table.touch(1000, BASE + secs(10));
    assert!(table.lookup(1000).unwrap().last_seen().is_none());
    assert!(table.lookup(1000).unwrap().last_used().is_none());
    // At exactly the age the route stays; past it, it goes.
    assert!(purge(&mut table, BASE + secs(300), secs(300)).is_empty());
    assert_eq!(purge(&mut table, BASE + secs(301), secs(300)), [3000]);
    assert!(table.lookup(1000).is_some());
}

#[test]
fn same_port_refresh_updates_seen_preserves_used() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(0), mac(&[1]), BASE);
    table.touch(3000, BASE + secs(10));
    assert!(
        table
            .apply_claim(3000, port(0), mac(&[2]), BASE + secs(20), &TIMING)
            .applied()
    );
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.last_seen(), Some(BASE + secs(20)));
    assert_eq!(entry.last_used(), Some(BASE + secs(10)));
    assert_eq!(entry.next_hop().as_slice(), &[2]);
}

#[test]
fn busy_available_touch_neither_timestamp() {
    let mut table = Hosted::new();
    table.add_learned(3000, port(0), mac(&[1]), BASE);
    table.touch(3000, BASE + secs(5));
    table.mark_busy(3000, BASE + secs(35));
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.last_seen(), Some(BASE));
    assert_eq!(entry.last_used(), Some(BASE + secs(5)));
    table.mark_available(3000);
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.last_seen(), Some(BASE));
    assert_eq!(entry.last_used(), Some(BASE + secs(5)));
}

// --- Management and wire Port IDs (init_routing_table_tests.rs) ---

#[test]
fn wire_port_id_mapping_is_stable_nonzero_and_checked() {
    assert_eq!(PortId(0).wire_id(), Some(1));
    assert_eq!(PortId(1).wire_id(), Some(2));
    assert_eq!(PortId(255).wire_id(), None);
    assert_eq!(PortId::from_wire_id(1, 2), Some(PortId(0)));
    assert_eq!(PortId::from_wire_id(2, 2), Some(PortId(1)));
    // Port ID 0 is the delete request, never a real port.
    assert_eq!(PortId::from_wire_id(0, 2), None);
    // Past the local port count names no local port.
    assert_eq!(PortId::from_wire_id(3, 2), None);
    assert_eq!(PortId::from_wire_id(255, 2), None);
    assert_eq!(PortId::from_wire_id(255, 255), Some(PortId(254)));
}

#[test]
fn management_updates_never_touch_direct_routes() {
    let mut table = Hosted::new();
    table.add_direct(1000, port(0));
    table.add_learned(3000, port(0), mac(&[1]), BASE);
    table.touch(3000, BASE + secs(1));

    // An update replaces a learned row at once, or adds one.
    assert!(table.apply_management_update(3000, port(1), mac(&[2]), BASE + secs(2)));
    assert!(table.apply_management_update(4000, port(1), mac(&[3]), BASE + secs(2)));
    let entry = table.lookup(3000).unwrap();
    assert_eq!((entry.port(), *entry.next_hop()), (port(1), mac(&[2])));
    assert_eq!(entry.last_seen(), Some(BASE + secs(2)));
    assert_eq!(entry.last_used(), None);
    // Direct rows and reserved networks are left alone.
    assert!(!table.apply_management_update(1000, port(1), mac(&[2]), BASE));
    assert!(!table.apply_management_update(0, port(1), mac(&[2]), BASE));
    assert!(!table.apply_management_update(0xFFFF, port(1), mac(&[2]), BASE));
    assert!(table.lookup(1000).unwrap().is_direct());

    // Port ID 0 deletes a learned row, never a direct one.
    assert!(table.apply_management_removal(3000));
    assert!(!table.apply_management_removal(3000));
    assert!(!table.apply_management_removal(1000));
    assert_eq!(table.networks().collect::<Vec<_>>(), [1000, 4000]);
}

// --- Storage (new in the core) ---

#[test]
fn a_full_table_refuses_new_networks_and_keeps_serving_old_ones() {
    let mut table: Table<2, 2> = Table::new();
    assert_eq!(Table::<2, 2>::CAPACITY, 2);
    assert!(table.add_direct(1, port(0)));
    assert!(table.add_learned(9, port(1), mac(&[7]), BASE));
    assert!(table.is_full());
    assert!(!table.add_direct(2, port(0)));
    assert!(!table.add_learned(8, port(1), mac(&[7]), BASE));
    assert_eq!(
        table.learn(8, port(1), mac(&[7]), BASE),
        Learned::Refused(LearnRefusal::Full)
    );
    assert_eq!(
        table.apply_claim(8, port(1), mac(&[7]), BASE, &TIMING),
        Learned::Refused(LearnRefusal::Full)
    );
    // The three refusals stay apart on a full table.
    assert_eq!(
        table.learn(0xFFFF, port(1), mac(&[7]), BASE),
        Learned::Refused(LearnRefusal::Reserved)
    );
    assert_eq!(
        table.learn(1, port(1), mac(&[7]), BASE),
        Learned::Refused(LearnRefusal::Direct)
    );
    // Refreshing or moving a stored network still works.
    assert!(table.add_learned(9, port(1), mac(&[6]), BASE + secs(1)));
    assert!(table.learn(9, port(0), mac(&[5]), BASE + secs(2)).applied());
    assert_eq!(table.networks().collect::<Vec<_>>(), [1, 9]);
    table.remove(1);
    assert!(table.add_direct(2, port(0)));
    assert_eq!(table.networks().collect::<Vec<_>>(), [2, 9]);
    // Network 0 and 0xFFFF are never stored.
    table.remove(2);
    assert!(!table.add_direct(0, port(0)));
    assert!(!table.add_direct(0xFFFF, port(0)));
    assert!(!table.add_learned(0xFFFF, port(0), Mac::EMPTY, BASE));
    assert_eq!(table.len(), 1);
}

#[test]
fn debug_shows_the_stored_routes_only() {
    let mut table: Table<64, 64> = Table::new();
    assert_eq!(
        format!("{table:?}").matches("Slot").count(),
        0,
        "an empty table prints no slots"
    );
    table.add_learned(3000, port(1), mac(&[7]), BASE);
    table.apply_reject(
        3000,
        port(1),
        RejectMessageReason::ROUTER_BUSY,
        BASE,
        &TIMING,
    );
    let text = format!("{table:?}");
    assert_eq!(text.matches("Slot").count(), 1, "{text}");
    assert_eq!(text.matches("Hold").count(), 1, "{text}");
    assert!(text.contains("network: 3000"), "{text}");
}

#[test]
fn a_panicking_purge_callback_leaves_the_table_whole() {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    let mut table = Hosted::new();
    table.add_direct(1000, port(0));
    for network in [3000, 3001, 3002] {
        table.add_learned(network, port(1), mac(&[7]), BASE);
    }
    table.apply_reject(
        3001,
        port(1),
        RejectMessageReason::ROUTER_BUSY,
        BASE,
        &TIMING,
    );
    let before: Vec<u16> = table.networks().collect();
    let mut calls = 0;
    let unwound = catch_unwind(AssertUnwindSafe(|| {
        table.purge_stale(BASE + secs(301), secs(300), |_| {
            calls += 1;
            if calls == 2 {
                panic!("callback");
            }
        })
    }));
    assert!(unwound.is_err());
    // Nothing was removed, so the routes, their order and the hold-down
    // records are all still there and a later sweep works as usual.
    assert_eq!(table.networks().collect::<Vec<_>>(), before);
    // 3001's hold-down is still kept: a change from that port is held off.
    let mut probe = table.clone();
    assert_eq!(
        probe.apply_reject(
            3001,
            port(1),
            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
            BASE,
            &TIMING
        ),
        RejectEffect::HeldDown
    );
    assert_eq!(
        purge(&mut table, BASE + secs(301), secs(300)),
        [3000, 3001, 3002]
    );
    assert_eq!(table.networks().collect::<Vec<_>>(), [1000]);
}

/// An operation on the table, for the model test.
#[derive(Debug, Clone)]
enum Op {
    Direct(u16, u8),
    Learn(u16, u8),
    Remove(u16),
    Purge(u64),
}

fn op() -> impl Strategy<Value = Op> {
    let network = prop_oneof![1u16..=12, Just(0), Just(0xFFFF)];
    prop_oneof![
        (network.clone(), 0u8..3).prop_map(|(n, p)| Op::Direct(n, p)),
        (network.clone(), 0u8..3).prop_map(|(n, p)| Op::Learn(n, p)),
        network.prop_map(Op::Remove),
        (0u64..20).prop_map(Op::Purge),
    ]
}

proptest! {
    /// Against a sorted map of the same rules, the table holds the same
    /// routes in the same order after any sequence of changes, and never more
    /// than it has room for.
    #[test]
    fn routes_stay_sorted_and_bounded(ops in prop::collection::vec(op(), 0..60)) {
        let mut table: Table<8, 4> = Table::new();
        // network -> (port, direct, last_seen seconds)
        let mut model: BTreeMap<u16, (u8, bool, u64)> = BTreeMap::new();
        for (step, op) in ops.into_iter().enumerate() {
            let now = BASE + secs(step as u64);
            let reserved = |n: u16| n == 0 || n == 0xFFFF;
            match op {
                Op::Direct(n, p) => {
                    let room = model.contains_key(&n) || model.len() < 8;
                    prop_assert_eq!(table.add_direct(n, PortId(p)), !reserved(n) && room);
                    if !reserved(n) && room {
                        model.insert(n, (p, true, 0));
                    }
                }
                Op::Learn(n, p) => {
                    let existing = model.get(&n).copied();
                    let room = existing.is_some() || model.len() < 8;
                    let installs = !reserved(n) && room && !existing.is_some_and(|e| e.1);
                    prop_assert_eq!(table.add_learned(n, PortId(p), mac(&[p]), now), installs);
                    if installs {
                        model.insert(n, (p, false, step as u64));
                    }
                }
                Op::Remove(n) => {
                    prop_assert_eq!(table.remove(n).is_some(), model.remove(&n).is_some());
                }
                Op::Purge(age) => {
                    let mut purged = Vec::new();
                    table.purge_stale(now, secs(age), |n| purged.push(n));
                    let stale: Vec<u16> = model
                        .iter()
                        .filter(|(_, (_, direct, seen))| !direct && step as u64 - seen > age)
                        .map(|(n, _)| *n)
                        .collect();
                    for n in &stale {
                        model.remove(n);
                    }
                    prop_assert_eq!(purged, stale);
                }
            }
            let stored: Vec<(u16, u8, bool)> = table
                .routes()
                .map(|(n, route)| (n, route.port().0, route.is_direct()))
                .collect();
            let expected: Vec<(u16, u8, bool)> =
                model.iter().map(|(n, (p, direct, _))| (*n, *p, *direct)).collect();
            prop_assert_eq!(stored, expected);
            prop_assert!(table.len() <= Table::<8, 4>::CAPACITY);
        }
    }
}
