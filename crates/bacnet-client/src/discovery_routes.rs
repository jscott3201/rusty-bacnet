//! The router a known routed row names, and what may change it (#2823).
//!
//! A routed row's `mac_address` is the station on this link that requests
//! to the device go through. The standard leaves the way a node learns that
//! station to the node, among them the source of the device's I-Am, a
//! router-discovery query, and the source of any frame from the network
//! (Clause 6.5.3), and none of those frames is authenticated. So the table
//! takes the router from the first I-Am that makes the row, and a later one
//! that arrives through another station changes nothing on that row: it
//! marks the path for the client's own check (`client::route_checks`), and
//! only answers to requests of the client's own, in two checks about a
//! minute apart, move the row ([`DeviceTable::reroute`]).
//!
//! The paths waiting for a check, and each one's pacing, live here, under
//! the table's lock, in one store of at most [`MAX_PATHS`]. A path is ranked
//! by how long the oldest row through it has been known and then by how
//! many rows go through it, so a long-known path is checked first and is
//! never pushed out for a newer one.

use std::cmp::Ordering;

use bacnet_npdu::RemoteBroadcast;

use super::*;

/// The most paths waiting at once. A cue for a new path when the store is
/// full takes the place of the lowest-ranked path, if it outranks it, and
/// is dropped otherwise: the row stays as it is either way.
pub(crate) const MAX_PATHS: usize = 256;

/// How long after a path's first check the next may run. Each check that
/// leaves the path as it was doubles the hold, up to [`LAST_HOLD`].
pub(crate) const FIRST_HOLD: Duration = Duration::from_secs(60);

/// The longest hold: as long as a silent row takes to age out of the
/// client's table.
pub(crate) const LAST_HOLD: Duration = Duration::from_secs(600);

/// How long after a check that found the path's router silent and one
/// other router alone answering the second check runs, whatever the hold:
/// the row moves only when that one finds the same.
pub(crate) const STRIKE_GAP: Duration = Duration::from_secs(60);

/// A network and the router its rows name: one path the client checks.
pub(crate) type PathKey = (u16, MacAddr);

/// How much a path matters: how long the oldest row through it has been
/// known, then how many rows go through it. Rows a first sight made are
/// cheap to make in numbers, so age comes first: a long-known path is
/// checked before new ones and kept longer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rank {
    rows: usize,
    oldest: Instant,
}

impl Ord for Rank {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .oldest
            .cmp(&self.oldest)
            .then_with(|| self.rows.cmp(&other.rows))
    }
}

impl PartialOrd for Rank {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// One path's state.
#[derive(Debug)]
struct Paced {
    /// A cue came since the last check.
    wanted: bool,
    /// The next check runs no sooner.
    due: Instant,
    /// The hold after the next check.
    hold: Duration,
    /// The rank when last cued or checked, for eviction.
    rank: Rank,
    /// The first strike, when the last check found the router silent and
    /// one other router alone answering: that router, and when the check
    /// began.
    strike: Option<Strike>,
}

/// A check that found a path's router silent and one other router alone
/// answering its router query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Strike {
    /// The router that answered alone.
    pub(crate) to: MacAddr,
    /// When the check began: anything the old router carried from the
    /// network since then clears the strike.
    pub(crate) since: Instant,
}

/// The paths waiting for the client's check.
#[derive(Debug, Default)]
pub(super) struct RoutePaths {
    paths: BTreeMap<PathKey, Paced>,
}

/// What a check of a path decided, for its pacing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Checked {
    /// The path stays, and a strike is cleared: the next check waits for a
    /// cue and the hold.
    Kept,
    /// The first strike: the second check runs [`STRIKE_GAP`] later.
    Struck(Strike),
    /// The rows moved, or none goes through the path any more: it is
    /// forgotten.
    Ended,
}

/// The rows on `network` whose router is `router`.
fn through<'a>(
    rows: &'a BTreeMap<Key, TableEntry>,
    network: u16,
    router: &'a [u8],
) -> impl Iterator<Item = (&'a Key, &'a TableEntry)> + 'a {
    rows.range(Key::at(network, &[])..)
        .take_while(move |(key, _)| key.network == network)
        .filter(move |(_, entry)| entry.device.mac_address.as_slice() == router)
}

/// The rank of the path through `router` to `network`: `None` when no row
/// goes through it.
fn rank(rows: &BTreeMap<Key, TableEntry>, network: u16, router: &[u8]) -> Option<Rank> {
    through(rows, network, router).fold(None, |rank, (_, entry)| {
        Some(match rank {
            None => Rank {
                rows: 1,
                oldest: entry.since,
            },
            Some(Rank { rows, oldest }) => Rank {
                rows: rows + 1,
                oldest: oldest.min(entry.since),
            },
        })
    })
}

impl DeviceTable {
    /// Whether `device`, from a routed I-Am, names a row that reaches the
    /// device through another router. Such a row keeps everything, its
    /// router, instance, capabilities and ages included, and its path is
    /// marked for a check. A new address (first sight) and a local row are
    /// not held.
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
        if let Some(rank) = rank(&self.rows, path.0, &path.1) {
            self.route_paths.cue(path, rank);
        }
        true
    }

    /// Up to `limit` paths whose check is due at `now`, the highest ranked
    /// first. Paths no row goes through any more, and paths left alone for a
    /// whole hold past their due, are forgotten first.
    pub(crate) fn due_paths(&mut self, now: Instant, limit: usize) -> Vec<PathKey> {
        let rows = &self.rows;
        let paths = &mut self.route_paths.paths;
        paths.retain(|(network, router), paced| {
            let Some(current) = rank(rows, *network, router) else {
                return false;
            };
            paced.rank = current;
            paced.wanted || paced.due.checked_add(LAST_HOLD).is_none_or(|end| now < end)
        });
        let mut due: Vec<(&PathKey, &Paced)> = paths
            .iter()
            .filter(|(_, paced)| paced.wanted && paced.due <= now)
            .collect();
        due.sort_by_key(|(_, paced)| std::cmp::Reverse(paced.rank));
        due.into_iter()
            .take(limit)
            .map(|(path, _)| path.clone())
            .collect()
    }

    /// When the next check is due: the earliest path with a cue waiting.
    pub(crate) fn next_path_due(&self) -> Option<Instant> {
        self.route_paths
            .paths
            .values()
            .filter(|paced| paced.wanted)
            .map(|paced| paced.due)
            .min()
    }

    /// The first strike `path` holds, if its last check made one.
    pub(crate) fn path_strike(&self, path: &PathKey) -> Option<Strike> {
        self.route_paths
            .paths
            .get(path)
            .and_then(|paced| paced.strike.clone())
    }

    /// Record a check of `path` that ended at `now`.
    pub(crate) fn path_checked(&mut self, path: &PathKey, checked: Checked, now: Instant) {
        let paths = &mut self.route_paths.paths;
        match checked {
            Checked::Ended => {
                paths.remove(path);
            }
            Checked::Kept => {
                if let Some(paced) = paths.get_mut(path) {
                    paced.strike = None;
                    paced.wanted = false;
                    paced.due = now + paced.hold;
                    paced.hold = (paced.hold * 2).min(LAST_HOLD);
                }
            }
            Checked::Struck(strike) => {
                if let Some(paced) = paths.get_mut(path) {
                    paced.strike = Some(strike);
                    paced.wanted = true;
                    paced.due = now + STRIKE_GAP;
                }
            }
        }
    }

    /// The station a check asks through `router`: of the rows on `network`
    /// that go through it, the one whose own I-Am through it came last, then
    /// the one heard from last, by this table's own record. An answer the
    /// transaction matching accepted moves only the second, so it can't
    /// make a quiet row the one asked. Never a station a cue named, and
    /// never an address that is a broadcast or group form on some link
    /// ([`RemoteBroadcast`]).
    pub(crate) fn probe_target(&self, network: u16, router: &[u8]) -> Option<MacAddr> {
        through(&self.rows, network, router)
            .filter(|(key, _)| RemoteBroadcast::of(&key.mac, false).is_none())
            .max_by_key(|(_, entry)| (entry.device.last_announced, entry.device.last_seen))
            .map(|(key, _)| key.mac.clone())
    }

    /// Whether any row on `network` still goes through `router`.
    pub(crate) fn routes_through(&self, network: u16, router: &[u8]) -> bool {
        through(&self.rows, network, router).next().is_some()
    }

    /// Whether a row on `network` that goes through `router` was heard from
    /// at `since` or later: any frame that router carried from the device,
    /// or an answer to a request sent through it. Another station's I-Ams
    /// are held and its frames refresh nothing, with two exceptions that can
    /// only keep a path, never move one: an answer the client's transaction
    /// matching accepted, and on B/IP a forwarded frame whose origin a BBMD
    /// gives as the router's (#2829).
    pub(crate) fn heard_through_since(&self, network: u16, router: &[u8], since: Instant) -> bool {
        through(&self.rows, network, router).any(|(_, entry)| entry.device.last_seen >= since)
    }

    /// A frame from the device at `station` on `network` came through
    /// `router`: when that is the router its row names, the row's age
    /// starts again, as an answer to a request starts it. A frame through
    /// any other station changes nothing.
    pub(crate) fn heard_through(&mut self, network: u16, station: &[u8], router: &[u8]) {
        if network == 0 {
            return;
        }
        if let Some(entry) = self.rows.get_mut(&Key::at(network, station))
            && entry.device.mac_address.as_slice() == router
        {
            entry.device.last_seen = entry.device.last_seen.max(now());
        }
    }

    /// Every row on `network` that went through `from` goes through `to`
    /// now. Called only by the client's check, after its own requests showed
    /// `from` silent and `to` alone answering its router query, twice, about
    /// a minute apart. Returns the
    /// rows as they are now; nothing else on them changes.
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

impl DeviceTable {
    /// An explicit registration gave the row at `device`'s address a router:
    /// when it differs from the row's, the strike the old path held is
    /// cleared, so the rows left on it start their checks again.
    pub(super) fn registered_router(&mut self, device: &DiscoveredDevice) {
        let Some(key) = Key::of(device) else {
            return;
        };
        if let Some(row) = self.rows.get(&key)
            && row.device.mac_address != device.mac_address
            && let Some(paced) = self
                .route_paths
                .paths
                .get_mut(&(key.network, row.device.mac_address.clone()))
        {
            paced.strike = None;
        }
    }
}

impl RoutePaths {
    /// Mark `path`, ranked `rank`, for a check: at once when it is new.
    fn cue(&mut self, path: PathKey, rank: Rank) {
        let now = now();
        if let Some(paced) = self.paths.get_mut(&path) {
            paced.wanted = true;
            paced.rank = rank;
            return;
        }
        if self.paths.len() >= MAX_PATHS {
            let lowest = self
                .paths
                .iter()
                .min_by_key(|(_, paced)| paced.rank)
                .map(|(path, paced)| (path.clone(), paced.rank));
            match lowest {
                Some((lowest, below)) if below < rank => {
                    self.paths.remove(&lowest);
                }
                _ => return,
            }
        }
        self.paths.insert(
            path,
            Paced {
                wanted: true,
                due: now,
                hold: FIRST_HOLD,
                rank,
                strike: None,
            },
        );
    }
}

#[cfg(test)]
#[path = "discovery_routes_tests.rs"]
mod tests;
