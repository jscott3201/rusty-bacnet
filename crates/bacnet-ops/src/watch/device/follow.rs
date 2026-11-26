//! Following points (#2795): a point that follows its device's instance
//! ([`Binding::Follows`]) goes to the address
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
//! following points start where it follows the device now, and points
//! placed at the old address just as the move was taken go with the
//! others, in the same step under the table's lock. A move it took after
//! asking the old address moves the client's handles following the
//! instance too, telling [`Client::events`](crate::Client::events); one
//! taken without (the hub's own request there went unanswered, or no
//! route) moves none.
//!
//! [`Follow`]: super::super::shared::Follow

use std::sync::Arc;

use bacnet_transport::port::TransportPort;

use super::super::Binding;
use super::super::shared::{Arrival, DeviceSlot, Shared};
use super::{Device, Mode, Row};
use crate::device::{DeviceAddress, FollowKey, Followed};

/// Whether `shared` follows the device of `instance` on a network `to` is
/// on: its watch goes there when the device does.
pub(super) fn follows_to(shared: &Shared, instance: u32, to: &DeviceAddress) -> bool {
    match shared.point.device {
        Binding::Follows {
            instance: followed,
            network,
        } => followed == instance && network.is_none_or(|network| network == to.network()),
        Binding::At(_) => false,
    }
}

impl<T: TransportPort + 'static> Device<T> {
    /// The device of `instance` answers at `to` now: in one step under the
    /// table's lock, the hub follows `keys` there (their next points start
    /// there), the following points `processes` go there, and so do those
    /// of the instance that arrived here since the scheduler last took its
    /// arrivals (a point added just as the move was taken), so none is
    /// stranded where the device left. The client's handles following the
    /// instance move with them only when this address was `asked` and
    /// didn't answer as the instance: the hub's own look after its own
    /// request went unanswered here moves none.
    pub(super) fn follow_to(
        &mut self,
        (instance, keys): (u32, Vec<FollowKey>),
        processes: Vec<u32>,
        to: &DeviceAddress,
        asked: bool,
    ) {
        let inner = Arc::clone(&self.inner);
        let registry = &inner.registry;
        let mut done = Vec::new();
        let mut fresh = None;
        let places: Vec<(FollowKey, Followed)> = {
            let mut table = registry.table();
            let mut places = Vec::new();
            for key in keys {
                if let Some(follow) = table.follows.get_mut(&key)
                    && follow.at == self.address
                {
                    follow.at = to.clone();
                    places.push((key, Arc::clone(&follow.place)));
                }
            }
            let mut going = Vec::new();
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
                going.push(Arrival { shared, running });
            }
            // Arrivals the table placed here before the move: never started
            // here, they start there.
            if !table.shut {
                let mut inbox = self.slot.inbox();
                let (stranded, stay): (Vec<Arrival>, Vec<Arrival>) = inbox
                    .drain(..)
                    .partition(|arrival| follows_to(&arrival.shared, instance, to));
                *inbox = stay;
                for arrival in stranded {
                    arrival.shared.rehome(to);
                    going.push(arrival);
                }
            }
            if !going.is_empty() {
                let slot = Arc::clone(table.devices.entry(to.clone()).or_insert_with(|| {
                    let slot = Arc::new(DeviceSlot::new());
                    fresh = Some(Arc::clone(&slot));
                    slot
                }));
                slot.inbox().extend(going);
                slot.wake.notify_one();
            }
            places
        };
        // Let go: nothing is cancelled at the address the device left.
        for mut row in done {
            row.mode = Mode::Gone;
            self.finish(row);
        }
        if let Some(slot) = fresh {
            inner.spawn_device(to.clone(), slot);
        }
        if asked {
            for (key, place) in places {
                self.inner
                    .client
                    .move_to(key, &place, &self.address, to.clone());
            }
        }
    }
}
