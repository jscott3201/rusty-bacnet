use super::*;

fn learned_table() -> RouterTable {
    let mut table = RouterTable::new();
    table.add_learned(3000, 0, MacAddr::from_slice(&[1]));
    table
}

fn claim(table: &mut RouterTable, port: usize, now: Instant) -> bool {
    table.apply_learning_claim(3000, port, MacAddr::from_slice(&[2]), now)
}

#[test]
fn corroboration_inclusive_deadline_and_expiry_restart() {
    for elapsed in [
        CORROBORATION_WINDOW - Duration::from_nanos(1),
        CORROBORATION_WINDOW,
        CORROBORATION_WINDOW + Duration::from_nanos(1),
    ] {
        let mut table = learned_table();
        let now = Instant::now();
        let seen = table.lookup(3000).unwrap().last_seen;
        assert!(!claim(&mut table, 1, now));
        assert_eq!(table.lookup(3000).unwrap().port_index, 0);
        assert_eq!(table.lookup(3000).unwrap().last_seen, seen);
        assert_eq!(table.lookup(3000).unwrap().next_hop_mac.as_slice(), &[1]);
        assert_eq!(
            table.claim_snapshot(),
            RoutingClaimSnapshot {
                pending_started: 1,
                ..Default::default()
            }
        );

        let applies = elapsed <= CORROBORATION_WINDOW;
        assert_eq!(claim(&mut table, 1, now + elapsed), applies);
        assert_eq!(table.lookup(3000).unwrap().port_index, usize::from(applies));
        assert_eq!(table.pending_replacements.is_empty(), applies);
        assert_eq!(
            table.claim_snapshot(),
            RoutingClaimSnapshot {
                pending_started: if applies { 1 } else { 2 },
                corroborated_applied: u64::from(applies),
                pending_expired: u64::from(!applies),
                ..Default::default()
            }
        );
        if !applies {
            assert_eq!(table.pending_replacements[&3000].first_seen, now + elapsed);
            assert_eq!(table.lookup(3000).unwrap().last_seen, seen);
            assert!(claim(&mut table, 1, now + elapsed + CORROBORATION_WINDOW));
            assert!(table.pending_replacements.is_empty());
            assert_eq!(table.claim_snapshot().corroborated_applied, 1);
            assert_eq!(table.claim_snapshot().pending_expired, 1);
        }
    }
}

#[test]
fn spaced_single_claims_never_accumulate_or_refresh_old_route() {
    let mut table = learned_table();
    let now = Instant::now();
    let seen = table.lookup(3000).unwrap().last_seen;
    for index in 0..10 {
        assert!(!claim(
            &mut table,
            1,
            now + (CORROBORATION_WINDOW + Duration::from_nanos(1)) * index
        ));
        assert_eq!(table.lookup(3000).unwrap().port_index, 0);
        assert_eq!(table.lookup(3000).unwrap().last_seen, seen);
        assert_eq!(table.pending_replacements.len(), 1);
        assert_eq!(
            table.claim_snapshot(),
            RoutingClaimSnapshot {
                pending_started: u64::from(index + 1),
                pending_expired: u64::from(index),
                ..Default::default()
            }
        );
    }
}

#[test]
fn alternating_challengers_replace_slot_and_reset_first_seen() {
    let mut table = learned_table();
    let now = Instant::now();
    for second in 0..100 {
        let port = 1 + second as usize % 2;
        let time = now + Duration::from_secs(second);
        assert!(!claim(&mut table, port, time));
        assert_eq!(table.lookup(3000).unwrap().port_index, 0);
        assert_eq!(table.pending_replacements.len(), 1);
        assert_eq!(table.pending_replacements[&3000].port_index, port);
        assert_eq!(table.pending_replacements[&3000].first_seen, time);
    }
    assert_eq!(
        table.claim_snapshot(),
        RoutingClaimSnapshot {
            pending_started: 100,
            ..Default::default()
        }
    );
    // Legitimate convergence resumes when a challenger can repeat uninterrupted.
    assert!(claim(&mut table, 2, now + Duration::from_secs(100)));
    assert_eq!(table.lookup(3000).unwrap().port_index, 2);
    assert_eq!(table.claim_snapshot().corroborated_applied, 1);
}

#[test]
fn absent_current_port_direct_and_reserved_claims_remain_immediate() {
    let mut table = RouterTable::new();
    let now = Instant::now();
    assert!(claim(&mut table, 0, now));
    assert!(table.pending_replacements.is_empty());
    assert!(!claim(&mut table, 1, now));
    table.mark_busy(3000, now + HOLD_DOWN);
    assert!(claim(&mut table, 0, now));
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.port_index, 0);
    assert_eq!(entry.reachability, ReachabilityStatus::Reachable);
    assert_eq!(entry.flap_count, 0);
    assert!(entry.last_port_change.is_none());
    assert!(entry.busy_until.is_none());
    assert!(table.pending_replacements.is_empty());
    assert!(!claim(&mut table, 1, now)); // Refresh discarded the earlier vote.
    table.add_direct(3000, 0);
    for net in [0, 0xffff, 3000] {
        for _ in 0..2 {
            assert!(!table.apply_learning_claim(net, 1, MacAddr::new(), now));
        }
    }
    assert_eq!(table.len(), 1);
    assert!(table.lookup(3000).unwrap().directly_connected);
    assert!(table.pending_replacements.is_empty());
    assert_eq!(
        table.claim_snapshot(),
        RoutingClaimSnapshot {
            pending_started: 2,
            ..Default::default()
        }
    );
}

#[test]
fn pending_claim_preserves_busy_and_reject_records_until_apply() {
    let mut table = learned_table();
    let now = Instant::now();
    table.apply_reject(3000, 0, 2, now);
    assert!(!claim(&mut table, 1, now));
    assert_eq!(
        table.lookup(3000).unwrap().busy_until,
        Some(now + HOLD_DOWN)
    );
    assert_eq!(table.reject_transitions[&3000][&0], now);
    table.apply_reject(3000, 0, 1, now);
    assert_eq!(table.claim_snapshot().reject_dampened_hold_down, 1);
    assert!(claim(&mut table, 1, now));
    assert!(table.reject_transitions.is_empty());
    assert_eq!(
        table.lookup(3000).unwrap().reachability,
        ReachabilityStatus::Reachable
    );
    assert_eq!(table.claim_snapshot().corroborated_applied, 1);
}

#[test]
fn cleanup_never_leaves_pending_without_a_learned_route() {
    for cleanup in [
        "remove",
        "reject_remove",
        "aging",
        "direct",
        "manual",
        "manual_flap",
        "mutable",
    ] {
        let mut table = learned_table();
        let now = Instant::now();
        // Set age before creating pending so this exercises purge, not mutable handoff.
        table.lookup_mut(3000).unwrap().last_seen = Some(now - Duration::from_secs(120));
        assert!(!claim(&mut table, 1, now));
        match cleanup {
            "remove" => {
                table.remove(3000);
            }
            "reject_remove" => table.apply_reject(3000, 0, 0, now),
            "aging" => assert_eq!(table.purge_stale(Duration::from_secs(60)), vec![3000]),
            "direct" => table.add_direct(3000, 1),
            "manual" => table.add_learned(3000, 2, MacAddr::new()),
            "manual_flap" => {
                assert!(table.add_learned_with_flap_detection(3000, 2, MacAddr::new()));
            }
            "mutable" => table.lookup_mut(3000).unwrap().directly_connected = true,
            _ => unreachable!(),
        }
        assert!(table.pending_replacements.is_empty(), "{cleanup}");
        assert_eq!(table.claim_snapshot().pending_expired, 0, "{cleanup}");
        table.add_learned(4000, 0, MacAddr::new());
        assert!(table.apply_learning_claim(4000, 0, MacAddr::new(), now));
        assert!(!table.apply_learning_claim(4000, 1, MacAddr::new(), now));
        assert_eq!(table.pending_replacements.len(), 1);
    }
}

#[test]
fn pending_store_is_bounded_and_networks_are_independent() {
    let mut table = RouterTable::new();
    let now = Instant::now();
    for net in 1..=256 {
        table.add_learned(net, 0, MacAddr::new());
        for port in 1..=10 {
            assert!(!table.apply_learning_claim(net, port, MacAddr::new(), now));
            assert_eq!(table.pending_replacements.len(), usize::from(net));
            assert!(
                table.pending_replacements.len()
                    <= table
                        .routes
                        .values()
                        .filter(|e| !e.directly_connected)
                        .count()
            );
        }
    }
    for net in 1..=256 {
        assert!(table.apply_learning_claim(net, 10, MacAddr::new(), now));
        assert_eq!(table.pending_replacements.len(), 256 - usize::from(net));
    }
    assert_eq!(
        table.claim_snapshot(),
        RoutingClaimSnapshot {
            pending_started: 2560,
            corroborated_applied: 256,
            ..Default::default()
        }
    );
}

#[test]
fn flap_warnings_only_count_applied_moves_and_same_port_refresh_resets_history() {
    let mut table = learned_table();
    let now = Instant::now();
    for port in 1..=3 {
        assert!(!claim(&mut table, port, now));
        assert_eq!(table.claim_snapshot().flap_warned, 0);
        assert!(claim(&mut table, port, now));
        assert_eq!(table.lookup(3000).unwrap().flap_count, port as u8);
    }
    assert_eq!(table.claim_snapshot().flap_warned, 1);
    assert!(claim(&mut table, 3, now));
    assert_eq!(table.lookup(3000).unwrap().flap_count, 0);
    assert!(!claim(&mut table, 4, now));
    assert!(claim(&mut table, 4, now));
    assert_eq!(table.lookup(3000).unwrap().flap_count, 1);
    assert_eq!(table.claim_snapshot().flap_warned, 1);
}

#[test]
fn new_counters_saturate_without_affecting_gate_and_clones_are_independent() {
    let mut table = learned_table();
    table.claim_counters = RoutingClaimSnapshot {
        pending_started: u64::MAX - 1,
        corroborated_applied: u64::MAX - 1,
        pending_expired: u64::MAX - 1,
        disconnect_removal_ignored: u64::MAX - 1,
        ..Default::default()
    };
    let now = Instant::now();
    let before = table.claim_snapshot();
    for port in 1..=2 {
        let start = now + Duration::from_secs(port as u64 * 120);
        assert!(!claim(&mut table, port, start));
        let mut cloned = table.clone();
        assert!(claim(&mut cloned, port, start));
        assert!(!table.pending_replacements.is_empty());
        assert!(!claim(&mut table, port, start + Duration::from_secs(61)));
        assert!(claim(&mut table, port, start + Duration::from_secs(62)));
        table.record_disconnect_removal_ignored();
    }
    let snapshot = table.claim_snapshot();
    drop(table);
    assert_eq!(before.pending_started, u64::MAX - 1);
    assert_eq!(
        snapshot,
        RoutingClaimSnapshot {
            pending_started: u64::MAX,
            corroborated_applied: u64::MAX,
            pending_expired: u64::MAX,
            disconnect_removal_ignored: u64::MAX,
            ..Default::default()
        }
    );
}
