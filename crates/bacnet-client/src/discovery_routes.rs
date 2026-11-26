//! The router a routed row names, and what may change it (#2823).
//!
//! A routed row's `mac_address` is the station on this link that requests
//! to the device go through. The standard leaves the way a node learns that
//! station to the node, among them the source of the device's I-Am, a
//! router-discovery query, and the source of any frame from the network
//! (Clause 6.5.3), and none of those frames is authenticated. So the table
//! takes the router from the first I-Am that makes the row, and never from
//! a later one that arrives through another station: that I-Am changes
//! nothing on the row and is kept as a cue. The client's own check
//! (`client::route_checks`) decides, and only an answer to a request of its
//! own moves the row: see [`DeviceTable::reroute`].

use super::*;

/// The most paths with a cue waiting at once. A cue for another path past
/// that is dropped: the row stays as it is, and the device's next I-Am
/// through the other station cues it again.
const MAX_ROUTE_CUES: usize = 256;

/// A network and the router its rows name: one path the client checks.
pub(crate) type PathKey = (u16, MacAddr);

/// Routed I-Ams heard through another station than the router of the row
/// they name, waiting for the client's check: for each network and router,
/// the station of the latest one, which the check asks.
#[derive(Debug, Default)]
pub(super) struct RouteCues {
    waiting: BTreeMap<PathKey, MacAddr>,
}

impl DeviceTable {
    /// Whether `device`, from a routed I-Am, names a row that reaches the
    /// device through another router. Such a row keeps everything, its
    /// router, instance, capabilities and ages included, and the I-Am is
    /// kept as a cue for the path's check. A new address (first sight) and
    /// a local row are not held.
    pub(super) fn held_route(&mut self, device: &DiscoveredDevice) -> bool {
        let Some(key) = Key::of(device) else {
            return false;
        };
        if key.network == 0 {
            return false;
        }
        let Some(row) = self.rows.get(&key) else {
            return false;
        };
        if row.device.mac_address == device.mac_address {
            return false;
        }
        let path = (key.network, row.device.mac_address.clone());
        let waiting = &mut self.route_cues.waiting;
        if let Some(station) = waiting.get_mut(&path) {
            *station = key.mac;
        } else if waiting.len() < MAX_ROUTE_CUES {
            waiting.insert(path, key.mac);
        }
        true
    }

    /// Take the waiting cues: each path, and the station on its network
    /// that the latest cue named.
    pub(crate) fn take_route_cues(&mut self) -> Vec<(PathKey, MacAddr)> {
        std::mem::take(&mut self.route_cues.waiting)
            .into_iter()
            .collect()
    }

    /// The rows on `network` whose router is `router`.
    fn through<'a>(
        &'a self,
        network: u16,
        router: &'a [u8],
    ) -> impl Iterator<Item = &'a TableEntry> + 'a {
        self.rows
            .range(Key::at(network, &[])..)
            .take_while(move |(key, _)| key.network == network)
            .map(|(_, entry)| entry)
            .filter(move |entry| entry.device.mac_address.as_slice() == router)
    }

    /// Whether any row on `network` still goes through `router`.
    pub(crate) fn routes_through(&self, network: u16, router: &[u8]) -> bool {
        self.through(network, router).next().is_some()
    }

    /// Whether a row on `network` that goes through `router` was heard from
    /// at `since` or later: an I-Am that came through that router, or an
    /// answer to a request sent through it. A station other than the
    /// router can move neither: its I-Ams are held, and the answers that
    /// count complete this client's own requests.
    pub(crate) fn heard_through_since(&self, network: u16, router: &[u8], since: Instant) -> bool {
        self.through(network, router)
            .any(|entry| entry.device.last_seen >= since)
    }

    /// Every row on `network` that went through `from` goes through `to`
    /// now. Called only by the client's check, after its own request
    /// through `from` drew nothing and its own router-discovery query was
    /// answered by `to` alone. Returns the rows as they are now; nothing
    /// else on them changes.
    pub(crate) fn reroute(
        &mut self,
        network: u16,
        from: &[u8],
        to: &[u8],
    ) -> Vec<DiscoveredDevice> {
        self.rows
            .range_mut(Key::at(network, &[])..)
            .take_while(|(key, _)| key.network == network)
            .map(|(_, entry)| &mut entry.device)
            .filter(|device| device.mac_address.as_slice() == from)
            .map(|device| {
                device.mac_address = MacAddr::from_slice(to);
                device.clone()
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "discovery_routes_tests.rs"]
mod tests;
