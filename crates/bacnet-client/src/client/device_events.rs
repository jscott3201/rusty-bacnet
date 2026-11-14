use super::*;
use bacnet_network::network_number::LocalNetworkNumber;
use bacnet_types::network_number::NetworkNumber;

/// Where an I-Am came from: the link MAC, the SNET/SADR a router added, and
/// this client's network number, read under the table's lock.
pub(super) struct IAmSource<'a> {
    pub(super) mac: &'a [u8],
    pub(super) network: &'a Option<NpduAddress>,
    pub(super) this_network: &'a LocalNetworkNumber,
}

/// The discovery table's subscribers: device notices, then collision
/// notices.
pub(super) type TableNotices<'a> = (
    &'a broadcast::Sender<DeviceEvent>,
    &'a broadcast::Sender<DeviceCollisionEvent>,
);

/// Take an I-Am into the discovery table and tell the subscribers what it
/// changed. The notices go out under the table's lock, as the purge's do (a
/// broadcast send never waits), so the two can't interleave out of order.
/// Kept out of line: the dispatch loop's hot path is the replies.
#[inline(never)]
pub(super) fn heard_i_am(
    table: &SyncMutex<DeviceTable>,
    from: IAmSource<'_>,
    i_am: &bacnet_services::who_is::IAmRequest,
    (device_tx, collision_tx): TableNotices<'_>,
) {
    let mut table = table.lock();
    // A router relaying back an I-Am from this client's own network names a
    // local device: one row, as it is one peer (#1358, #1465). The number is
    // read under the lock its learning folds the table under, so a routed
    // row made before then is folded by that, and none is made after it
    // (#2572). While the number is unknown the relay is a routed station.
    let this_network = from.this_network.get();
    let (mac_address, source_network, source_address) = match from.network {
        Some(routed) if !routed.mac_address.is_empty() => {
            if Some(routed.network) == this_network {
                (routed.mac_address.clone(), None, None)
            } else {
                let station = Some(routed.mac_address.clone());
                (MacAddr::from_slice(from.mac), Some(routed.network), station)
            }
        }
        _ => (MacAddr::from_slice(from.mac), None, None),
    };
    let now = crate::discovery::now();
    let device = DiscoveredDevice {
        object_identifier: i_am.object_identifier,
        mac_address,
        max_apdu_length: i_am.max_apdu_length,
        segmentation_supported: i_am.segmentation_supported,
        max_segments_accepted: None,
        vendor_id: i_am.vendor_id,
        last_seen: now,
        last_announced: Some(now),
        source_network,
        source_address,
    };
    let heard = table.heard(device.clone());
    // The device that left the address, then the one now there.
    if let Some(old) = heard.replaced {
        let _ = device_tx.send(DeviceEvent {
            kind: DeviceEventKind::Lost,
            device: old,
        });
    }
    let kind = match heard.change {
        Change::Inserted | Change::Replaced => Some(DeviceEventKind::Discovered),
        Change::Updated => Some(DeviceEventKind::Updated),
        Change::Dropped => None,
    };
    if let Some(kind) = kind {
        let _ = device_tx.send(DeviceEvent { kind, device });
    }
    // One notice per instance, each with the claimants left at the end.
    for instance in heard.contested {
        let _ = collision_tx.send(DeviceCollisionEvent::of(&table, instance));
    }
}

/// Learn this client's network number `number`, and fold the discovery
/// table to it in the same locked call (#2572): every routed row on that
/// network names a local station, so it is taken out where the station has
/// a local row, or becomes that row. Each routed row taken out is reported
/// `Lost`, the local row it became `Discovered`, then one collision notice
/// per instance whose claimants changed, as the I-Am path and the purge
/// report theirs. The number is published under the lock, so an I-Am taken
/// in after this sees it, and one taken in before has made the routed row
/// this folds.
pub(super) fn learned_number(
    table: &SyncMutex<DeviceTable>,
    this_network: &LocalNetworkNumber,
    number: NetworkNumber,
    (device_tx, collision_tx): TableNotices<'_>,
) {
    let mut table = table.lock();
    this_network.publish(number);
    let localized = table.localize(number.snapshot().0);
    for (routed, local) in localized.rows {
        let _ = device_tx.send(DeviceEvent {
            kind: DeviceEventKind::Lost,
            device: routed,
        });
        if let Some(device) = local {
            let _ = device_tx.send(DeviceEvent {
                kind: DeviceEventKind::Discovered,
                device,
            });
        }
    }
    for instance in localized.contested {
        let _ = collision_tx.send(DeviceCollisionEvent::of(&table, instance));
    }
}

impl DeviceCollisionEvent {
    /// The rows that claim `instance` in `table` now.
    pub(super) fn of(table: &DeviceTable, instance: u32) -> Self {
        Self {
            instance,
            claimants: table.claiming(instance).cloned().collect(),
        }
    }

    /// True when at most one address claims the instance now, so the
    /// contest has ended and a lookup by the instance works again.
    pub fn is_resolved(&self) -> bool {
        self.claimants.len() < 2
    }
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Get a receiver for device discovery events. Each call returns a new
    /// independent receiver.
    ///
    /// Events are notification-only; `discovered_devices()` remains the
    /// authoritative snapshot of the current discovery table.
    pub fn device_events(&self) -> broadcast::Receiver<DeviceEvent> {
        self.device_tx.subscribe()
    }

    /// Get a receiver for Device-instance collision notifications. Each call
    /// returns a new independent receiver.
    ///
    /// Delivery is notification-only and best-effort: receivers can lag and
    /// missed notifications do not affect collision handling. Each notice
    /// lists the rows that claim the instance after a change, and the table
    /// holds every one of them; `discovered_devices()` remains the
    /// authoritative snapshot.
    pub fn device_collision_events(&self) -> broadcast::Receiver<DeviceCollisionEvent> {
        self.device_collision_tx.subscribe()
    }
}
