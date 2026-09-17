use super::*;

#[test]
fn add_direct_and_lookup() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);

    let entry = table.lookup(1000).unwrap();
    assert!(entry.directly_connected);
    assert_eq!(entry.port_index, 0);
    assert!(entry.next_hop_mac.is_empty());
}

#[test]
fn add_learned_route() {
    let mut table = RouterTable::new();
    let next_hop = MacAddr::from_slice(&[192, 168, 1, 100, 0xBA, 0xC0]);
    table.add_learned(2000, 0, next_hop.clone());

    let entry = table.lookup(2000).unwrap();
    assert!(!entry.directly_connected);
    assert_eq!(entry.port_index, 0);
    assert_eq!(entry.next_hop_mac, next_hop);
}

#[test]
fn lookup_unknown_returns_none() {
    let table = RouterTable::new();
    assert!(table.lookup(9999).is_none());
}

#[test]
fn remove_route() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    assert_eq!(table.len(), 1);

    let removed = table.remove(1000);
    assert!(removed.is_some());
    assert!(table.is_empty());
}

#[test]
fn networks_on_port() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);
    table.add_learned(3000, 0, MacAddr::from_slice(&[1, 2, 3]));

    let port0 = table.networks_on_port(0);
    assert_eq!(port0.len(), 2);
    assert!(port0.contains(&1000));
    assert!(port0.contains(&3000));

    let port1 = table.networks_on_port(1);
    assert_eq!(port1.len(), 1);
    assert!(port1.contains(&2000));
}

#[test]
fn list_all_networks() {
    let mut table = RouterTable::new();
    table.add_direct(100, 0);
    table.add_direct(200, 1);
    table.add_direct(300, 2);

    let nets = table.networks();
    assert_eq!(nets.len(), 3);
}

#[test]
fn learned_route_does_not_override_direct() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);

    let entry = table.lookup(1000).unwrap();
    assert!(entry.directly_connected);
    assert_eq!(entry.port_index, 0);

    // add_learned should not overwrite a direct route
    table.add_learned(1000, 1, MacAddr::from_slice(&[10, 0, 1, 1]));

    let entry = table.lookup(1000).unwrap();
    assert!(entry.directly_connected);
    assert_eq!(entry.port_index, 0);
    assert!(entry.next_hop_mac.is_empty());
}

#[test]
fn add_learned_overwrites_existing_learned() {
    let mut table = RouterTable::new();
    table.add_learned(3000, 0, MacAddr::from_slice(&[10, 0, 1, 1]));

    let entry = table.lookup(3000).unwrap();
    assert!(!entry.directly_connected);
    assert_eq!(entry.next_hop_mac.as_slice(), &[10, 0, 1, 1]);

    table.add_learned(3000, 1, MacAddr::from_slice(&[10, 0, 2, 1]));

    let entry = table.lookup(3000).unwrap();
    assert!(!entry.directly_connected);
    assert_eq!(entry.port_index, 1);
    assert_eq!(entry.next_hop_mac.as_slice(), &[10, 0, 2, 1]);
}

#[test]
fn lookup_unknown_network_returns_none() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);

    assert!(table.lookup(9999).is_none());
}

#[test]
fn purge_stale_routes() {
    let mut table = RouterTable::new();
    table.add_learned(3000, 0, MacAddr::from_slice(&[1, 2, 3]));
    let purged = table.purge_stale(Duration::from_secs(0));
    assert_eq!(purged, vec![3000]);
    assert!(table.lookup(3000).is_none());
}

#[test]
fn direct_routes_never_expire() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    let purged = table.purge_stale(Duration::from_secs(0));
    assert!(purged.is_empty());
    assert!(table.lookup(1000).is_some());
}

#[test]
fn touch_refreshes_timestamp() {
    let mut table = RouterTable::new();
    table.add_learned(3000, 0, MacAddr::from_slice(&[1, 2, 3]));
    table.touch(3000);
    let purged = table.purge_stale(Duration::from_secs(3600));
    assert!(purged.is_empty());
    assert!(table.lookup(3000).is_some());
}

#[test]
fn learned_route_has_last_seen() {
    let mut table = RouterTable::new();
    table.add_learned(3000, 0, MacAddr::from_slice(&[1, 2, 3]));
    let entry = table.lookup(3000).unwrap();
    assert!(entry.last_seen.is_some());
}

#[test]
fn direct_route_has_no_last_seen() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    let entry = table.lookup(1000).unwrap();
    assert!(entry.last_seen.is_none());
}

#[test]
fn networks_not_on_port_excludes_requesting_port() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);
    table.add_learned(3000, 1, MacAddr::from_slice(&[10, 0, 1, 1]));
    table.add_learned(4000, 0, MacAddr::from_slice(&[10, 0, 2, 1]));

    let nets = table.networks_not_on_port(0);
    assert!(nets.contains(&2000));
    assert!(nets.contains(&3000));
    assert!(!nets.contains(&1000));
    assert!(!nets.contains(&4000));
    assert_eq!(nets.len(), 2);

    let nets = table.networks_not_on_port(1);
    assert!(nets.contains(&1000));
    assert!(nets.contains(&4000));
    assert!(!nets.contains(&2000));
    assert!(!nets.contains(&3000));
    assert_eq!(nets.len(), 2);
}

#[test]
fn add_learned_flap_inserts_new_route() {
    let mut table = RouterTable::new();
    let result =
        table.add_learned_with_flap_detection(3000, 0, MacAddr::from_slice(&[10, 0, 1, 1]));
    assert!(result);
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.port_index, 0);
}

#[test]
fn add_learned_flap_refreshes_same_port() {
    let mut table = RouterTable::new();
    table.add_learned(3000, 0, MacAddr::from_slice(&[10, 0, 1, 1]));
    let result =
        table.add_learned_with_flap_detection(3000, 0, MacAddr::from_slice(&[10, 0, 1, 2]));
    assert!(result);
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.next_hop_mac.as_slice(), &[10, 0, 1, 2]);
}

#[test]
fn add_learned_flap_always_updates_different_port() {
    let mut table = RouterTable::new();
    table.add_learned(3000, 0, MacAddr::from_slice(&[10, 0, 1, 1]));
    // Spec 6.6.3.2: last I-Am-Router wins — always accept even from different port
    let result =
        table.add_learned_with_flap_detection(3000, 1, MacAddr::from_slice(&[10, 0, 2, 1]));
    assert!(result);
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.port_index, 1);
    assert_eq!(entry.next_hop_mac.as_slice(), &[10, 0, 2, 1]);
}

#[test]
fn add_learned_flap_increments_flap_count() {
    let mut table = RouterTable::new();
    table.add_learned_with_flap_detection(3000, 0, MacAddr::from_slice(&[10, 0, 1, 1]));
    table.add_learned_with_flap_detection(3000, 1, MacAddr::from_slice(&[10, 0, 2, 1]));
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.flap_count, 1);
    table.add_learned_with_flap_detection(3000, 0, MacAddr::from_slice(&[10, 0, 1, 1]));
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.flap_count, 2);
}

#[test]
fn add_learned_flap_rejects_direct_route() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    let result =
        table.add_learned_with_flap_detection(1000, 1, MacAddr::from_slice(&[10, 0, 2, 1]));
    assert!(!result);
    assert!(table.lookup(1000).unwrap().directly_connected);
}

#[test]
fn mark_busy_sets_reachability_and_deadline() {
    let mut table = RouterTable::new();
    table.add_learned(3000, 0, MacAddr::from_slice(&[1, 2, 3]));
    let deadline = Instant::now() + Duration::from_secs(30);
    table.mark_busy(3000, deadline);
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.reachability, ReachabilityStatus::Busy);
    assert_eq!(entry.busy_until, Some(deadline));
}

#[test]
fn mark_available_clears_busy() {
    let mut table = RouterTable::new();
    table.add_learned(3000, 0, MacAddr::from_slice(&[1, 2, 3]));
    table.mark_busy(3000, Instant::now() + Duration::from_secs(30));
    table.mark_available(3000);
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.reachability, ReachabilityStatus::Reachable);
    assert!(entry.busy_until.is_none());
}

#[test]
fn mark_unreachable_keeps_entry() {
    let mut table = RouterTable::new();
    table.add_learned(3000, 0, MacAddr::from_slice(&[1, 2, 3]));
    table.mark_unreachable(3000);
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.reachability, ReachabilityStatus::Unreachable);
    assert!(table.lookup(3000).is_some());
}

#[test]
fn mark_unreachable_does_not_affect_direct_routes() {
    let mut table = RouterTable::new();
    table.add_direct(1000, 0);
    table.mark_unreachable(1000);
    let entry = table.lookup(1000).unwrap();
    assert_eq!(entry.reachability, ReachabilityStatus::Reachable);
}

#[test]
fn clear_expired_busy_clears_elapsed_deadlines() {
    let mut table = RouterTable::new();
    table.add_learned(3000, 0, MacAddr::from_slice(&[1, 2, 3]));
    table.mark_busy(3000, Instant::now() - Duration::from_secs(1));
    table.clear_expired_busy();
    let entry = table.lookup(3000).unwrap();
    assert_eq!(entry.reachability, ReachabilityStatus::Reachable);
    assert!(entry.busy_until.is_none());
}

#[test]
fn effective_reachability_checks_deadline_inline() {
    let mut table = RouterTable::new();
    table.add_learned(3000, 0, MacAddr::from_slice(&[1, 2, 3]));
    table.mark_busy(3000, Instant::now() - Duration::from_secs(1));
    assert_eq!(
        table.effective_reachability(3000),
        Some(ReachabilityStatus::Reachable)
    );
}

#[test]
fn effective_reachability_returns_busy_when_deadline_not_elapsed() {
    let mut table = RouterTable::new();
    table.add_learned(3000, 0, MacAddr::from_slice(&[1, 2, 3]));
    table.mark_busy(3000, Instant::now() + Duration::from_secs(30));
    assert_eq!(
        table.effective_reachability(3000),
        Some(ReachabilityStatus::Busy)
    );
}
