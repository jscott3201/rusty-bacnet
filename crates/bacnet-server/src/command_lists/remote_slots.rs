//! Room for the requests runs make in other devices: a Command's remote
//! actions, and a Channel's reads and writes of its remote members (#1343).
//!
//! The server's runs share two bounds, whichever run makes the request:
//!
//! - [`RUN_REQUESTS`] requests outstanding at once across the whole server.
//!   Every run is its own task, and one WriteGroup can start many Channel
//!   distributions at once, so a bound per run would let them take most of
//!   the device's 256 invoke IDs, which confirmed COV and event notifications
//!   (up to 255 in flight) and Audit deliveries (up to 64) lease from too.
//!   Thirty-two, an eighth of them, leaves those the rest, and a run that
//!   still finds no invoke ID free fails that write as before.
//! - [`PER_DEVICE`] request outstanding at once in each device. Before
//!   Channel members were written side by side a device met one request from
//!   a run at a time, and many small devices, MS/TP ones especially, serve
//!   one confirmed request at a time and answer more with an Abort or not at
//!   all. One at a time also lets a device that answers nothing be found
//!   silent by a single request before the others for it go out.
//!
//! A request takes its device's slot before one of the server's, so a request
//! queued behind a slow device holds none of the server's. Both queues are
//! first come, first served. Dropping a slot, or a request still waiting for
//! one, frees what it held.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::sync::MutexExt;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// The most requests in other devices the server's runs keep outstanding at
/// once, reads and writes together.
pub(crate) const RUN_REQUESTS: usize = 32;

/// The most requests the server's runs keep outstanding at once in any one
/// device.
pub(crate) const PER_DEVICE: usize = 1;

/// The queues requests from runs to other devices wait in.
pub(crate) struct RemoteSlots {
    server: Arc<Semaphore>,
    /// The queue of each device a request is made in or waiting for; a
    /// device leaves once nothing holds or waits for its slot.
    devices: Mutex<HashMap<ObjectIdentifier, DeviceQueue>>,
}

struct DeviceQueue {
    /// Requests holding or waiting for this device's slot.
    users: usize,
    slots: Arc<Semaphore>,
}

impl Default for RemoteSlots {
    fn default() -> Self {
        Self {
            server: Arc::new(Semaphore::new(RUN_REQUESTS)),
            devices: Mutex::default(),
        }
    }
}

impl RemoteSlots {
    /// Wait for room to make one request in `device`: its own slot, then one
    /// of the server's. The request is made while the returned slot lives.
    pub(crate) async fn acquire(&self, device: ObjectIdentifier) -> RemoteSlot<'_> {
        let (user, slots) = self.join(device);
        let device_slot = slots
            .acquire_owned()
            .await
            .expect("a device's slots are never closed");
        let server_slot = Arc::clone(&self.server)
            .acquire_owned()
            .await
            .expect("the server's slots are never closed");
        RemoteSlot {
            _server: server_slot,
            _device: device_slot,
            _user: user,
        }
    }

    /// Requests from runs outstanding in other devices now.
    #[cfg(test)]
    pub(crate) fn outstanding(&self) -> usize {
        RUN_REQUESTS - self.server.available_permits()
    }

    /// Devices with a request held or waiting.
    #[cfg(test)]
    pub(crate) fn devices(&self) -> usize {
        self.lock().len()
    }

    fn join(&self, device: ObjectIdentifier) -> (DeviceUser<'_>, Arc<Semaphore>) {
        let mut devices = self.lock();
        let queue = devices.entry(device).or_insert_with(|| DeviceQueue {
            users: 0,
            slots: Arc::new(Semaphore::new(PER_DEVICE)),
        });
        queue.users += 1;
        let slots = Arc::clone(&queue.slots);
        (
            DeviceUser {
                slots: self,
                device,
            },
            slots,
        )
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<ObjectIdentifier, DeviceQueue>> {
        self.devices.locked()
    }
}

/// Room for one request in another device.
pub(crate) struct RemoteSlot<'s> {
    _server: OwnedSemaphorePermit,
    _device: OwnedSemaphorePermit,
    _user: DeviceUser<'s>,
}

/// One request holding or waiting for a device's slot; dropping it lets the
/// device's queue go once nothing else uses it.
struct DeviceUser<'s> {
    slots: &'s RemoteSlots,
    device: ObjectIdentifier,
}

impl Drop for DeviceUser<'_> {
    fn drop(&mut self) {
        let mut devices = self.slots.lock();
        if let Some(queue) = devices.get_mut(&self.device) {
            queue.users -= 1;
            if queue.users == 0 {
                devices.remove(&self.device);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;
    use std::future::Future;
    use std::task::Poll;

    fn device(instance: u32) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
    }

    #[tokio::test]
    async fn a_device_takes_one_request_and_the_server_thirty_two() {
        let slots = RemoteSlots::default();
        let held: Vec<_> = futures_util::future::join_all(
            (0..RUN_REQUESTS as u32).map(|instance| slots.acquire(device(instance))),
        )
        .await;
        assert_eq!(slots.outstanding(), RUN_REQUESTS);
        // A 33rd device waits for the server, a second request to device 0
        // for that device.
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let mut more = Box::pin(slots.acquire(device(99)));
        let mut again = Box::pin(slots.acquire(device(0)));
        assert!(more.as_mut().poll(&mut cx).is_pending());
        assert!(again.as_mut().poll(&mut cx).is_pending());
        drop(held);
        let (Poll::Ready(more), Poll::Ready(again)) =
            (more.as_mut().poll(&mut cx), again.as_mut().poll(&mut cx))
        else {
            panic!("both get through once the others let go");
        };
        assert_eq!(slots.outstanding(), 2);
        drop((more, again));
        assert_eq!((slots.outstanding(), slots.devices()), (0, 0));
    }

    #[tokio::test]
    async fn a_device_leaves_once_nothing_holds_or_waits_for_it() {
        let slots = RemoteSlots::default();
        let held = slots.acquire(device(1)).await;
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let mut waiting = Box::pin(slots.acquire(device(1)));
        assert!(waiting.as_mut().poll(&mut cx).is_pending());
        assert_eq!(slots.devices(), 1);
        // A waiter dropped, then the holder: the device's queue goes.
        drop(waiting);
        assert_eq!(slots.devices(), 1);
        drop(held);
        assert_eq!((slots.devices(), slots.outstanding()), (0, 0));
    }
}
