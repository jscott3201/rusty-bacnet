//! Device discovery table: the devices the client has heard, one row per
//! address, with an index from each Device instance to the addresses that
//! claim it.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::time::{Duration, Instant};

use bacnet_types::MacAddr;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::Segmentation;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

/// The most rows the table holds, one per address.
const MAX_DEVICE_TABLE_ENTRIES: usize = 4096;

/// The clock the client ages rows by: Tokio's, which reads the system's
/// monotonic clock unless a test has paused it, so a test's virtual time
/// ages rows too.
pub(crate) fn now() -> Instant {
    tokio::time::Instant::now().into_std()
}

/// Information about a discovered BACnet device.
#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    /// The device's object identifier (always ObjectType::DEVICE).
    pub object_identifier: ObjectIdentifier,
    /// The MAC address from which the IAm was received: the device itself,
    /// or for a routed device the router it was heard through.
    pub mac_address: MacAddr,
    /// Maximum APDU length the device accepts.
    pub max_apdu_length: u32,
    /// Segmentation support level.
    pub segmentation_supported: Segmentation,
    /// Maximum segments the remote device accepts (None = unlimited/unspecified).
    pub max_segments_accepted: Option<u32>,
    /// Vendor identifier.
    pub vendor_id: u16,
    /// When the device was last heard from: its latest I-Am, or its latest
    /// answer to one of this client's confirmed requests.
    pub last_seen: Instant,
    /// If this device is behind a router, the BACnet network number it resides on.
    pub source_network: Option<u16>,
    /// If this device is behind a router, its MAC address on the remote network.
    pub source_address: Option<MacAddr>,
}

impl DiscoveredDevice {
    /// True when this row represents a local peer: neither routing field is
    /// set. See the [`DeviceTable`] address-space invariant.
    pub fn is_local(&self) -> bool {
        self.source_network.is_none() && self.source_address.is_none()
    }

    /// Where the device answers, which identifies it in the [`DeviceTable`]:
    /// network 0 and `mac_address` for a local device, `source_network` and
    /// `source_address` for a routed one. `None` when the routing fields name
    /// no one station: only one of them is set, the network is 0 or 65535,
    /// or the remote MAC is empty.
    pub fn address(&self) -> Option<BACnetAddress> {
        Key::of(self).map(Key::address)
    }
}

/// Manual registration details for a known routed peer, for use with
/// [`crate::client::BACnetClient::add_routed_device`].
///
/// Everything a caller must supply so requests to the peer route correctly
/// and are sized by the peer's advertised limits rather than local defaults.
#[derive(Debug, Clone)]
pub struct RoutedDeviceConfig {
    /// Device instance number.
    pub instance: u32,
    /// MAC of the immediate-hop router (transport next hop, not peer identity).
    pub router_mac: Vec<u8>,
    /// Remote network number (SNET) the peer resides on.
    pub remote_network: u16,
    /// Peer's MAC on the remote network (SADR).
    pub remote_mac: Vec<u8>,
    /// The peer's advertised Max APDU Length Accepted.
    pub max_apdu_length: u32,
    /// The peer's advertised segmentation capability.
    pub segmentation_supported: Segmentation,
    /// The peer's Max Segments Accepted, if known (None = unspecified).
    pub max_segments_accepted: Option<u32>,
}

/// Provenance of a row's `Segmentation_Supported` value (Clause 12.11).
///
/// Only authoritative capability may drive a preflight refusal: a legacy
/// manual registration stores placeholder defaults that must not be mistaken
/// for a peer advertising NO_SEGMENTATION (#371).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PeerSegmentation {
    /// The value is authoritative: learned from the peer's I-Am or explicitly
    /// supplied by a caller (`add_routed_device`, `DeviceTable::upsert`).
    Authoritative(Segmentation),
    /// The row stores manual placeholder defaults; capability is unknown.
    Placeholder,
}

#[derive(Debug)]
struct TableEntry {
    device: DiscoveredDevice,
    segmentation: PeerSegmentation,
}

/// A row's address: the network (0 for the local one) and the MAC there.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    network: u16,
    mac: MacAddr,
}

impl Key {
    fn at(network: u16, mac: &[u8]) -> Self {
        Self {
            network,
            mac: MacAddr::from_slice(mac),
        }
    }

    /// The device's address, unless its routing fields name no one station:
    /// one of them alone, network 0 or 65535, or an empty remote MAC.
    fn of(device: &DiscoveredDevice) -> Option<Self> {
        match (device.source_network, &device.source_address) {
            (None, None) => Some(Self::at(0, &device.mac_address)),
            (Some(network @ 1..=0xFFFE), Some(mac)) if !mac.is_empty() => {
                Some(Self::at(network, mac))
            }
            _ => None,
        }
    }

    /// Sorts before every key, so `(instance, Key::lowest())` starts the
    /// instance's run of claims.
    fn lowest() -> Self {
        Self {
            network: 0,
            mac: MacAddr::new(),
        }
    }

    fn address(self) -> BACnetAddress {
        BACnetAddress {
            network_number: self.network,
            mac_address: self.mac,
        }
    }
}

/// Thread-safe device discovery table.
///
/// # Identity
///
/// A row is keyed by where the device answers: the network number, 0 for the
/// local network, and the MAC on that network. Hearing an address again
/// refreshes its row in place, whatever instance, capabilities or vendor it
/// now announces. A routed device's router MAC is how to reach its network,
/// kept on the row, and not part of the key, so the same MAC on two networks
/// is two devices, and one device heard through a new router keeps its row.
///
/// An index maps each Device instance to the addresses whose rows claim it.
/// Two or more addresses can claim one instance, and every one keeps its row:
/// each stays reachable by its address, and a lookup by that instance
/// ([`find`](Self::find)) refuses with [`Error::AmbiguousDevice`]. The
/// ambiguity ends when only one claimant is left, because a row aged out
/// ([`purge_stale`](Self::purge_stale)) or its address announced another
/// instance.
///
/// The table holds at most 4,096 rows, one per address: an instance two
/// addresses claim takes two. A new address beyond that is dropped; a known
/// address is still refreshed.
///
/// # Address-space invariant
///
/// A row is *routed* when it carries routed-source identity (`source_network`
/// and `source_address`, per Clause 6.2.2 the original source SNET/SADR); its
/// `mac_address` is then only the immediate-hop router, shared by every peer
/// behind that router. A row is *local* only when it is unambiguously local:
/// both routing fields are `None`. A device whose routing fields name no one
/// station ([`DiscoveredDevice::address`] is `None`) has no row: the table
/// doesn't store it.
///
/// # Capability provenance
///
/// Each row records whether its `segmentation_supported` is authoritative
/// (an I-Am, explicit [`upsert`][DeviceTable::upsert], or
/// `add_routed_device`) or a legacy placeholder (`add_device`). Provenance
/// lives in the same table entry as the device data, so it stays coherent
/// across insert, update, purge, and clear.
#[derive(Debug, Default)]
pub struct DeviceTable {
    rows: BTreeMap<Key, TableEntry>,
    /// Every row's instance and address, so the rows that claim one
    /// instance are found without a scan.
    claims: BTreeSet<(u32, Key)>,
}

/// What an I-Am did to the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Change {
    /// A row for a new address.
    Inserted,
    /// The address's row, refreshed: the same instance.
    Updated,
    /// The address announced another instance: one device at the address
    /// took the place of another, whose row is [`Heard::replaced`].
    Replaced,
    /// Not stored: the table is full, or the device names no address.
    Dropped,
}

/// [`DeviceTable::heard`]'s outcome.
#[derive(Debug)]
pub(crate) struct Heard {
    pub(crate) change: Change,
    /// The row a [`Change::Replaced`] took the place of.
    pub(crate) replaced: Option<DiscoveredDevice>,
    /// Instances whose claimants changed while two or more claimed them,
    /// before or after the change.
    pub(crate) contested: Vec<u32>,
}

/// [`DeviceTable::purge_stale_collect`]'s outcome.
#[derive(Debug, Default)]
pub(crate) struct Purged {
    /// The rows removed.
    pub(crate) lost: Vec<DiscoveredDevice>,
    /// Instances two or more addresses claimed before the purge, of which it
    /// removed at least one.
    pub(crate) contested: Vec<u32>,
}

impl DeviceTable {
    /// Create an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a device, or refresh the row at its address.
    ///
    /// The row's `segmentation_supported` is treated as authoritative
    /// capability (explicit configuration); see `PeerSegmentation`. A row
    /// another address holds for the same instance stays, and the instance
    /// is then ambiguous. A device with no address
    /// ([`DiscoveredDevice::address`]), or a new address once the table is
    /// full, is not stored.
    pub fn upsert(&mut self, device: DiscoveredDevice) {
        let segmentation = PeerSegmentation::Authoritative(device.segmentation_supported);
        let _ = self.insert(device, segmentation);
    }

    /// Ingest a device heard in an I-Am: [`upsert`](Self::upsert), saying
    /// what changed.
    pub(crate) fn heard(&mut self, device: DiscoveredDevice) -> Heard {
        let segmentation = PeerSegmentation::Authoritative(device.segmentation_supported);
        self.insert(device, segmentation)
    }

    /// Insert or update a device whose capability fields are manual
    /// placeholders rather than learned or explicitly supplied values
    /// (legacy `add_device`). Its `segmentation_supported` must never drive
    /// a capability decision (#371).
    pub(crate) fn upsert_placeholder(&mut self, device: DiscoveredDevice) {
        let _ = self.insert(device, PeerSegmentation::Placeholder);
    }

    fn insert(&mut self, device: DiscoveredDevice, segmentation: PeerSegmentation) -> Heard {
        let mut heard = Heard {
            change: Change::Dropped,
            replaced: None,
            contested: Vec::new(),
        };
        let Some(key) = Key::of(&device) else {
            return heard;
        };
        let instance = device.object_identifier.instance_number();
        let entry = TableEntry {
            device,
            segmentation,
        };
        let full = self.rows.len() >= MAX_DEVICE_TABLE_ENTRIES;
        let previous = match self.rows.get_mut(&key) {
            Some(row) => {
                let old = std::mem::replace(row, entry).device;
                let previous = old.object_identifier.instance_number();
                if previous == instance {
                    heard.change = Change::Updated;
                    return heard;
                }
                heard.change = Change::Replaced;
                heard.replaced = Some(old);
                self.claims.remove(&(previous, key.clone()));
                Some(previous)
            }
            None if full => return heard,
            None => {
                self.rows.insert(key.clone(), entry);
                heard.change = Change::Inserted;
                None
            }
        };
        self.claims.insert((instance, key));
        // The address moved to `instance`: a contest it left ends or shrinks,
        // and one it joined begins or grows.
        if let Some(previous) = previous
            && self.claims_of(previous).next().is_some()
        {
            heard.contested.push(previous);
        }
        if self.claims_of(instance).nth(1).is_some() {
            heard.contested.push(instance);
        }
        heard
    }

    /// The addresses that claim `instance`, in order.
    fn claims_of(&self, instance: u32) -> impl Iterator<Item = &Key> + '_ {
        let end = match instance.checked_add(1) {
            Some(next) => Bound::Excluded((next, Key::lowest())),
            None => Bound::Unbounded,
        };
        self.claims
            .range((Bound::Included((instance, Key::lowest())), end))
            .map(|(_, key)| key)
    }

    /// Every row, ordered by network number and then MAC.
    pub fn all(&self) -> Vec<DiscoveredDevice> {
        self.rows.values().map(|e| e.device.clone()).collect()
    }

    /// The row at `mac` on `network`: network 0 is the local network, where
    /// `mac` is the device's own; on a remote network it is the device's
    /// MAC there (SADR), never the router's.
    pub fn get(&self, network: u16, mac: &[u8]) -> Option<&DiscoveredDevice> {
        self.entry(network, mac).map(|e| &e.device)
    }

    /// The entry at `mac` on `network`. A few rows, as a small site or a
    /// client naming its devices by address holds, are scanned, which costs
    /// less on the request path than building a key to search with.
    fn entry(&self, network: u16, mac: &[u8]) -> Option<&TableEntry> {
        const SCANNED: usize = 4;
        if self.rows.len() <= SCANNED {
            return self
                .rows
                .iter()
                .find(|(key, _)| key.network == network && key.mac.as_slice() == mac)
                .map(|(_, entry)| entry);
        }
        self.rows.get(&Key::at(network, mac))
    }

    /// The rows that claim `instance`, ordered by address: none, one, or
    /// more when the instance is ambiguous.
    pub fn claiming(&self, instance: u32) -> impl Iterator<Item = &DiscoveredDevice> + '_ {
        self.claims_of(instance)
            .filter_map(|key| self.rows.get(key).map(|e| &e.device))
    }

    /// The one row that claims `instance`, or `Ok(None)` when no row does.
    ///
    /// # Errors
    ///
    /// [`Error::AmbiguousDevice`], naming every claimant's address, when two
    /// or more rows claim it.
    pub fn find(&self, instance: u32) -> Result<Option<&DiscoveredDevice>, Error> {
        let mut claims = self.claims_of(instance);
        let Some(first) = claims.next() else {
            return Ok(None);
        };
        if claims.next().is_none() {
            return Ok(self.rows.get(first).map(|e| &e.device));
        }
        Err(Error::AmbiguousDevice {
            instance,
            candidates: self
                .claims_of(instance)
                .cloned()
                .map(Key::address)
                .collect(),
        })
    }

    /// The row at `mac` on `network`, as [`get`](Self::get) finds it, with
    /// its segmentation provenance: request sizing reads both under one
    /// borrow, so they come from the same row.
    pub(crate) fn peer(
        &self,
        network: u16,
        mac: &[u8],
    ) -> Option<(&DiscoveredDevice, PeerSegmentation)> {
        self.entry(network, mac)
            .map(|e| (&e.device, e.segmentation))
    }

    /// The device at `mac` on `network` answered a confirmed request: its
    /// row's age starts again, so a device that keeps answering is not
    /// purged for want of an I-Am.
    pub(crate) fn answered(&mut self, network: u16, mac: &[u8]) {
        if let Some(entry) = self.rows.get_mut(&Key::at(network, mac)) {
            entry.device.last_seen = entry.device.last_seen.max(now());
        }
    }

    /// Clear all entries.
    pub fn clear(&mut self) {
        self.rows.clear();
        self.claims.clear();
    }

    /// Number of rows, one per address.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Remove entries whose `last_seen` is older than `max_age`.
    pub fn purge_stale(&mut self, max_age: Duration) {
        let _ = self.purge_stale_at(now(), max_age);
    }

    pub(crate) fn purge_stale_collect(&mut self, max_age: Duration) -> Purged {
        self.purge_stale_at(now(), max_age)
    }

    fn purge_stale_at(&mut self, now: Instant, max_age: Duration) -> Purged {
        let stale: Vec<Key> = self
            .rows
            .iter()
            .filter(|(_, entry)| {
                now.checked_duration_since(entry.device.last_seen)
                    .is_some_and(|age| age > max_age)
            })
            .map(|(key, _)| key.clone())
            .collect();
        let mut purged = Purged::default();
        for key in stale {
            let Some(entry) = self.rows.remove(&key) else {
                continue;
            };
            let instance = entry.device.object_identifier.instance_number();
            self.claims.remove(&(instance, key));
            if self.claims_of(instance).next().is_some() && !purged.contested.contains(&instance) {
                purged.contested.push(instance);
            }
            purged.lost.push(entry.device);
        }
        purged
    }
}

#[cfg(test)]
#[path = "discovery_collision_tests.rs"]
mod discovery_collision_tests;

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
