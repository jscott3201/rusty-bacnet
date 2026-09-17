//! Router table — maps BACnet network numbers to transport ports.
//!
//! Per ASHRAE 135-2020 Clause 6.4, a BACnet router maintains a routing table
//! that records which directly-connected or learned networks can be reached
//! via which port.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use bacnet_types::MacAddr;

// Align with the existing 30s reject-busy deadline: at most one accepted
// reject-driven change per (ingress port, network) per window without fresh
// learning. A legitimate unreachable-after-busy signal may wait up to 30s.
const HOLD_DOWN: Duration = Duration::from_secs(30);

// Repeat-based local hardening, aligned with the existing flap window.
const CORROBORATION_WINDOW: Duration = Duration::from_secs(60);

/// Count-only outcomes for routing claims handled by one router table.
///
/// All totals saturate at `u64::MAX`, never affect routing decisions, and expose
/// no peer identities. Learning totals cover inspected I-Am-Router and
/// Initialize-Routing-Table-Ack entries, not manual table edits, management
/// Initialize-Routing-Table writes, or other message types. Existing cap-triggered early stops remain: their unexamined suffixes
/// are not classified or counted. Flap warnings are counted at their emission.
/// Pending replacements are not successful learning until corroborated. Disconnect
/// totals count each well-formed ignored removal request, even for absent/direct
/// routes; malformed messages are not counted.
/// Duplicate cross-port entries within one message supply no extra vote or count.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RoutingClaimSnapshot {
    /// Rejects that changed a learned route's effective state or removed it.
    pub reject_applied: u64,
    /// Rejects ignored as no-ops or during hold-down (counted once each).
    pub reject_dampened: u64,
    /// No-op rejects, including missing routes and directly-connected immunity.
    /// Busy duplicates do not extend the existing busy deadline.
    pub reject_dampened_same_state: u64,
    /// State-changing rejects ignored within 30s of this key's last applied one.
    pub reject_dampened_hold_down: u64,
    /// Inspected learning claims that inserted or refreshed a learned route.
    pub learned_ok: u64,
    /// Inspected entries that triggered the existing route-cap early stop.
    pub learned_cap_ignored: u64,
    /// Flap warnings emitted at the existing port-change threshold.
    pub flap_warned: u64,
    /// Cross-port claims held pending, including new challengers and expiry restarts.
    pub pending_started: u64,
    /// Replacements applied on a second same-network/port claim within 60s inclusive.
    pub corroborated_applied: u64,
    /// Pending slots found older than 60s and discarded on the next learning claim.
    /// Non-expiry cleanup (removal, aging, manual edits or refresh) does not count.
    pub pending_expired: u64,
    /// Disconnect-Connection-To-Network removal requests ignored (PTP unsupported).
    pub disconnect_removal_ignored: u64,
}

#[derive(Debug, Clone)]
struct PendingReplacement {
    port_index: usize,
    first_seen: Instant,
}

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
    /// Recorded reachability state of this route.
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
    // Network → (ingress port → last APPLIED reject). Grouping by network makes
    // re-arming on learning/removal local. Only live learned routes retain
    // records, at most one per router ingress port; absent/direct spam allocates
    // nothing. Neither peer MACs nor advertised source addresses are keys.
    reject_transitions: HashMap<u16, HashMap<usize, Instant>>,
    // One challenger per live learned network, never keyed by peer identity.
    // Thus pending entries <= live learned routes. Every route replacement,
    // removal and mutable-entry handoff clears its slot; expiry is checked lazily.
    pending_replacements: HashMap<u16, PendingReplacement>,
    claim_counters: RoutingClaimSnapshot,
}

impl RouterTable {
    /// Create an empty routing table.
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
            reject_transitions: HashMap::new(),
            pending_replacements: HashMap::new(),
            claim_counters: RoutingClaimSnapshot::default(),
        }
    }

    /// Read a consistent, non-resetting copy of this table's routing-claim totals.
    ///
    /// For a live router, call `router.table().lock().await.claim_snapshot()`.
    /// The snapshot owns only counts and remains readable after table drop;
    /// cloning the table copies, rather than shares, its counters and records.
    pub fn claim_snapshot(&self) -> RoutingClaimSnapshot {
        self.claim_counters
    }

    pub(crate) fn record_learned(&mut self) {
        self.claim_counters.learned_ok = self.claim_counters.learned_ok.saturating_add(1);
    }

    pub(crate) fn record_learning_cap(&mut self) {
        self.claim_counters.learned_cap_ignored =
            self.claim_counters.learned_cap_ignored.saturating_add(1);
    }

    pub(crate) fn record_disconnect_removal_ignored(&mut self) {
        self.claim_counters.disconnect_removal_ignored = self
            .claim_counters
            .disconnect_removal_ignored
            .saturating_add(1);
    }

    /// Gate I-Am-Router and Init-Routing-Table-ACK cross-port replacements only.
    /// The caller holds the table lock across this synchronous gate and update.
    /// Absent learning and current-port refreshes use the existing immediate path.
    /// `now` makes the inclusive 60s boundary testable without sleeping.
    pub(crate) fn apply_learning_claim(
        &mut self,
        network: u16,
        port_index: usize,
        next_hop_mac: MacAddr,
        now: Instant,
    ) -> bool {
        let pending = self
            .pending_replacements
            .remove(&network)
            .filter(|pending| {
                if now.duration_since(pending.first_seen) > CORROBORATION_WINDOW {
                    self.claim_counters.pending_expired =
                        self.claim_counters.pending_expired.saturating_add(1);
                    false
                } else {
                    true
                }
            });
        if self
            .routes
            .get(&network)
            .is_some_and(|entry| !entry.directly_connected && entry.port_index != port_index)
        {
            if pending.is_some_and(|pending| pending.port_index == port_index) {
                // K=2: the second claim supplies the next hop, even if its MAC
                // differs. Neither source identity nor message type is a key.
                self.claim_counters.corroborated_applied =
                    self.claim_counters.corroborated_applied.saturating_add(1);
            } else {
                self.pending_replacements.insert(
                    network,
                    PendingReplacement {
                        port_index,
                        first_seen: now,
                    },
                );
                self.claim_counters.pending_started =
                    self.claim_counters.pending_started.saturating_add(1);
                debug_assert!(self.pending_replacements.keys().all(|net| self
                    .routes
                    .get(net)
                    .is_some_and(|entry| !entry.directly_connected)));
                return false; // Keep all old route state, including forwarding and age.
            }
        }
        self.add_learned_with_flap_detection(network, port_index, next_hop_mac)
    }

    /// Apply only the reject-driven table transition, never its relay.
    /// The caller holds the table lock across this synchronous gate and update.
    /// `now` is supplied so deadline boundaries can be tested without sleeping.
    pub(crate) fn apply_reject(
        &mut self,
        network: u16,
        ingress_port: usize,
        reason: u8,
        now: Instant,
    ) {
        let changes_state = self.routes.get(&network).is_some_and(|entry| {
            if entry.directly_connected {
                return false;
            }
            // Compare effective state, not the stored Busy marker: a busy
            // deadline may have elapsed before the next aging sweep.
            match reason {
                1 => entry.reachability != ReachabilityStatus::Unreachable,
                2 => {
                    // A Busy (flow-control) claim never lifts permanent
                    // Unreachable: the route is already beyond reachable, so
                    // there is no change to apply, dampen, or record — only
                    // Available or fresh learning exits Unreachable. An
                    // unexpired Busy is likewise already in effect.
                    entry.reachability != ReachabilityStatus::Unreachable
                        && (entry.reachability != ReachabilityStatus::Busy
                            || entry.busy_until.is_some_and(|deadline| now >= deadline))
                }
                _ => true,
            }
        });
        let held_down = self
            .reject_transitions
            .get(&network)
            .and_then(|ports| ports.get(&ingress_port))
            .is_some_and(|last| now.duration_since(*last) < HOLD_DOWN);
        if !changes_state || held_down {
            let counters = &mut self.claim_counters;
            counters.reject_dampened = counters.reject_dampened.saturating_add(1);
            if !changes_state {
                counters.reject_dampened_same_state =
                    counters.reject_dampened_same_state.saturating_add(1);
            } else {
                counters.reject_dampened_hold_down =
                    counters.reject_dampened_hold_down.saturating_add(1);
            }
            return; // Ignored claims never slide the window or busy deadline.
        }

        match reason {
            1 => self.mark_unreachable(network),
            2 => self.mark_busy(network, now + Duration::from_secs(30)),
            _ => {
                self.remove(network); // Removal also discards every port's record.
            }
        }
        if self.routes.contains_key(&network) {
            self.reject_transitions
                .entry(network)
                .or_default()
                .insert(ingress_port, now);
        }
        self.claim_counters.reject_applied = self.claim_counters.reject_applied.saturating_add(1);
    }

    /// Add a directly-connected network on the given port.
    /// Network 0 and 0xFFFF are reserved and will be silently ignored.
    pub fn add_direct(&mut self, network: u16, port_index: usize) {
        if network == 0 || network == 0xFFFF {
            return;
        }
        self.reject_transitions.remove(&network);
        self.pending_replacements.remove(&network);
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
        self.reject_transitions.remove(&network);
        self.pending_replacements.remove(&network);
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

    /// Wire Port ID for a dispatch port index (135-2020 6.4.7, Fig. 6-11).
    ///
    /// "If the Port ID field has a value of zero, then all table entries for
    /// the specified DNET shall be purged from the table" — so 0 is a removal
    /// trigger, never a real port. The stable mapping is `port_index + 1`;
    /// `None` when the index cannot fit the 1-octet field (unreachable for a
    /// bounded port vector, handled by skipping the entry on encode).
    pub fn wire_port_id(port_index: usize) -> Option<u8> {
        u8::try_from(port_index.checked_add(1)?).ok()
    }

    /// Dispatch port index for a wire Port ID, given the local port count.
    ///
    /// Inverse of [`Self::wire_port_id`]: Port ID 0 (the purge trigger) and
    /// IDs past the local port count map to `None`; the caller applies purge
    /// versus unknown-port skip semantics.
    pub fn port_index_for_wire_id(port_id: u8, port_count: usize) -> Option<usize> {
        usize::from(port_id)
            .checked_sub(1)
            .filter(|index| *index < port_count)
    }

    /// Network numbers in ascending order for deterministic query replies.
    pub fn sorted_networks(&self) -> Vec<u16> {
        let mut networks = self.networks();
        networks.sort_unstable();
        networks
    }

    /// Apply one Initialize-Routing-Table update entry (135-2020 6.6.3.8).
    ///
    /// "It shall update its current port-to-network-number mappings for each
    /// network specified in the NPDU": "the routing information for this DNET
    /// shall either replace any previous entry ... or, if no such entry
    /// exists, be appended". Management writes install learned routes and
    /// never direct ones — direct attachments stay locally configured until
    /// RB-09 authorization — so an existing direct entry is left untouched
    /// and reports `false`. Reserved networks are skipped. Replacement clears
    /// pending/reject records exactly like fresh learning.
    pub(crate) fn apply_management_update(
        &mut self,
        network: u16,
        port_index: usize,
        next_hop_mac: MacAddr,
    ) -> bool {
        if network == 0 || network == 0xFFFF {
            return false;
        }
        if self
            .routes
            .get(&network)
            .is_some_and(|entry| entry.directly_connected)
        {
            return false;
        }
        self.add_learned(network, port_index, next_hop_mac);
        true
    }

    /// Apply one Initialize-Routing-Table purge entry (Port ID 0, 6.4.7).
    ///
    /// "All table entries for the specified DNET shall be purged from the
    /// table" — scoped to learned entries here for the same direct-safety
    /// reason as [`Self::apply_management_update`]. Returns `true` only when
    /// a learned entry was actually removed.
    pub(crate) fn apply_management_removal(&mut self, network: u16) -> bool {
        if network == 0 || network == 0xFFFF {
            return false;
        }
        if self
            .routes
            .get(&network)
            .is_some_and(|entry| entry.directly_connected)
        {
            return false;
        }
        self.remove(network).is_some()
    }

    /// Add a learned route immediately (manual table update, without corroboration).
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
                    self.claim_counters.flap_warned =
                        self.claim_counters.flap_warned.saturating_add(1);
                    tracing::warn!(
                        network,
                        old_port = existing.port_index,
                        new_port = port_index,
                        flap_count,
                        "Route flapping detected — network changed ports {} times in 60s",
                        flap_count
                    );
                }
                self.reject_transitions.remove(&network);
                self.pending_replacements.remove(&network);
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
    /// Directly-connected paths are never overridden by Busy: they are served
    /// locally, not via the announcing peer. A permanently Unreachable entry
    /// is likewise left untouched: a temporary congestion claim must never
    /// resurrect a failed route via the 30s auto-clear — only Available or
    /// fresh learning lifts Unreachable.
    pub fn mark_busy(&mut self, network: u16, deadline: Instant) {
        if let Some(entry) = self.routes.get_mut(&network) {
            if !entry.directly_connected && entry.reachability != ReachabilityStatus::Unreachable {
                entry.reachability = ReachabilityStatus::Busy;
                entry.busy_until = Some(deadline);
            }
        }
    }

    /// Mark a network as available, clearing any busy state (spec 6.6.3.7).
    /// Directly-connected paths are never touched: Busy/Available only covers
    /// routes served via the announcing peer.
    pub fn mark_available(&mut self, network: u16) {
        if let Some(entry) = self.routes.get_mut(&network) {
            if !entry.directly_connected {
                entry.reachability = ReachabilityStatus::Reachable;
                entry.busy_until = None;
            }
        }
    }

    /// Whether `network` is served via the announcing peer on this ingress
    /// path: a learned (never directly-connected) route whose egress port and
    /// next-hop MAC match the immediate link peer — not the routed SNET/SADR.
    /// RB-04 locks this predicate to 135-2020 Clauses 6.6.3.6/6.6.3.7: "If the
    /// 2-octet network numbers are omitted, it means the router wishes to
    /// stop the flow of messages to all the networks it normally serves"
    /// (Busy; Available re-enables "the flow of messages to all the networks
    /// it serves"). Explicit lists intersect with the same set: listed nets
    /// outside the announcing path are not marked.
    pub fn is_served_via_peer(&self, network: u16, port_index: usize, next_hop: &MacAddr) -> bool {
        self.routes.get(&network).is_some_and(|entry| {
            !entry.directly_connected
                && entry.port_index == port_index
                && entry.next_hop_mac == *next_hop
        })
    }

    /// All networks served via the announcing peer: the omitted-list scope
    /// for Router-Busy/Router-Available-To-Network (Clauses 6.6.3.6/6.6.3.7).
    pub fn routes_served_via_peer(&self, port_index: usize, next_hop: &MacAddr) -> Vec<u16> {
        self.routes
            .iter()
            .filter(|(_, entry)| {
                !entry.directly_connected
                    && entry.port_index == port_index
                    && entry.next_hop_mac == *next_hop
            })
            .map(|(net, _)| *net)
            .collect()
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
    /// Clears any pending challenger since the caller can replace the route state.
    pub fn lookup_mut(&mut self, network: u16) -> Option<&mut RouteEntry> {
        self.pending_replacements.remove(&network);
        self.routes.get_mut(&network)
    }

    /// Remove a route.
    pub fn remove(&mut self, network: u16) -> Option<RouteEntry> {
        self.reject_transitions.remove(&network);
        self.pending_replacements.remove(&network);
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
            self.remove(*net);
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
#[path = "router_table_reject_tests.rs"]
mod reject_tests;

#[cfg(test)]
#[path = "router_table_corroboration_tests.rs"]
mod corroboration_tests;

#[cfg(test)]
#[path = "router_table_tests.rs"]
mod tests;
