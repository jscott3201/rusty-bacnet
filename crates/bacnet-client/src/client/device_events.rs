use super::*;
use crate::discovery::Folded;
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

/// What an I-Am taken in calls for beyond the table.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct IAmWants {
    /// It stored a routed row while this client's network number is
    /// unknown, which wants the number asked for (#2572).
    pub(super) number: bool,
    /// It named a routed row through another station than the row's router,
    /// which wants that path checked (#2823).
    pub(super) route_check: bool,
}

/// Take an I-Am into the discovery table and tell the subscribers what it
/// changed. The notices go out under the table's lock, as the purge's do (a
/// broadcast send never waits), so the two can't interleave out of order.
/// Returns what it wants done beyond the table. Kept out of line: the
/// dispatch loop's hot path is the replies.
#[inline(never)]
pub(super) fn heard_i_am(
    table: &SyncMutex<DeviceTable>,
    from: IAmSource<'_>,
    i_am: &bacnet_services::who_is::IAmRequest,
    (device_tx, collision_tx): TableNotices<'_>,
) -> IAmWants {
    let mut table = table.lock();
    // A router relaying back an I-Am from this client's own network names a
    // local device, as a request relayed that way does (#1404): one row, as
    // it is one peer (#1358). The number is
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
    // A relay naming this network lands on a local row by the number alone.
    let relayed_here = source_network.is_none()
        && from
            .network
            .as_ref()
            .is_some_and(|routed| !routed.mac_address.is_empty());
    let heard = if relayed_here {
        table.heard_by_number(device.clone())
    } else {
        table.heard(device.clone())
    };
    if heard.change == Change::Held {
        // The row keeps its router and everything else: the frame is a cue
        // for the client's own check, never a change (#2823).
        debug!(
            device = i_am.object_identifier.instance_number(),
            "a routed I-Am came through another station than its row's router; checking the path"
        );
        return IAmWants {
            number: false,
            route_check: true,
        };
    }
    let routed_while_unknown =
        this_network.is_none() && source_network.is_some() && heard.change != Change::Dropped;
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
        Change::Dropped | Change::Held => None,
    };
    if let Some(kind) = kind {
        let _ = device_tx.send(DeviceEvent { kind, device });
    }
    // One notice per instance, each with the claimants left at the end.
    for instance in heard.contested {
        let _ = collision_tx.send(DeviceCollisionEvent::of(&table, instance));
    }
    IAmWants {
        number: routed_while_unknown,
        route_check: false,
    }
}

/// A request or notification that came from a routed device through
/// `source_mac`: when that is the router the device's row names, the row's
/// age starts again, as an answer to a request starts it (#2823), so a
/// device heard only through COV or event notifications is not aged out
/// and opened to first sight again. Another station can't send from the
/// router's link address, so it can't keep a row young this way, except on
/// B/IP with a forwarded frame whose origin a BBMD gives as the router's
/// (#2829); that can only keep a path, never move one. Kept out of line:
/// the dispatch loop's hot path is the replies.
#[inline(never)]
pub(super) fn heard_routed(
    table: &SyncMutex<DeviceTable>,
    source_mac: &[u8],
    source_network: &Option<NpduAddress>,
) {
    if let Some(routed) = source_network
        && !routed.mac_address.is_empty()
    {
        table
            .lock()
            .heard_through(routed.network, &routed.mac_address, source_mac);
    }
}

/// Learn this client's network number `number`, and fold the discovery
/// table to it in the same locked call (#2572). When it replaces a number
/// known before, the rows local only by that number are dropped first,
/// each reported `Lost`. Then every routed row on the new network names a
/// local station: it is taken out where the station has a local row
/// (`Lost`), or becomes that row (`Lost`, then `Discovered`, or `Updated`
/// over an `add_device` placeholder for the same instance). Then one
/// collision notice per instance whose claimants changed, as the I-Am path
/// and the purge report theirs. The number is published under the lock, so
/// an I-Am taken in after this sees it, and one taken in before has made
/// the routed row this folds.
pub(super) fn learned_number(
    table: &SyncMutex<DeviceTable>,
    this_network: &LocalNetworkNumber,
    number: NetworkNumber,
    (device_tx, collision_tx): TableNotices<'_>,
) {
    let mut table = table.lock();
    let previous = this_network.get();
    this_network.publish(number);
    fold_to_number(
        &mut table,
        previous.is_some(),
        number.snapshot().0,
        (device_tx, collision_tx),
    );
}

/// Fold `table` to this network's number `number`, which was just learned:
/// [`learned_number`]'s work on the table. `relearned` says a number was
/// known before, whose local rows go first.
pub(super) fn fold_to_number(
    table: &mut DeviceTable,
    relearned: bool,
    number: u16,
    (device_tx, collision_tx): TableNotices<'_>,
) {
    let lost = |device| {
        let _ = device_tx.send(DeviceEvent {
            kind: DeviceEventKind::Lost,
            device,
        });
    };
    let mut contested = Vec::new();
    if relearned {
        let dropped = table.unlocalize();
        dropped.lost.into_iter().for_each(lost);
        contested = dropped.contested;
    }
    let (folded, more) = table.localize(number);
    for Folded {
        routed,
        local,
        placeholder,
    } in folded
    {
        lost(routed);
        let Some(local) = local else {
            continue;
        };
        let kind = match placeholder {
            Some(p) if p.object_identifier == local.object_identifier => DeviceEventKind::Updated,
            Some(p) => {
                lost(p);
                DeviceEventKind::Discovered
            }
            None => DeviceEventKind::Discovered,
        };
        let _ = device_tx.send(DeviceEvent {
            kind,
            device: local,
        });
    }
    for instance in more {
        if !contested.contains(&instance) {
            contested.push(instance);
        }
    }
    for instance in contested {
        let _ = collision_tx.send(DeviceCollisionEvent::of(table, instance));
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
