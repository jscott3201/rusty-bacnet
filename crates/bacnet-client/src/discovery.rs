//! Device discovery table — collects IAm responses for WhoIs/WhoHas lookups.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use bacnet_types::enums::Segmentation;
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;

/// Information about a discovered BACnet device.
#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    /// The device's object identifier (always ObjectType::DEVICE).
    pub object_identifier: ObjectIdentifier,
    /// The MAC address from which the IAm was received.
    pub mac_address: MacAddr,
    /// Maximum APDU length the device accepts.
    pub max_apdu_length: u32,
    /// Segmentation support level.
    pub segmentation_supported: Segmentation,
    /// Maximum segments the remote device accepts (None = unlimited/unspecified).
    pub max_segments_accepted: Option<u32>,
    /// Vendor identifier.
    pub vendor_id: u16,
    /// When this entry was last updated.
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

/// Thread-safe device discovery table.
///
/// Keyed by device instance number (the instance part of the DEVICE object
/// identifier). Ordinary I-Am refreshes update a row, while a conflicting
/// complete endpoint claim leaves the first authoritative row unchanged.
///
/// # Address-space invariant
///
/// A row is *routed* when it carries routed-source identity (`source_network`
/// and `source_address`, per Clause 6.2.2 the original source SNET/SADR); its
/// `mac_address` is then only the immediate-hop router, shared by every peer
/// behind that router. A row is *local* only when it is unambiguously local:
/// both routing fields are `None`. A row with exactly one routing field set is
/// malformed; it counts as neither local nor routable and matches no secondary
/// lookup until a complete I-Am refreshes it.
///
/// # Capability provenance
///
/// Each row records whether its `segmentation_supported` is authoritative
/// (I-Am ingestion via `upsert_with_result`, explicit
/// [`upsert`][DeviceTable::upsert], or `add_routed_device`) or a legacy
/// placeholder (`add_device`). Provenance lives in the same table entry as
/// the device data, so it stays coherent across insert, update, purge, and
/// clear: any replacement of an instance replaces its provenance with it.
#[derive(Debug, Default)]
pub struct DeviceTable {
    devices: HashMap<u32, TableEntry>,
}

#[derive(Debug, Clone)]
pub(crate) enum DeviceUpsertResult {
    Inserted,
    Updated,
    Collision { retained: DiscoveredDevice },
    Dropped,
}

enum DeviceEndpoint<'a> {
    Local(&'a [u8]),
    Routed(u16, &'a [u8]),
    Incomplete,
}

impl DeviceTable {
    pub fn new() -> Self {
        Self {
            devices: HashMap::new(),
        }
    }

    /// Insert or update a discovered device.
    ///
    /// The row's `segmentation_supported` is treated as authoritative
    /// capability (explicit configuration); see [`PeerSegmentation`].
    /// This explicit administrative path replaces any row with the same
    /// instance regardless of endpoint identity.
    ///
    /// The table is capped at 4096 entries. If the table is full and the
    /// device is not already present, the new entry is silently dropped.
    pub fn upsert(&mut self, device: DiscoveredDevice) {
        let segmentation = PeerSegmentation::Authoritative(device.segmentation_supported);
        let _ = self.insert_entry(device, segmentation);
    }

    pub(crate) fn upsert_with_result(&mut self, device: DiscoveredDevice) -> DeviceUpsertResult {
        let key = device.object_identifier.instance_number();
        if let Some(existing) = self.devices.get(&key) {
            let replace = existing.segmentation == PeerSegmentation::Placeholder
                || match (Self::endpoint(&existing.device), Self::endpoint(&device)) {
                    // A malformed retained row has no stable endpoint identity;
                    // accepting the next I-Am keeps it repairable.
                    (DeviceEndpoint::Incomplete, _) => true,
                    // A malformed incoming row cannot displace stable routing.
                    (_, DeviceEndpoint::Incomplete) => false,
                    (DeviceEndpoint::Local(current), DeviceEndpoint::Local(incoming)) => {
                        current == incoming
                    }
                    (
                        DeviceEndpoint::Routed(current_network, current_address),
                        DeviceEndpoint::Routed(incoming_network, incoming_address),
                    ) => current_network == incoming_network && current_address == incoming_address,
                    _ => false,
                };
            if !replace {
                return DeviceUpsertResult::Collision {
                    retained: existing.device.clone(),
                };
            }
        }

        let segmentation = PeerSegmentation::Authoritative(device.segmentation_supported);
        self.insert_entry(device, segmentation)
    }

    fn endpoint(device: &DiscoveredDevice) -> DeviceEndpoint<'_> {
        match (&device.source_network, &device.source_address) {
            (None, None) => DeviceEndpoint::Local(device.mac_address.as_slice()),
            (Some(network), Some(address)) => DeviceEndpoint::Routed(*network, address.as_slice()),
            _ => DeviceEndpoint::Incomplete,
        }
    }

    /// Insert or update a device whose capability fields are manual
    /// placeholders rather than learned or explicitly supplied values
    /// (legacy `add_device`). Its `segmentation_supported` must never drive
    /// a capability decision (#371).
    pub(crate) fn upsert_placeholder(&mut self, device: DiscoveredDevice) {
        let _ = self.insert_entry(device, PeerSegmentation::Placeholder);
    }

    fn insert_entry(
        &mut self,
        device: DiscoveredDevice,
        segmentation: PeerSegmentation,
    ) -> DeviceUpsertResult {
        const MAX_DEVICE_TABLE_ENTRIES: usize = 4096;
        let key = device.object_identifier.instance_number();
        let is_existing = self.devices.contains_key(&key);
        if !is_existing && self.devices.len() >= MAX_DEVICE_TABLE_ENTRIES {
            return DeviceUpsertResult::Dropped;
        }
        self.devices.insert(
            key,
            TableEntry {
                device,
                segmentation,
            },
        );
        if is_existing {
            DeviceUpsertResult::Updated
        } else {
            DeviceUpsertResult::Inserted
        }
    }

    /// Get all discovered devices as a snapshot.
    pub fn all(&self) -> Vec<DiscoveredDevice> {
        self.devices.values().map(|e| e.device.clone()).collect()
    }

    /// Look up a device by instance number.
    pub fn get(&self, instance: u32) -> Option<&DiscoveredDevice> {
        self.devices.get(&instance).map(|e| &e.device)
    }

    /// Look up a local device by its MAC address.
    ///
    /// Only unambiguously local rows (no `source_network`/`source_address`)
    /// are considered: a routed row's `mac_address` is the router it was
    /// heard through, not the remote device's identity, so a lookup of the
    /// router's own address must never select a peer behind it (Clause 6.2.2).
    ///
    /// The table is keyed by device instance, so two local rows can share one
    /// MAC until the stale purge; the freshest `last_seen` wins, mirroring
    /// [`DeviceTable::get_by_network_address`].
    pub fn get_by_mac(&self, mac: &[u8]) -> Option<&DiscoveredDevice> {
        self.devices
            .values()
            .filter(|e| e.device.is_local())
            .filter(|e| e.device.mac_address.as_slice() == mac)
            .max_by_key(|e| e.device.last_seen)
            .map(|e| &e.device)
    }

    /// Look up a routed device by its remote network number and MAC address.
    ///
    /// A routed device's `mac_address` holds the router it was heard through,
    /// which every device behind that router shares; its own identity is the
    /// SNET/SADR of the NPDU that carried its I-Am, stored as
    /// `source_network` and `source_address`.
    ///
    /// The table is keyed by device instance, so two rows can share one
    /// SNET/SADR (a re-commissioned instance survives until the stale purge).
    /// The freshest `last_seen` wins: it holds what the device most recently
    /// advertised.
    pub fn get_by_network_address(
        &self,
        network: u16,
        address: &[u8],
    ) -> Option<&DiscoveredDevice> {
        self.devices
            .values()
            .filter(|e| {
                e.device.source_network == Some(network)
                    && e.device
                        .source_address
                        .as_ref()
                        .is_some_and(|a| a.as_slice() == address)
            })
            .max_by_key(|e| e.device.last_seen)
            .map(|e| &e.device)
    }

    /// Segmentation capability knowledge for the local row [`get_by_mac`]
    /// selects, or `None` when no local row matches. Coherent with request
    /// sizing: both consult the same row under one shared borrow.
    pub(crate) fn local_peer_segmentation(&self, mac: &[u8]) -> Option<PeerSegmentation> {
        let device = self.get_by_mac(mac)?;
        self.devices
            .get(&device.object_identifier.instance_number())
            .map(|e| e.segmentation)
    }

    /// Segmentation capability knowledge for the routed row
    /// [`get_by_network_address`] selects, or `None` when no row matches.
    pub(crate) fn routed_peer_segmentation(
        &self,
        network: u16,
        address: &[u8],
    ) -> Option<PeerSegmentation> {
        let device = self.get_by_network_address(network, address)?;
        self.devices
            .get(&device.object_identifier.instance_number())
            .map(|e| e.segmentation)
    }

    /// Clear all entries.
    pub fn clear(&mut self) {
        self.devices.clear();
    }

    /// Number of discovered devices.
    pub fn len(&self) -> usize {
        self.devices.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    /// Remove entries whose `last_seen` is older than `max_age`.
    pub fn purge_stale(&mut self, max_age: Duration) {
        let _ = self.purge_stale_at(Instant::now(), max_age);
    }

    pub(crate) fn purge_stale_collect(&mut self, max_age: Duration) -> Vec<DiscoveredDevice> {
        self.purge_stale_at(Instant::now(), max_age)
    }

    fn purge_stale_at(&mut self, now: Instant, max_age: Duration) -> Vec<DiscoveredDevice> {
        let stale_keys: Vec<u32> = self
            .devices
            .iter()
            .filter_map(|(key, entry)| {
                let is_stale = now
                    .checked_duration_since(entry.device.last_seen)
                    .is_some_and(|age| age > max_age);
                is_stale.then_some(*key)
            })
            .collect();

        stale_keys
            .into_iter()
            .filter_map(|key| self.devices.remove(&key).map(|e| e.device))
            .collect()
    }
}

#[cfg(test)]
#[path = "discovery_collision_tests.rs"]
mod discovery_collision_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    fn make_device(instance: u32) -> DiscoveredDevice {
        DiscoveredDevice {
            object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
            mac_address: MacAddr::from_slice(&[192, 168, 1, 100, 0xBA, 0xC0]),
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NONE,
            max_segments_accepted: None,
            vendor_id: 42,
            last_seen: Instant::now(),
            source_network: None,
            source_address: None,
        }
    }

    #[test]
    fn upsert_and_get() {
        let mut table = DeviceTable::new();
        table.upsert(make_device(1234));
        assert_eq!(table.len(), 1);
        let dev = table.get(1234).unwrap();
        assert_eq!(dev.vendor_id, 42);
    }

    #[test]
    fn upsert_updates_existing() {
        let mut table = DeviceTable::new();
        table.upsert(make_device(1234));
        let mut updated = make_device(1234);
        updated.vendor_id = 99;
        table.upsert(updated);
        assert_eq!(table.len(), 1);
        assert_eq!(table.get(1234).unwrap().vendor_id, 99);
    }

    #[test]
    fn upsert_with_result_reports_insert_and_update() {
        let mut table = DeviceTable::new();
        assert!(matches!(
            table.upsert_with_result(make_device(1234)),
            DeviceUpsertResult::Inserted
        ));
        assert!(matches!(
            table.upsert_with_result(make_device(1234)),
            DeviceUpsertResult::Updated
        ));
    }

    #[test]
    fn all_returns_snapshot() {
        let mut table = DeviceTable::new();
        table.upsert(make_device(1));
        table.upsert(make_device(2));
        table.upsert(make_device(3));
        assert_eq!(table.all().len(), 3);
    }

    #[test]
    fn clear_empties_table() {
        let mut table = DeviceTable::new();
        table.upsert(make_device(1));
        table.clear();
        assert!(table.is_empty());
    }

    #[test]
    fn get_by_mac_finds_device() {
        let mut table = DeviceTable::new();
        table.upsert(make_device(1234));
        let mac = &[192, 168, 1, 100, 0xBA, 0xC0];
        let dev = table.get_by_mac(mac).unwrap();
        assert_eq!(dev.object_identifier.instance_number(), 1234);
    }

    #[test]
    fn get_by_mac_not_found() {
        let mut table = DeviceTable::new();
        table.upsert(make_device(1234));
        assert!(table.get_by_mac(&[10, 0, 0, 1, 0xBA, 0xC0]).is_none());
    }

    fn make_routed_device(instance: u32, network: u16, address: &[u8]) -> DiscoveredDevice {
        let mut device = make_device(instance);
        device.source_network = Some(network);
        device.source_address = Some(MacAddr::from_slice(address));
        device
    }

    #[test]
    fn get_by_network_address_finds_routed_device() {
        let mut table = DeviceTable::new();
        table.upsert(make_routed_device(1, 100, &[0x03]));
        table.upsert(make_routed_device(2, 200, &[0x03]));
        let dev = table.get_by_network_address(200, &[0x03]).unwrap();
        assert_eq!(dev.object_identifier.instance_number(), 2);
    }

    /// A local device whose MAC happens to equal the queried remote address
    /// lives in a different address space and must not match.
    #[test]
    fn get_by_network_address_ignores_local_devices() {
        let mut table = DeviceTable::new();
        let mut local = make_device(1);
        local.mac_address = MacAddr::from_slice(&[0x03]);
        table.upsert(local);
        assert!(table.get_by_network_address(100, &[0x03]).is_none());
    }

    #[test]
    fn get_by_network_address_requires_both_terms() {
        let mut table = DeviceTable::new();
        table.upsert(make_routed_device(1, 100, &[0x03]));
        assert!(table.get_by_network_address(100, &[0x04]).is_none());
        assert!(table.get_by_network_address(101, &[0x03]).is_none());
    }

    /// A re-commissioned device instance leaves two rows at one SNET/SADR
    /// until the stale purge; the freshest advertisement must win, not an
    /// arbitrary hash-order pick.
    #[test]
    fn get_by_network_address_prefers_freshest_entry() {
        let mut table = DeviceTable::new();
        let now = Instant::now();
        let mut stale = make_routed_device(1, 100, &[0x03]);
        stale.max_apdu_length = 1476;
        stale.last_seen = now;
        let mut fresh = make_routed_device(2, 100, &[0x03]);
        fresh.max_apdu_length = 128;
        fresh.last_seen = now + Duration::from_secs(60);
        table.upsert(stale);
        table.upsert(fresh);

        let dev = table.get_by_network_address(100, &[0x03]).unwrap();
        assert_eq!(dev.object_identifier.instance_number(), 2);
        assert_eq!(dev.max_apdu_length, 128);
    }

    /// A routed row's `mac_address` is the router it was heard through, not
    /// the remote device's identity (Clause 6.2.2: SNET/SADR identify the
    /// original source; the router MAC is only the local next hop). A local
    /// MAC lookup for the router's own address must therefore never select a
    /// routed peer behind that router (#372).
    /// Provenance transitions (#371): a legacy placeholder row becomes
    /// authoritative when refreshed through the I-Am insertion path, and
    /// provenance never outlives its row across purge/clear.
    #[test]
    fn segmentation_provenance_tracks_row_lifecycle() {
        let mut table = DeviceTable::new();
        table.upsert_placeholder(make_device(1));
        assert_eq!(
            table.local_peer_segmentation(&[192, 168, 1, 100, 0xBA, 0xC0]),
            Some(PeerSegmentation::Placeholder)
        );

        // I-Am refresh of the same instance (the dispatch path).
        table.upsert_with_result(make_device(1));
        assert_eq!(
            table.local_peer_segmentation(&[192, 168, 1, 100, 0xBA, 0xC0]),
            Some(PeerSegmentation::Authoritative(Segmentation::NONE))
        );

        // Purge removes the row and its provenance with it.
        let mut stale = make_device(1);
        stale.last_seen = Instant::now() - Duration::from_secs(120);
        table.upsert_placeholder(stale);
        table.purge_stale(Duration::from_secs(60));
        assert_eq!(
            table.local_peer_segmentation(&[192, 168, 1, 100, 0xBA, 0xC0]),
            None
        );

        // Clear leaves no orphan provenance either.
        table.upsert(make_device(2));
        table.clear();
        assert_eq!(
            table.local_peer_segmentation(&[192, 168, 1, 100, 0xBA, 0xC0]),
            None
        );
    }

    #[test]
    fn get_by_mac_ignores_routed_rows() {
        let mut table = DeviceTable::new();
        let router_mac = &[192, 168, 1, 1, 0xBA, 0xC0];
        let routed = make_routed_device(3003, 100, &[0x03]);
        let mut routed_with_router_mac = routed;
        routed_with_router_mac.mac_address = MacAddr::from_slice(router_mac);
        table.upsert(routed_with_router_mac);

        assert!(table.get_by_mac(router_mac).is_none());
    }

    /// Several routed peers can share one router MAC; none of them may be
    /// returned by a local lookup of that router's address.
    #[test]
    fn get_by_mac_ignores_multiple_routed_rows_behind_one_router() {
        let mut table = DeviceTable::new();
        let router_mac = &[192, 168, 1, 1, 0xBA, 0xC0];
        for instance in [1u32, 2, 3] {
            let mut peer = make_routed_device(instance, 100, &[instance as u8]);
            peer.mac_address = MacAddr::from_slice(router_mac);
            table.upsert(peer);
        }
        assert!(table.get_by_mac(router_mac).is_none());
    }

    fn make_local_device_at_mac(instance: u32, mac: &[u8]) -> DiscoveredDevice {
        let mut device = make_device(instance);
        device.mac_address = MacAddr::from_slice(mac);
        device
    }

    /// A local device whose MAC byte-equals a routed peer's SADR lives in a
    /// different address space; the local lookup must return the local row.
    #[test]
    fn get_by_mac_returns_local_row_not_routed_sadr_namesake() {
        let mut table = DeviceTable::new();
        table.upsert(make_local_device_at_mac(10, &[0x03]));
        table.upsert(make_routed_device(3003, 100, &[0x03]));

        let dev = table.get_by_mac(&[0x03]).unwrap();
        assert_eq!(dev.object_identifier.instance_number(), 10);
    }

    /// Two local rows sharing one MAC (re-commissioned instance surviving
    /// until the stale purge) must resolve deterministically to the freshest
    /// advertisement, mirroring the routed lookup rule.
    #[test]
    fn get_by_mac_prefers_freshest_local_row() {
        let mut table = DeviceTable::new();
        let now = Instant::now();
        let mut stale = make_device(1);
        stale.last_seen = now;
        let mut fresh = make_device(2);
        fresh.last_seen = now + Duration::from_secs(60);
        table.upsert(stale);
        table.upsert(fresh);

        let dev = table.get_by_mac(&[192, 168, 1, 100, 0xBA, 0xC0]).unwrap();
        assert_eq!(dev.object_identifier.instance_number(), 2);
    }

    /// Partial routing metadata (one half of SNET/SADR set) is malformed:
    /// such a row is not unambiguously local, so it is excluded from local
    /// lookup; the routed lookup already requires both terms.
    #[test]
    fn partial_routing_metadata_is_excluded_from_local_lookup() {
        let mut table = DeviceTable::new();
        let mut partial = make_device(7);
        partial.source_network = Some(100);
        table.upsert(partial);

        assert!(table.get_by_mac(&[192, 168, 1, 100, 0xBA, 0xC0]).is_none());
        assert!(table
            .get_by_network_address(100, &[192, 168, 1, 100, 0xBA, 0xC0])
            .is_none());
    }

    #[test]
    fn purge_stale_removes_old_entries() {
        let mut table = DeviceTable::new();
        let now = Instant::now();
        let mut old_device = make_device(1);
        old_device.last_seen = now;
        table.upsert(old_device);
        let mut fresh_device = make_device(2);
        fresh_device.last_seen = now + Duration::from_secs(120);
        table.upsert(fresh_device);
        assert_eq!(table.len(), 2);

        let removed = table.purge_stale_at(now + Duration::from_secs(120), Duration::from_secs(60));
        assert_eq!(table.len(), 1);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].object_identifier.instance_number(), 1);
        assert!(table.get(1).is_none());
        assert!(table.get(2).is_some());
    }

    #[test]
    fn purge_stale_keeps_all_when_fresh() {
        let mut table = DeviceTable::new();
        table.upsert(make_device(1));
        table.upsert(make_device(2));
        table.purge_stale(Duration::from_secs(60));
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn purge_stale_removes_all_when_expired() {
        let mut table = DeviceTable::new();
        let now = Instant::now();
        let mut d1 = make_device(1);
        d1.last_seen = now;
        let mut d2 = make_device(2);
        d2.last_seen = now;
        table.upsert(d1);
        table.upsert(d2);
        let removed = table.purge_stale_at(now + Duration::from_secs(200), Duration::from_secs(60));
        assert!(table.is_empty());
        assert_eq!(removed.len(), 2);
    }

    #[test]
    fn upsert_refreshes_last_seen() {
        let mut table = DeviceTable::new();
        let now = Instant::now();
        let mut old_device = make_device(1);
        old_device.last_seen = now;
        table.upsert(old_device);

        let mut refreshed = make_device(1);
        refreshed.last_seen = now + Duration::from_secs(120);
        table.upsert(refreshed);
        let removed = table.purge_stale_at(now + Duration::from_secs(120), Duration::from_secs(60));
        assert_eq!(table.len(), 1);
        assert!(removed.is_empty());
        assert!(table.get(1).is_some());
    }

    #[test]
    fn purge_stale_handles_max_age_larger_than_instant_history() {
        let mut table = DeviceTable::new();
        table.upsert(make_device(1));

        table.purge_stale(Duration::MAX);

        assert_eq!(table.len(), 1);
    }
}
