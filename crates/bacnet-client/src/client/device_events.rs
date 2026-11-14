use super::*;

/// Where an I-Am came from: the link MAC, the SNET/SADR a router added, and
/// the number of this client's network, when it has learned one.
pub(super) struct IAmSource<'a> {
    pub(super) mac: &'a [u8],
    pub(super) network: &'a Option<NpduAddress>,
    pub(super) this_network: Option<u16>,
}

/// Take an I-Am into the discovery table and tell the subscribers what it
/// changed. The notices go out under the table's lock, as the purge's do (a
/// broadcast send never waits), so the two can't interleave out of order.
/// Kept out of line: the dispatch loop's hot path is the replies.
#[inline(never)]
pub(super) fn heard_i_am(
    table: &SyncMutex<DeviceTable>,
    from: IAmSource<'_>,
    i_am: &bacnet_services::who_is::IAmRequest,
    (device_tx, collision_tx): (
        &broadcast::Sender<DeviceEvent>,
        &broadcast::Sender<DeviceCollisionEvent>,
    ),
) {
    // A router relaying back an I-Am from this client's own network names a
    // local device: one row, as it is one peer (#1358, #1465). A relay heard
    // before the client learned the number made a routed row for the same
    // peer, which this one folds into the local row. While the number stays
    // unknown, the routed row stays until the purge takes it.
    let (mac_address, source_network, source_address, fold) = match from.network {
        Some(routed) if !routed.mac_address.is_empty() => {
            if Some(routed.network) == from.this_network {
                (routed.mac_address.clone(), None, None, Some(routed.network))
            } else {
                let station = Some(routed.mac_address.clone());
                (
                    MacAddr::from_slice(from.mac),
                    Some(routed.network),
                    station,
                    None,
                )
            }
        }
        _ => (MacAddr::from_slice(from.mac), None, None, None),
    };
    let device = DiscoveredDevice {
        object_identifier: i_am.object_identifier,
        mac_address,
        max_apdu_length: i_am.max_apdu_length,
        segmentation_supported: i_am.segmentation_supported,
        max_segments_accepted: None,
        vendor_id: i_am.vendor_id,
        last_seen: crate::discovery::now(),
        source_network,
        source_address,
    };
    let mut table = table.lock();
    // Folded first, so it frees its place in a full table.
    let folded = fold.and_then(|network| table.remove(network, &device.mac_address));
    let heard = table.heard(device.clone());
    let lost = folded.as_ref().map(|f| &f.device).into_iter();
    // The routed row folded in, then the device that left the address.
    for old in lost.chain(heard.replaced.as_ref()) {
        let _ = device_tx.send(DeviceEvent {
            kind: DeviceEventKind::Lost,
            device: old.clone(),
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
    let shrank = folded
        .and_then(|f| f.contested)
        .filter(|instance| !heard.contested.contains(instance));
    for instance in shrank.into_iter().chain(heard.contested) {
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
