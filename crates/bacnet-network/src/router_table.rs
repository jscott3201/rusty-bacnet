//! Router table — maps BACnet network numbers to transport ports.
//!
//! Per ASHRAE 135-2020 Clause 6.4, a BACnet router maintains a routing table
//! that records which directly-connected or learned networks can be reached
//! via which port.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use bacnet_types::MacAddr;

/// Reachability status of a route entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReachabilityStatus {
    /// Route is available for traffic.
    Reachable,
    /// Route is temporarily unreachable due to congestion (Router-Busy).
    Busy,
    /// Route has permanently failed.
    Unreachable,
}

/// A route entry in the router table.
#[derive(Debug, Clone)]
pub struct RouteEntry {
    /// Index of the port this network is reachable through.
    pub port_index: usize,
    /// Whether this is a directly-connected network (vs learned via another router).
    pub directly_connected: bool,
    /// MAC address of the next-hop router (empty for directly-connected networks).
    pub next_hop_mac: MacAddr,
    /// When this learned route was last confirmed. `None` for direct routes.
    pub last_seen: Option<Instant>,
    pub reachability: ReachabilityStatus,
    /// Deadline after which a `Busy` status auto-clears (spec 6.6.3.6).
    pub busy_until: Option<Instant>,
    /// Number of times this route changed ports within the flap detection window.
    pub flap_count: u8,
    /// When the route last changed ports.
    pub last_port_change: Option<Instant>,
}

/// BACnet routing table.
///
/// Maps network numbers to the port through which they can be reached.
#[derive(Debug, Clone)]
pub struct RouterTable {
    /// Network number → route entry.
    routes: HashMap<u16, RouteEntry>,
}

impl RouterTable {
    /// Create an empty routing table.
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
        }
    }

    /// Add a directly-connected network on the given port.
    /// Network 0 and 0xFFFF are reserved and will be silently ignored.
    pub fn add_direct(&mut self, network: u16, port_index: usize) {
        if network == 0 || network == 0xFFFF {
            return;
        }
        self.routes.insert(
            network,
            RouteEntry {
                port_index,
                directly_connected: true,
                next_hop_mac: MacAddr::new(),
                last_seen: None,
                reachability: ReachabilityStatus::Reachable,
                busy_until: None,
                flap_count: 0,
                last_port_change: None,
            },
        );
    }

    /// Add a learned route (network reachable via a next-hop router on the given port).
    /// Network 0 and 0xFFFF are reserved and will be silently ignored.
    /// Does not overwrite direct routes.
    pub fn add_learned(&mut self, network: u16, port_index: usize, next_hop_mac: MacAddr) {
        if network == 0 || network == 0xFFFF {
            return;
        }
        if let Some(existing) = self.routes.get(&network) {
            if existing.directly_connected {
                return; // never overwrite direct routes
            }
        }
        self.routes.insert(
            network,
            RouteEntry {
                port_index,
                directly_connected: false,
                next_hop_mac,
                last_seen: Some(Instant::now()),
                reachability: ReachabilityStatus::Reachable,
                busy_until: None,
                flap_count: 0,
                last_port_change: None,
            },
        );
    }

    /// Add a learned route, always accepting (spec 6.6.3.2: last I-Am-Router wins).
    /// Detects rapid port changes for operator visibility but never suppresses updates.
    ///
    /// Returns `true` if the route was inserted/updated.
    pub fn add_learned_with_flap_detection(
        &mut self,
        network: u16,
        port_index: usize,
        next_hop_mac: MacAddr,
    ) -> bool {
        if network == 0 || network == 0xFFFF {
            return false;
        }
        if let Some(existing) = self.routes.get(&network) {
            if existing.directly_connected {
                return false;
            }
            if existing.port_index != port_index {
                let now = Instant::now();
                let flap_count = match existing.last_port_change {
                    Some(changed) if now.duration_since(changed) < Duration::from_secs(60) => {
                        existing.flap_count.saturating_add(1)
                    }
                    _ => 1,
                };
                if flap_count >= 3 {
                    tracing::warn!(
                        network,
                        old_port = existing.port_index,
                        new_port = port_index,
                        flap_count,
                        "Route flapping detected — network changed ports {} times in 60s",
                        flap_count
                    );
                }
                self.routes.insert(
                    network,
                    RouteEntry {
                        port_index,
                        directly_connected: false,
                        next_hop_mac,
                        last_seen: Some(now),
                        reachability: ReachabilityStatus::Reachable,
                        busy_until: None,
                        flap_count,
                        last_port_change: Some(now),
                    },
                );
                return true;
            }
        }
        self.add_learned(network, port_index, next_hop_mac);
        true
    }

    /// Mark a network as busy with a deadline for auto-clear (spec 6.6.3.6).
    pub fn mark_busy(&mut self, network: u16, deadline: Instant) {
        if let Some(entry) = self.routes.get_mut(&network) {
            entry.reachability = ReachabilityStatus::Busy;
            entry.busy_until = Some(deadline);
        }
    }

    /// Mark a network as available, clearing any busy state (spec 6.6.3.7).
    pub fn mark_available(&mut self, network: u16) {
        if let Some(entry) = self.routes.get_mut(&network) {
            entry.reachability = ReachabilityStatus::Reachable;
            entry.busy_until = None;
        }
    }

    /// Mark a network as permanently unreachable (spec 6.6.3.5, reject reason 1).
    /// Keeps the entry in the table (unlike `remove`).
    pub fn mark_unreachable(&mut self, network: u16) {
        if let Some(entry) = self.routes.get_mut(&network) {
            if !entry.directly_connected {
                entry.reachability = ReachabilityStatus::Unreachable;
                entry.busy_until = None;
            }
        }
    }

    /// Clear busy state for entries whose `busy_until` deadline has elapsed.
    pub fn clear_expired_busy(&mut self) {
        let now = Instant::now();
        for entry in self.routes.values_mut() {
            if let Some(deadline) = entry.busy_until {
                if now >= deadline {
                    entry.reachability = ReachabilityStatus::Reachable;
                    entry.busy_until = None;
                }
            }
        }
    }

    /// Get effective reachability, checking busy_until inline for immediate accuracy.
    /// This avoids up to 90s worst-case from the 60s aging sweep granularity.
    pub fn effective_reachability(&self, network: u16) -> Option<ReachabilityStatus> {
        self.routes.get(&network).map(|entry| {
            if entry.reachability == ReachabilityStatus::Busy {
                if let Some(deadline) = entry.busy_until {
                    if Instant::now() >= deadline {
                        return ReachabilityStatus::Reachable;
                    }
                }
            }
            entry.reachability
        })
    }

    /// Look up the route for a network number.
    pub fn lookup(&self, network: u16) -> Option<&RouteEntry> {
        self.routes.get(&network)
    }

    /// Lookup a mutable route entry by network number.
    pub fn lookup_mut(&mut self, network: u16) -> Option<&mut RouteEntry> {
        self.routes.get_mut(&network)
    }

    /// Remove a route.
    pub fn remove(&mut self, network: u16) -> Option<RouteEntry> {
        self.routes.remove(&network)
    }

    /// List all known network numbers.
    pub fn networks(&self) -> Vec<u16> {
        self.routes.keys().copied().collect()
    }

    /// List networks reachable via ports OTHER than `exclude_port`.
    pub fn networks_not_on_port(&self, exclude_port: usize) -> Vec<u16> {
        self.routes
            .iter()
            .filter(|(_, entry)| entry.port_index != exclude_port)
            .map(|(net, _)| *net)
            .collect()
    }

    /// List networks reachable on a given port.
    pub fn networks_on_port(&self, port_index: usize) -> Vec<u16> {
        self.routes
            .iter()
            .filter(|(_, entry)| entry.port_index == port_index)
            .map(|(net, _)| *net)
            .collect()
    }

    /// Number of routes.
    pub fn len(&self) -> usize {
        self.routes.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// Refresh the `last_seen` timestamp for a learned route.
    ///
    /// Direct routes are unaffected since they never expire.
    pub fn touch(&mut self, network: u16) {
        if let Some(entry) = self.routes.get_mut(&network) {
            if !entry.directly_connected {
                entry.last_seen = Some(Instant::now());
            }
        }
    }

    /// Remove learned routes that have not been refreshed within `max_age`.
    ///
    /// Returns the network numbers that were purged.
    pub fn purge_stale(&mut self, max_age: Duration) -> Vec<u16> {
        let now = Instant::now();
        let stale: Vec<u16> = self
            .routes
            .iter()
            .filter(|(_, entry)| {
                if let Some(seen) = entry.last_seen {
                    !entry.directly_connected && now.duration_since(seen) > max_age
                } else {
                    false
                }
            })
            .map(|(net, _)| *net)
            .collect();
        for net in &stale {
            self.routes.remove(net);
        }
        stale
    }
}

impl Default for RouterTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
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
}
