//! Following points (#2795): a point that follows its device's instance
//! ([`Target::Follows`](super::super::Target::Follows)) goes to the address
//! the hub's own reads found the device at ([`moves`](super::moves)),
//! keeping its subscriber process identifier, its values and its
//! listeners. Nothing else moves one: an I-Am, a handle moving, or the
//! discovery table only cue those reads.
//!
//! The scheduler that took the move hands each such row to the scheduler
//! at the new address, under the table's lock, so the point's address
//! (`at`, which the lease's release and the notification receiver look its
//! scheduler up by) and the scheduler holding it always agree. A row whose
//! last lease went meanwhile, or of a hub shutting down, stops here
//! instead. What the row held here is dropped: its managed subscription
//! stops renewing, and nothing is cancelled here, as the device here is
//! another one or silent by the move's own answers (#2801: no cancel to a
//! reused address); that context lapses with its lifetime. The device at
//! the new address is read and subscribed with the same subscriber process
//! identifier, which a device that kept its table through the move takes as
//! the same subscription.
//!
//! The hub follows each instance from one place ([`Follow`]): its
//! following points start where it follows the device now, and a move it
//! took moves the client's handles following the instance too, telling
//! [`Client::events`](crate::Client::events).
//!
//! [`Follow`]: super::super::shared::Follow

use std::sync::Arc;

use bacnet_transport::port::TransportPort;

use super::super::shared::{Arrival, DeviceSlot};
use super::{Device, Mode, Row};
use crate::device::{DeviceAddress, FollowKey, Followed};

impl<T: TransportPort + 'static> Device<T> {
    /// The hub follows `keys` at `to` now, not at this address: their next
    /// points start there, and the client's handles following them move
    /// there.
    pub(super) fn follow_moved(&self, keys: Vec<FollowKey>, to: &DeviceAddress) {
        let from = &self.address;
        let places: Vec<(FollowKey, Followed)> = {
            let mut table = self.inner.registry.table();
            keys.into_iter()
                .filter_map(|key| {
                    let follow = table.follows.get_mut(&key)?;
                    if follow.at != *from {
                        return None;
                    }
                    follow.at = to.clone();
                    Some((key, Arc::clone(&follow.place)))
                })
                .collect()
        };
        for (key, place) in places {
            self.inner.client.move_to(key, &place, from, to.clone());
        }
    }

    /// Hand the following points `processes` to the scheduler at `to`,
    /// started there with it if it has none.
    pub(super) fn hand_over(&mut self, processes: Vec<u32>, to: &DeviceAddress) {
        if processes.is_empty() {
            return;
        }
        let inner = Arc::clone(&self.inner);
        let registry = &inner.registry;
        let mut done = Vec::new();
        let mut fresh = None;
        {
            let mut table = registry.table();
            let mut target: Option<Arc<DeviceSlot>> = None;
            for process in processes {
                let Some(row) = self.rows.remove(&process) else {
                    continue;
                };
                let held = table
                    .entries
                    .get(&row.shared.point)
                    .is_some_and(|slot| Arc::ptr_eq(&slot.shared, &row.shared));
                if table.shut || !held || row.removing {
                    done.push(row);
                    continue;
                }
                let Row {
                    shared,
                    running,
                    mode,
                    ..
                } = row;
                // A subscription held here stops renewing; nothing is
                // cancelled here.
                drop(mode);
                shared.handed(&registry.seq, &self.address, to);
                let slot = target.get_or_insert_with(|| {
                    Arc::clone(table.devices.entry(to.clone()).or_insert_with(|| {
                        let slot = Arc::new(DeviceSlot::new());
                        fresh = Some(Arc::clone(&slot));
                        slot
                    }))
                });
                slot.inbox().push(Arrival { shared, running });
            }
            if let Some(slot) = target {
                slot.wake.notify_one();
            }
        }
        // Let go: nothing is cancelled at the address the device left.
        for mut row in done {
            row.mode = Mode::Gone;
            self.finish(row);
        }
        if let Some(slot) = fresh {
            inner.spawn_device(to.clone(), slot);
        }
    }
}
