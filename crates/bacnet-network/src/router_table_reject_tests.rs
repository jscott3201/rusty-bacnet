use super::*;

fn learned_table() -> RouterTable {
    let mut table = RouterTable::new();
    table.add_learned(3000, 0, MacAddr::from_slice(&[1]));
    table
}

#[test]
fn reject_spam_is_idempotent_without_refreshing_route_or_window() {
    let mut table = learned_table();
    let seen = table.lookup(3000).unwrap().last_seen;
    let now = Instant::now();
    for second in 0..100 {
        table.apply_reject(3000, 0, 1, now + Duration::from_secs(second));
    }
    assert_eq!(table.lookup(3000).unwrap().last_seen, seen);
    assert_eq!(
        table.lookup(3000).unwrap().reachability,
        ReachabilityStatus::Unreachable
    );
    assert_eq!(table.reject_transitions[&3000][&0], now);
    assert_eq!(
        table.claim_snapshot(),
        RoutingClaimSnapshot {
            reject_applied: 1,
            reject_dampened: 99,
            reject_dampened_same_state: 99,
            ..Default::default()
        }
    );
}

#[test]
fn alternating_rejects_hold_until_exact_boundary_without_sliding() {
    for first in [1, 2] {
        let mut table = learned_table();
        let now = Instant::now();
        let status = if first == 1 {
            ReachabilityStatus::Unreachable
        } else {
            ReachabilityStatus::Busy
        };
        for second in 0..10 {
            let reason = if second % 2 == 0 { first } else { 3 - first };
            table.apply_reject(3000, 0, reason, now + Duration::from_secs(second));
            assert_eq!(table.lookup(3000).unwrap().reachability, status);
        }
        table.apply_reject(
            3000,
            0,
            3 - first,
            now + HOLD_DOWN - Duration::from_nanos(1),
        );
        assert_eq!(table.lookup(3000).unwrap().reachability, status);
        assert_eq!(table.reject_transitions[&3000][&0], now);
        if first == 2 {
            assert_eq!(
                table.lookup(3000).unwrap().busy_until,
                Some(now + HOLD_DOWN)
            );
        }
        table.apply_reject(3000, 0, 3 - first, now + HOLD_DOWN);
        assert_ne!(table.lookup(3000).unwrap().reachability, status);
        assert_eq!(table.reject_transitions[&3000][&0], now + HOLD_DOWN);
        assert_eq!(
            table.claim_snapshot(),
            RoutingClaimSnapshot {
                reject_applied: 2,
                reject_dampened: 10,
                reject_dampened_same_state: 4,
                reject_dampened_hold_down: 6,
                ..Default::default()
            }
        );
    }
}

#[test]
fn busy_duplicates_do_not_renew_deadline_and_expired_busy_can_be_rejected_again() {
    let mut table = learned_table();
    let now = Instant::now();
    let deadline = now + Duration::from_secs(10);
    table.mark_busy(3000, deadline); // Non-reject busy marking, no reject record.
    table.apply_reject(3000, 0, 2, now);
    assert_eq!(table.lookup(3000).unwrap().busy_until, Some(deadline));
    assert!(table.reject_transitions.is_empty());
    table.apply_reject(3000, 0, 2, deadline); // Inline expiry, no aging sweep.
    assert_eq!(
        table.lookup(3000).unwrap().busy_until,
        Some(deadline + HOLD_DOWN)
    );
    table.apply_reject(3000, 0, 2, deadline + HOLD_DOWN - Duration::from_nanos(1));
    table.apply_reject(3000, 0, 2, deadline + HOLD_DOWN);
    assert_eq!(
        table.lookup(3000).unwrap().busy_until,
        Some(deadline + HOLD_DOWN * 2)
    );
    assert_eq!(
        table.claim_snapshot(),
        RoutingClaimSnapshot {
            reject_applied: 2,
            reject_dampened: 2,
            reject_dampened_same_state: 2,
            ..Default::default()
        }
    );
}

#[test]
fn every_legacy_removal_reason_applies_first_but_obeys_hold_down() {
    for reason in (0..=u8::MAX).filter(|reason| ![1, 2].contains(reason)) {
        let now = Instant::now();
        let mut table = learned_table();
        table.apply_reject(3000, 0, reason, now);
        assert!(table.lookup(3000).is_none());
        assert!(table.reject_transitions.is_empty());
        table.apply_reject(3000, 0, reason, now);
        assert_eq!(table.claim_snapshot().reject_applied, 1);
        assert_eq!(table.claim_snapshot().reject_dampened_same_state, 1);

        table.add_learned(3000, 0, MacAddr::new());
        table.apply_reject(3000, 0, 2, now);
        table.apply_reject(3000, 0, reason, now + Duration::from_secs(1));
        assert!(table.lookup(3000).is_some());
        table.apply_reject(3000, 0, reason, now + HOLD_DOWN);
        assert!(table.lookup(3000).is_none());
        assert!(table.reject_transitions.is_empty());
        assert_eq!(table.claim_snapshot().reject_applied, 3);
        assert_eq!(table.claim_snapshot().reject_dampened_hold_down, 1);
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
        let now = Instant::now();
        table.apply_reject(3000, 0, 1, now);
        table.apply_reject(3000, 1, 2, now);
        assert_eq!(table.reject_transitions[&3000].len(), 2);
        match rearm {
            "same_peer" => {
                assert!(table.add_learned_with_flap_detection(3000, 0, MacAddr::from_slice(&[1])));
            }
            "new_mac" => {
                assert!(table.add_learned_with_flap_detection(3000, 0, MacAddr::from_slice(&[2])));
            }
            "new_port" => {
                assert!(table.add_learned_with_flap_detection(3000, 1, MacAddr::from_slice(&[1])));
            }
            "manual_learning" => table.add_learned(3000, 0, MacAddr::new()),
            "remove" => {
                assert!(table.remove(3000).is_some());
                assert!(table.reject_transitions.is_empty());
                table.add_learned(3000, 0, MacAddr::new());
            }
            "age" => {
                table.lookup_mut(3000).unwrap().last_seen = Some(now - Duration::from_secs(120));
                assert_eq!(table.purge_stale(Duration::from_secs(60)), vec![3000]);
                assert!(table.reject_transitions.is_empty());
                table.add_learned(3000, 0, MacAddr::new());
            }
            _ => unreachable!(),
        }
        assert!(table.reject_transitions.is_empty(), "{rearm}");
        table.apply_reject(3000, 0, 1, now + Duration::from_secs(1));
        table.apply_reject(3000, 1, 2, now + Duration::from_secs(1));
        assert_eq!(
            table.claim_snapshot(),
            RoutingClaimSnapshot {
                reject_applied: 4,
                ..Default::default()
            },
            "{rearm}"
        );
    }
}

#[test]
fn reject_keys_are_independent_by_ingress_port_and_network() {
    let mut table = learned_table();
    table.add_learned(4000, 0, MacAddr::from_slice(&[1]));
    let now = Instant::now();
    for (network, ingress, reason) in [(3000, 0, 1), (4000, 0, 2), (3000, 1, 2), (4000, 1, 1)] {
        table.apply_reject(network, ingress, reason, now);
    }
    table.apply_reject(3000, 0, 1, now);
    table.apply_reject(4000, 0, 2, now);
    assert_eq!(
        table.lookup(3000).unwrap().reachability,
        ReachabilityStatus::Busy
    );
    assert_eq!(
        table.lookup(4000).unwrap().reachability,
        ReachabilityStatus::Unreachable
    );
    assert_eq!(
        table.claim_snapshot(),
        RoutingClaimSnapshot {
            reject_applied: 4,
            reject_dampened: 2,
            reject_dampened_hold_down: 2,
            ..Default::default()
        }
    );
    table.add_learned(3000, 0, MacAddr::new());
    assert_eq!(table.reject_transitions.len(), 1);
    assert_eq!(table.reject_transitions[&4000].len(), 2);
}

#[test]
fn direct_and_absent_spam_never_mutates_routes_or_allocates_records() {
    let mut table = learned_table();
    let now = Instant::now();
    table.apply_reject(3000, 0, 2, now);
    table.add_direct(3000, 1); // Replacing a learned route clears its records.
    assert!(table.reject_transitions.is_empty());
    for reason in 0..=u8::MAX {
        for network in [3000, 4000] {
            table.apply_reject(network, 0, reason, now);
        }
    }
    let entry = table.lookup(3000).unwrap();
    assert!(entry.directly_connected);
    assert_eq!(entry.port_index, 1);
    assert_eq!(entry.reachability, ReachabilityStatus::Reachable);
    assert!(entry.busy_until.is_none());
    assert!(entry.last_seen.is_none());
    assert!(entry.next_hop_mac.is_empty());
    assert!(table.lookup(4000).is_none());
    assert!(table.reject_transitions.is_empty());
    assert_eq!(
        table.claim_snapshot(),
        RoutingClaimSnapshot {
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
    let now = Instant::now();
    table.apply_reject(3000, 0, 1, now);
    table.touch(3000);
    table.mark_available(3000);
    table.clear_expired_busy();
    table.apply_reject(3000, 0, 2, now);
    assert_eq!(
        table.lookup(3000).unwrap().reachability,
        ReachabilityStatus::Reachable
    );
    assert_eq!(table.claim_snapshot().reject_dampened_hold_down, 1);
    assert_eq!(table.reject_transitions[&3000][&0], now);
}

#[test]
fn counters_saturate_without_affecting_decisions_and_snapshots_are_owned() {
    let mut table = learned_table();
    table.claim_counters = RoutingClaimSnapshot {
        reject_applied: u64::MAX - 1,
        reject_dampened: u64::MAX - 1,
        reject_dampened_same_state: u64::MAX - 1,
        reject_dampened_hold_down: u64::MAX - 1,
        learned_ok: u64::MAX - 1,
        learned_cap_ignored: u64::MAX - 1,
        flap_warned: u64::MAX - 1,
        ..Default::default()
    };
    let before = table.claim_snapshot();
    let now = Instant::now();
    for _ in 0..2 {
        table.add_learned(3000, 0, MacAddr::new());
        table.apply_reject(3000, 0, 1, now);
        table.apply_reject(3000, 0, 1, now);
        table.apply_reject(3000, 0, 2, now);
        assert_eq!(
            table.lookup(3000).unwrap().reachability,
            ReachabilityStatus::Unreachable
        );
        table.record_learned();
        table.record_learning_cap();
        let entry = table.lookup_mut(3000).unwrap();
        entry.flap_count = 2;
        entry.last_port_change = Some(now);
        assert!(table.add_learned_with_flap_detection(3000, 1, MacAddr::new()));
    }
    let snapshot = table.claim_snapshot();
    assert_eq!(
        snapshot,
        RoutingClaimSnapshot {
            reject_applied: u64::MAX,
            reject_dampened: u64::MAX,
            reject_dampened_same_state: u64::MAX,
            reject_dampened_hold_down: u64::MAX,
            learned_ok: u64::MAX,
            learned_cap_ignored: u64::MAX,
            flap_warned: u64::MAX,
            ..Default::default()
        }
    );
    let cloned = table.clone();
    drop(table);
    assert_eq!(cloned.claim_snapshot(), snapshot);
    assert_eq!(before.reject_applied, u64::MAX - 1);
    assert_eq!(
        RouterTable::default().claim_snapshot(),
        RoutingClaimSnapshot::default()
    );
}
