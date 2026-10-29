//! Received rejects and the hold-down. Ported from `bacnet-network`'s
//! `router_table_reject_tests.rs`, plus the core's bounded record pool.
//! Unit tests, since the hold-down records they inspect are the crate's
//! own.

use std::vec::Vec;

use super::support::{
    BASE, HOLD_DOWN, Hosted, NANO, TIMING, held_networks, holds_for, learned_table, mac, port,
    purge, secs,
};
use crate::npdu::RejectMessageReason;
use crate::table::holds::Hold;
use crate::table::{ClaimCounters, Reachability, RejectEffect, Table, Timing};
use crate::{Mac, PortId};

#[test]
fn reject_spam_is_idempotent_without_refreshing_route_or_window() {
    let mut table = learned_table();
    let seen = table.lookup(3000).unwrap().last_seen;
    let now = BASE;
    assert_eq!(
        table.apply_reject(
            3000,
            port(0),
            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
            now,
            &TIMING
        ),
        RejectEffect::Applied
    );
    for second in 1..100 {
        assert_eq!(
            table.apply_reject(
                3000,
                port(0),
                RejectMessageReason::NOT_DIRECTLY_CONNECTED,
                now + secs(second),
                &TIMING
            ),
            RejectEffect::SameState
        );
    }
    assert_eq!(table.lookup(3000).unwrap().last_seen, seen);
    assert_eq!(
        table.lookup(3000).unwrap().reachability,
        Reachability::Unreachable
    );
    assert_eq!(table.held_since(3000, port(0)), Some(now));
    assert_eq!(
        table.claim_counters(),
        ClaimCounters {
            reject_applied: 1,
            reject_dampened: 99,
            reject_dampened_same_state: 99,
            ..Default::default()
        }
    );
}

#[test]
fn alternating_rejects_hold_until_exact_boundary_without_sliding() {
    for first in [1u8, 2] {
        let mut table = learned_table();
        let now = BASE;
        let status = if first == 1 {
            Reachability::Unreachable
        } else {
            Reachability::Busy
        };
        for second in 0..10 {
            let reason = if second % 2 == 0 { first } else { 3 - first };
            table.apply_reject(
                3000,
                port(0),
                RejectMessageReason::from_raw(reason),
                now + secs(second),
                &TIMING,
            );
            assert_eq!(table.lookup(3000).unwrap().reachability, status);
        }
        table.apply_reject(
            3000,
            port(0),
            RejectMessageReason::from_raw(3 - first),
            now + HOLD_DOWN - NANO,
            &TIMING,
        );
        assert_eq!(table.lookup(3000).unwrap().reachability, status);
        assert_eq!(table.held_since(3000, port(0)), Some(now));
        if first == 2 {
            assert_eq!(
                table.lookup(3000).unwrap().busy_until,
                Some(now + HOLD_DOWN)
            );
        }
        table.apply_reject(
            3000,
            port(0),
            RejectMessageReason::from_raw(3 - first),
            now + HOLD_DOWN,
            &TIMING,
        );
        // A Busy claim never lifts permanent Unreachable, even at the
        // hold-down boundary: only Available or fresh learning exits it, so
        // the window never slides on that claim.
        if first == 2 {
            assert_ne!(table.lookup(3000).unwrap().reachability, status);
            assert_eq!(table.held_since(3000, port(0)), Some(now + HOLD_DOWN));
            assert_eq!(
                table.claim_counters(),
                ClaimCounters {
                    reject_applied: 2,
                    reject_dampened: 10,
                    reject_dampened_same_state: 4,
                    reject_dampened_hold_down: 6,
                    ..Default::default()
                }
            );
        } else {
            assert_eq!(table.lookup(3000).unwrap().reachability, status);
            assert_eq!(table.held_since(3000, port(0)), Some(now));
            assert_eq!(
                table.claim_counters(),
                ClaimCounters {
                    reject_applied: 1,
                    reject_dampened: 11,
                    reject_dampened_same_state: 11,
                    ..Default::default()
                }
            );
        }
    }
}

#[test]
fn busy_duplicates_do_not_renew_deadline_and_expired_busy_can_be_rejected_again() {
    let mut table = learned_table();
    let now = BASE;
    let deadline = now + secs(10);
    table.mark_busy(3000, deadline); // A mark with no reject record.
    table.apply_reject(
        3000,
        port(0),
        RejectMessageReason::ROUTER_BUSY,
        now,
        &TIMING,
    );
    assert_eq!(table.lookup(3000).unwrap().busy_until, Some(deadline));
    assert_eq!(table.holds().count(), 0);
    table.apply_reject(
        3000,
        port(0),
        RejectMessageReason::ROUTER_BUSY,
        deadline,
        &TIMING,
    ); // Lapsed inline, no sweep.
    assert_eq!(
        table.lookup(3000).unwrap().busy_until,
        Some(deadline + HOLD_DOWN)
    );
    table.apply_reject(
        3000,
        port(0),
        RejectMessageReason::ROUTER_BUSY,
        deadline + HOLD_DOWN - NANO,
        &TIMING,
    );
    table.apply_reject(
        3000,
        port(0),
        RejectMessageReason::ROUTER_BUSY,
        deadline + HOLD_DOWN,
        &TIMING,
    );
    assert_eq!(
        table.lookup(3000).unwrap().busy_until,
        Some(deadline + HOLD_DOWN + HOLD_DOWN)
    );
    assert_eq!(
        table.claim_counters(),
        ClaimCounters {
            reject_applied: 2,
            reject_dampened: 2,
            reject_dampened_same_state: 2,
            ..Default::default()
        }
    );
}

/// Clause 6.6.3.5 changes the table for reasons 1 and 2 only (#1679): any
/// other reason leaves a learned route as it was, starts no
/// hold-down, and counts as kept, apart from the dampened rejects, even
/// within a hold-down.
#[test]
fn every_other_reason_keeps_a_learned_route() {
    for reason in (0..=u8::MAX).filter(|reason| ![1, 2].contains(reason)) {
        let mut table = learned_table();
        let before = table.lookup(3000).unwrap();
        assert_eq!(
            table.apply_reject(
                3000,
                port(0),
                RejectMessageReason::from_raw(reason),
                BASE,
                &TIMING
            ),
            RejectEffect::Kept
        );
        assert_eq!(table.lookup(3000), Some(before));
        assert_eq!(table.holds().count(), 0);

        table.apply_reject(
            3000,
            port(0),
            RejectMessageReason::ROUTER_BUSY,
            BASE,
            &TIMING,
        );
        let busy = table.lookup(3000).unwrap();
        assert_eq!(
            table.apply_reject(
                3000,
                port(0),
                RejectMessageReason::from_raw(reason),
                BASE + secs(1),
                &TIMING
            ),
            RejectEffect::Kept
        );
        assert_eq!(table.lookup(3000), Some(busy));
        assert_eq!(table.held_since(3000, port(0)), Some(BASE));
        assert_eq!(
            table.claim_counters(),
            ClaimCounters {
                reject_applied: 1,
                reject_kept: 2,
                ..Default::default()
            }
        );
    }
}

#[test]
fn learning_refresh_replacement_removal_and_aging_rearm_all_ingress_keys() {
    for rearm in [
        "same_peer",
        "new_mac",
        "new_port",
        "manual_learning",
        "remove",
        "age",
    ] {
        let mut table = learned_table();
        let now = BASE;
        // Busy first, then Unreachable: both transitions apply and record
        // their arrival port (the reverse order would swallow the Busy
        // claim, since Busy never lifts Unreachable).
        table.apply_reject(
            3000,
            port(0),
            RejectMessageReason::ROUTER_BUSY,
            now,
            &TIMING,
        );
        table.apply_reject(
            3000,
            port(1),
            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
            now,
            &TIMING,
        );
        assert_eq!(holds_for(&table, 3000), 2);
        match rearm {
            "same_peer" => {
                assert!(table.learn(3000, port(0), mac(&[1]), now).applied());
            }
            "new_mac" => {
                assert!(table.learn(3000, port(0), mac(&[2]), now).applied());
            }
            "new_port" => {
                assert!(table.learn(3000, port(1), mac(&[1]), now).applied());
            }
            "manual_learning" => {
                table.add_learned(3000, port(0), Mac::EMPTY, now);
            }
            "remove" => {
                assert!(table.remove(3000).is_some());
                assert_eq!(table.holds().count(), 0);
                table.add_learned(3000, port(0), Mac::EMPTY, now);
            }
            "age" => {
                table.slot_mut(3000).unwrap().route.last_seen = Some(now - secs(120));
                assert_eq!(purge(&mut table, now, secs(60)), [3000]);
                assert_eq!(table.holds().count(), 0);
                table.add_learned(3000, port(0), Mac::EMPTY, now);
            }
            _ => unreachable!(),
        }
        assert_eq!(table.holds().count(), 0, "{rearm}");
        table.apply_reject(
            3000,
            port(0),
            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
            now + secs(1),
            &TIMING,
        );
        table.apply_reject(
            3000,
            port(1),
            RejectMessageReason::ROUTER_BUSY,
            now + secs(1),
            &TIMING,
        );
        assert_eq!(
            table.claim_counters(),
            ClaimCounters {
                reject_applied: 3,
                // The second claim finds a permanently Unreachable route, so
                // it dampens as same-state instead of applying.
                reject_dampened: 1,
                reject_dampened_same_state: 1,
                ..Default::default()
            },
            "{rearm}"
        );
    }
}

#[test]
fn reject_keys_are_independent_by_ingress_port_and_network() {
    let mut table = learned_table();
    table.add_learned(4000, port(0), mac(&[1]), BASE);
    let now = BASE;
    for (network, ingress, reason) in [(3000, 0, 2), (4000, 0, 2), (3000, 1, 1), (4000, 1, 1)] {
        table.apply_reject(
            network,
            port(ingress),
            RejectMessageReason::from_raw(reason),
            now,
            &TIMING,
        );
    }
    table.apply_reject(
        3000,
        port(0),
        RejectMessageReason::NOT_DIRECTLY_CONNECTED,
        now,
        &TIMING,
    );
    table.apply_reject(
        4000,
        port(0),
        RejectMessageReason::ROUTER_BUSY,
        now,
        &TIMING,
    );
    assert_eq!(
        table.lookup(3000).unwrap().reachability,
        Reachability::Unreachable
    );
    assert_eq!(
        table.lookup(4000).unwrap().reachability,
        Reachability::Unreachable
    );
    assert_eq!(
        table.claim_counters(),
        ClaimCounters {
            reject_applied: 4,
            reject_dampened: 2,
            reject_dampened_same_state: 2,
            ..Default::default()
        }
    );
    table.add_learned(3000, port(0), Mac::EMPTY, now);
    assert_eq!(held_networks(&table), 1);
    assert_eq!(holds_for(&table, 4000), 2);
}

#[test]
fn direct_and_absent_spam_never_mutates_routes_or_allocates_records() {
    let mut table = learned_table();
    let now = BASE;
    table.apply_reject(
        3000,
        port(0),
        RejectMessageReason::ROUTER_BUSY,
        now,
        &TIMING,
    );
    table.add_direct(3000, port(1)); // Replacing a learned route clears its records.
    assert_eq!(table.holds().count(), 0);
    for reason in 0..=u8::MAX {
        for network in [3000, 4000] {
            assert_eq!(
                table.apply_reject(
                    network,
                    port(0),
                    RejectMessageReason::from_raw(reason),
                    now,
                    &TIMING
                ),
                RejectEffect::SameState
            );
        }
    }
    let entry = table.lookup(3000).unwrap();
    assert!(entry.direct);
    assert_eq!(entry.port, port(1));
    assert_eq!(entry.reachability, Reachability::Reachable);
    assert!(entry.busy_until.is_none());
    assert!(entry.last_seen.is_none());
    assert!(entry.next_hop.is_empty());
    assert!(table.lookup(4000).is_none());
    assert_eq!(table.holds().count(), 0);
    assert_eq!(
        table.claim_counters(),
        ClaimCounters {
            reject_applied: 1,
            reject_dampened: 512,
            reject_dampened_same_state: 512,
            ..Default::default()
        }
    );
}

#[test]
fn nonlearning_maintenance_does_not_rearm_rejects() {
    let mut table = learned_table();
    let now = BASE;
    table.apply_reject(
        3000,
        port(0),
        RejectMessageReason::NOT_DIRECTLY_CONNECTED,
        now,
        &TIMING,
    );
    table.touch(3000, now);
    table.mark_available(3000);
    table.clear_expired_busy(now);
    table.apply_reject(
        3000,
        port(0),
        RejectMessageReason::ROUTER_BUSY,
        now,
        &TIMING,
    );
    assert_eq!(
        table.lookup(3000).unwrap().reachability,
        Reachability::Reachable
    );
    assert_eq!(table.claim_counters().reject_dampened_hold_down, 1);
    assert_eq!(table.held_since(3000, port(0)), Some(now));
}

// --- The bounded record pool (new in the core) ---

#[test]
fn a_full_hold_pool_evicts_its_oldest_record_and_counts_it() {
    let mut table: Table<8, 2> = Table::new();
    for network in [10, 20, 30] {
        table.add_learned(network, port(0), mac(&[1]), BASE);
    }
    table.apply_reject(
        10,
        port(0),
        RejectMessageReason::ROUTER_BUSY,
        BASE + secs(1),
        &TIMING,
    );
    table.apply_reject(
        20,
        port(0),
        RejectMessageReason::ROUTER_BUSY,
        BASE + secs(2),
        &TIMING,
    );
    assert_eq!(table.claim_counters().hold_evicted, 0);
    // A third record takes the oldest's place.
    table.apply_reject(
        30,
        port(0),
        RejectMessageReason::ROUTER_BUSY,
        BASE + secs(3),
        &TIMING,
    );
    assert_eq!(table.claim_counters().hold_evicted, 1);
    let mut holds: Vec<Hold> = table.holds().collect();
    holds.sort_by_key(|hold| hold.network);
    assert_eq!(
        holds,
        [
            Hold {
                network: 20,
                port: port(0),
                since: BASE + secs(2),
            },
            Hold {
                network: 30,
                port: port(0),
                since: BASE + secs(3),
            },
        ]
    );
    // Network 10 lost its hold-down, so a change from port 0 applies at
    // once, and its new record evicts the next oldest, network 20's.
    assert_eq!(
        table.apply_reject(
            10,
            port(0),
            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
            BASE + secs(4),
            &TIMING
        ),
        RejectEffect::Applied
    );
    assert_eq!(table.claim_counters().hold_evicted, 2);
    assert_eq!(table.held_since(20, port(0)), None);
    assert_eq!(table.held_since(10, port(0)), Some(BASE + secs(4)));
    // Updating a record already kept evicts nothing.
    table.mark_available(30);
    assert_eq!(
        table.apply_reject(
            30,
            port(0),
            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
            BASE + secs(60),
            &TIMING
        ),
        RejectEffect::Applied
    );
    assert_eq!(table.held_since(30, port(0)), Some(BASE + secs(60)));
    assert_eq!(table.claim_counters().hold_evicted, 2);

    // With no pool at all, every applied change is an eviction.
    let mut bare: Table<4, 0> = Table::new();
    bare.add_learned(10, PortId(0), Mac::EMPTY, BASE);
    assert_eq!(
        bare.apply_reject(10, port(0), RejectMessageReason::ROUTER_BUSY, BASE, &TIMING),
        RejectEffect::Applied
    );
    assert_eq!(bare.holds().count(), 0);
    assert_eq!(bare.claim_counters().hold_evicted, 1);
}

/// Dropping a network's records keeps the others in the order they were
/// made, which decides the eviction between two records made at the same
/// instant: the earlier one goes. A compaction that refilled the gap from
/// the end would evict the later one instead.
#[test]
fn forgetting_keeps_the_pools_order_for_eviction() {
    let mut table: Table<8, 3> = Table::new();
    for network in [10, 20, 30, 40, 50] {
        table.add_learned(network, port(0), mac(&[1]), BASE);
    }
    let busy = |table: &mut Table<8, 3>, network, at| {
        table.apply_reject(
            network,
            port(0),
            RejectMessageReason::ROUTER_BUSY,
            at,
            &TIMING,
        )
    };
    busy(&mut table, 10, BASE + secs(1));
    // Two records made at the same instant, 20's first.
    busy(&mut table, 20, BASE + secs(2));
    busy(&mut table, 30, BASE + secs(2));
    // Removing 10 drops its record; 20's and 30's keep their order.
    table.remove(10);
    let order: Vec<u16> = table.holds().map(|hold| hold.network).collect();
    assert_eq!(order, [20, 30]);
    busy(&mut table, 40, BASE + secs(3));
    assert_eq!(table.claim_counters().hold_evicted, 0);
    // The pool is full: the next record evicts the oldest, and of the two
    // equally old, 20's, made first.
    busy(&mut table, 50, BASE + secs(4));
    assert_eq!(table.claim_counters().hold_evicted, 1);
    assert_eq!(table.held_since(20, port(0)), None);
    assert_eq!(table.held_since(30, port(0)), Some(BASE + secs(2)));
}

#[test]
fn the_hold_down_and_busy_period_follow_the_timing() {
    let mut table: Hosted = learned_table();
    let timing = Timing {
        hold_down: secs(5),
        busy_for: secs(7),
        ..Timing::DEFAULT
    };
    table.apply_reject(
        3000,
        port(0),
        RejectMessageReason::ROUTER_BUSY,
        BASE,
        &timing,
    );
    assert_eq!(table.lookup(3000).unwrap().busy_until, Some(BASE + secs(7)));
    assert_eq!(
        table.apply_reject(
            3000,
            port(0),
            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
            BASE + secs(5) - NANO,
            &timing
        ),
        RejectEffect::HeldDown
    );
    assert_eq!(
        table.apply_reject(
            3000,
            port(0),
            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
            BASE + secs(5),
            &timing
        ),
        RejectEffect::Applied
    );
    // The default timing a moment later would still have held it down.
    let mut table: Hosted = learned_table();
    table.apply_reject(
        3000,
        port(0),
        RejectMessageReason::ROUTER_BUSY,
        BASE,
        &TIMING,
    );
    assert_eq!(
        table.apply_reject(
            3000,
            port(0),
            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
            BASE + secs(5),
            &TIMING
        ),
        RejectEffect::HeldDown
    );
}
