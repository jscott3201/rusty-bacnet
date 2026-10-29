//! The Networks tab's map (#960): which router on the local network
//! reaches which network, built from the I-Am-Router-To-Network messages
//! heard. A network more than one router claims is flagged: two next hops
//! to one network number mean two networks share the number, or a loop.
//!
//! A pure function of what was heard, with no clock and no transport.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

/// Maximum routers retained; least recently heard is evicted first.
pub(crate) const MAX_ROUTERS: usize = 128;
/// Maximum networks per router; at most 16,384 router/network edges in total.
pub(crate) const MAX_ROUTER_NETWORKS: usize = 128;

/// One network on the map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NetworkRow {
    /// The network number.
    pub(crate) network: u16,
    /// Every router that listed it, by address, ascending.
    pub(crate) routers: Vec<String>,
    /// When a router last listed it.
    pub(crate) heard: Instant,
}

impl NetworkRow {
    /// Whether more than one router claims the network.
    pub(crate) fn duplicate(&self) -> bool {
        self.routers.len() > 1
    }
}

/// The routers heard, and the networks each reaches.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RouterMap {
    /// Router address to the networks it listed, with when each was last
    /// listed.
    routers: BTreeMap<String, BTreeMap<u16, Instant>>,
}

impl RouterMap {
    /// Record an I-Am-Router-To-Network from `router` listing `networks`.
    /// A router's networks add up across its messages: one answering a
    /// question about a single network lists only that one.
    pub(crate) fn record(&mut self, router: &str, networks: &[u16], at: Instant) {
        if !self.routers.contains_key(router) && self.routers.len() == MAX_ROUTERS {
            let oldest = self
                .routers
                .iter()
                .min_by_key(|(address, networks)| (networks.values().max().copied(), *address))
                .map(|(address, _)| address.clone())
                .unwrap();
            self.routers.remove(&oldest);
        }
        let entry = self.routers.entry(router.to_string()).or_default();
        for &network in networks {
            if !entry.contains_key(&network) && entry.len() == MAX_ROUTER_NETWORKS {
                let oldest = *entry
                    .iter()
                    .min_by_key(|(network, at)| (*at, *network))
                    .unwrap()
                    .0;
                entry.remove(&oldest);
            }
            entry.insert(network, at);
        }
    }

    /// How many routers have been heard.
    pub(crate) fn router_count(&self) -> usize {
        self.routers.len()
    }

    /// Whether no router has been heard.
    pub(crate) fn is_empty(&self) -> bool {
        self.routers.is_empty()
    }

    /// Every network heard, ascending, with the routers that reach it.
    pub(crate) fn rows(&self) -> Vec<NetworkRow> {
        let mut rows: BTreeMap<u16, NetworkRow> = BTreeMap::new();
        for (router, networks) in &self.routers {
            for (&network, &at) in networks {
                let row = rows.entry(network).or_insert_with(|| NetworkRow {
                    network,
                    routers: Vec::new(),
                    heard: at,
                });
                row.routers.push(router.clone());
                row.heard = row.heard.max(at);
            }
        }
        rows.into_values().collect()
    }

    /// The networks more than one router claims, ascending.
    pub(crate) fn duplicates(&self) -> BTreeSet<u16> {
        self.rows()
            .into_iter()
            .filter(NetworkRow::duplicate)
            .map(|row| row.network)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn networks_map_to_their_routers_and_duplicates_are_flagged() {
        let t0 = Instant::now();
        let later = t0 + Duration::from_secs(5);
        let mut map = RouterMap::default();
        assert!(map.is_empty());
        map.record("10.0.0.1:47808", &[2, 3], t0);
        map.record("10.0.0.9:47808", &[4], t0);
        assert_eq!(map.duplicates(), BTreeSet::new());
        // A second router claims network 3; the first is heard again.
        map.record("10.0.0.7:47808", &[3], later);
        map.record("10.0.0.1:47808", &[2], later);
        assert_eq!(map.router_count(), 3);
        let rows = map.rows();
        let networks: Vec<u16> = rows.iter().map(|row| row.network).collect();
        assert_eq!(networks, [2, 3, 4]);
        assert_eq!(rows[0].routers, ["10.0.0.1:47808"]);
        assert_eq!(rows[0].heard, later);
        assert!(!rows[0].duplicate());
        assert_eq!(rows[1].routers, ["10.0.0.1:47808", "10.0.0.7:47808"]);
        assert!(rows[1].duplicate());
        assert_eq!(rows[1].heard, later);
        assert_eq!(rows[2].heard, t0);
        assert_eq!(map.duplicates(), BTreeSet::from([3]));
    }

    #[test]
    fn a_router_heard_twice_is_one_router() {
        let t0 = Instant::now();
        let mut map = RouterMap::default();
        map.record("02", &[7], t0);
        map.record("02", &[7, 8], t0);
        assert_eq!(map.router_count(), 1);
        assert_eq!(map.duplicates(), BTreeSet::new());
        assert_eq!(map.rows().len(), 2);
    }
}
