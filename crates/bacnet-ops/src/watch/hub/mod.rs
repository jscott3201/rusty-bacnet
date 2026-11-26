//! The hub under [`Watch`](super::Watch), for a front end that runs its
//! own (the `bacnet` tool's watch list and MCP server keep a history,
//! serve several sessions and pick their own [`WatchOptions`]). An
//! application reads a [`Watch`](super::Watch) instead.
//!
//! [`Hub::watch`] hands out a [`Lease`] on a [`Point`]. The first lease on
//! a point starts its entry, which goes to its device's scheduler (#2115):
//! one task per watched address, running one job at a time (a following
//! point's goes where the hub follows its device, and moves with it,
//! #2795; [`Lease::at`] says where). The scheduler
//! reads the point once (what it reads is [`WatchOptions::reads`]), then
//! asks for a finite COV subscription, renewed before it lapses; points
//! the device won't subscribe are polled, all of a device's together in
//! batched ReadPropertyMultiple requests (the batches
//! [`Device::read_many`](crate::Device::read_many) reads in), never faster
//! than [`WatchOptions::poll_floor`]. Later leases on the point share the
//! entry; the last one dropped stops it, and the scheduler cancels its
//! subscription. A listener joining while the last one leaves gets either
//! the running entry or a new one with its own subscriber process
//! identifier: both happen under one lock, so no entry is shared after its
//! stop.
//!
//! The hub keeps each point's latest values ([`Snapshot`]), so reading a
//! watched point sends nothing. COV notifications come from one receiver
//! of the client's channel
//! ([`BACnetClient::cov_notifications`](bacnet_client::client::BACnetClient::cov_notifications)),
//! the path the managed renewals use; the hub adds no second notification
//! path. The receiver never blocks the client: a burst it falls behind on
//! is counted as lagged, and changes to one point within
//! [`WatchOptions::coalesce`] are folded into one signal and counted as
//! coalesced, while the values themselves are always the latest. A front
//! end that keeps a history reads every value from [`Lease::samples`].
//!
//! Every request a scheduler sends (first reads, subscriptions, polls) is
//! charged to the hub's own [`Account`], whose token bucket
//! ([`WatchOptions::bucket`]) holds the hub to its own rate besides the
//! client's limits. A shared point costs the same whatever the number of
//! listeners, and notifications cost nothing. Renewals take the device's
//! gate but no rate slot; cancels skip the hub's bucket so a shutdown
//! isn't held to it.

pub mod first;
pub(crate) mod ranges;
mod snapshot;

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::sync::MutexExt;
use tokio::sync::watch;
use tokio::task::JoinHandle;

pub use self::snapshot::{Phase, Snapshot, Source};
pub use super::options::{
    Backoff, Fallback, MAX_POLL, MAX_ROWS, MAX_SAMPLES, MIN_POLL, MissingDevice, POLL_FLOOR, Reads,
    WatchOptions,
};
pub use super::sample::{Sample, Samples};
pub use super::settings::Lifetime;
use super::shared::{Arrival, DeviceSlot, Registry, Running, Shared};
use super::{Binding, Point, WatchError, device, dispatch, shared};
use crate::client::Client;
use crate::device::{DeviceAddress, FollowKey, Followed};
use crate::limits::Account;

/// The properties an object watch reads: what an agent or an operator needs
/// to reason about a point (value, status, units, priorities, increment).
pub const POINT_PROPERTIES: [PropertyIdentifier; 10] = [
    PropertyIdentifier::OBJECT_NAME,
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::STATUS_FLAGS,
    PropertyIdentifier::EVENT_STATE,
    PropertyIdentifier::RELIABILITY,
    PropertyIdentifier::OUT_OF_SERVICE,
    PropertyIdentifier::UNITS,
    PropertyIdentifier::PRIORITY_ARRAY,
    PropertyIdentifier::RELINQUISH_DEFAULT,
    PropertyIdentifier::COV_INCREMENT,
];

/// Why a polled point has no reading: the device's ReadPropertyMultiple
/// answer left its object out, or its rows don't hold the watched
/// property.
pub const MISSING: &str = "missing from the device's answer";

/// Subscriber process identifiers start here, clear of the small numbers
/// the one-shot commands use.
pub(in crate::watch) const PROCESS_BASE: u32 = 0x4D43_0000;

/// Each hub on a client takes its own range of this many subscriber
/// process identifiers (#2185): two hubs on one client (the TUI's watch
/// list and its attached agents') then never share one, so neither's
/// cancel can end the other's subscription. A hub's `max_points` is held
/// below the range's size.
const PROCESS_SPAN: u32 = 0x1_0000;

/// How many ranges a client has: at most this many hubs live on one client
/// at once, its pooled ones included. A hub holds its range until it is gone,
/// its cancels done or given up ([`ranges`]).
pub(in crate::watch) const PROCESS_RANGES: u32 = 256;

/// The first identifier of range `index`.
fn process_base(index: u32) -> u32 {
    PROCESS_BASE + (index % PROCESS_RANGES) * PROCESS_SPAN
}

/// The identifier after `process` in the range starting at `base`.
fn next_process(base: u32, process: u32) -> u32 {
    base + (process.wrapping_sub(base).wrapping_add(1)) % PROCESS_SPAN
}

/// How long before a lifetime of `secs` ends its renewal should go: a
/// tenth of it (1 to 30 s), but never later than two `request_budget`s
/// before the end. A budget is the time one request takes with every
/// retry: the renewal waits at most one for the request holding the
/// device's gate (#1791), which may be a whole budget on a device that
/// stops answering, then needs one of its own, so neither a busy device nor
/// one lost packet lets it lapse. Each worker queues at most one request
/// per device behind that holder, and they rarely all coincide with a
/// renewal, so two budgets cover the case that matters without renewing
/// far earlier than needed. The margin stays at most half the lifetime.
pub fn renewal_margin(secs: u32, request_budget: Duration) -> Duration {
    let tenth = Duration::from_secs(u64::from(secs / 10).clamp(1, 30));
    let half = Duration::from_secs(u64::from(secs)) / 2;
    tenth.max(request_budget * 2).min(half)
}

/// The hub's footprint, for a session view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubStats {
    /// Points watched.
    pub points: usize,
    /// Entries still running, those cancelling included.
    pub tasks: usize,
    /// Points under a COV subscription.
    pub cov: usize,
    /// Points polled.
    pub polled: usize,
    /// [`WatchOptions::max_points`].
    pub max_points: usize,
    /// Requests the entries sent.
    pub requests: u32,
    /// How long they waited for the limits.
    pub throttled: Duration,
    /// COV notifications lost because the hub's receiver fell behind.
    pub lagged: u64,
    /// COV-multiple notifications (ConfirmedCOVNotificationMultiple and
    /// UnconfirmedCOVNotificationMultiple) the hub heard and left alone. It
    /// subscribes per object or property, never with
    /// SubscribeCOVPropertyMultiple, so none answers one of its watches; they
    /// still reach the client's other receivers
    /// ([`BACnetClient::cov_notifications`](bacnet_client::client::BACnetClient::cov_notifications)).
    pub cov_multiple_ignored: u64,
    /// Cancels queued or running: subscriptions the devices may still hold
    /// for points no longer watched.
    pub cancelling: usize,
}

/// A lease on a watched point: while any lease on it lives, the hub keeps
/// it watched. Dropping the last one stops the watch and cancels the
/// subscription.
pub struct Lease {
    pub(in crate::watch) shared: Arc<Shared>,
    registry: Arc<Registry>,
    signal: watch::Receiver<u64>,
}

impl Lease {
    /// The point, as the hub keys it.
    pub fn point(&self) -> Point {
        self.shared.point.clone()
    }

    /// Where the point is watched now: a pinned point's address, or where
    /// the hub follows a following point's device (#2795).
    pub fn at(&self) -> DeviceAddress {
        self.shared.at()
    }

    /// Its latest values and state.
    pub fn snapshot(&self) -> Snapshot {
        self.shared.snapshot()
    }

    /// Wait for the next signalled change. Changes within the coalescing
    /// window make one signal, and a listener that falls behind sees only
    /// the latest.
    pub async fn changed(&mut self) {
        // The sender lives as long as the entry, which this lease holds.
        if self.signal.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }

    /// Every value the point gets from now on, as read or notified, which
    /// neither coalescing nor a slow listener folds away; it holds
    /// [`WatchOptions::samples`] and counts what a reader that falls behind
    /// loses.
    pub fn samples(&self) -> Samples {
        Samples::new(self.shared.samples.subscribe())
    }

    /// A second view of this lease's change signal, for a waiter that
    /// can't hold the lease itself (one kept under a lock, say). It starts
    /// where the lease stands and keeps nothing watched: once the point's
    /// last lease goes, it never fires again.
    pub fn changes(&self) -> Changes {
        Changes {
            signal: self.signal.clone(),
        }
    }
}

/// A point's change signal without a lease ([`Lease::changes`]).
#[derive(Debug, Clone)]
pub struct Changes {
    signal: watch::Receiver<u64>,
}

impl Changes {
    /// Wait for the next signalled change, as [`Lease::changed`] does;
    /// pending for ever once the point is no longer watched. Cancel safe.
    pub async fn changed(&mut self) {
        if self.signal.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }

    /// Whether `other` is a signal of the same watch: false once the point
    /// was let go and watched again, which makes a new watch.
    pub fn same(&self, other: &Changes) -> bool {
        self.signal.same_channel(&other.signal)
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.registry.release(&self.shared);
    }
}

impl std::fmt::Debug for Lease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lease")
            .field("point", &self.shared.point)
            .finish()
    }
}

pub(crate) struct Inner<T: TransportPort + 'static> {
    pub(in crate::watch) client: Client<T>,
    pub(in crate::watch) options: WatchOptions,
    pub(in crate::watch) registry: Arc<Registry>,
    pub(in crate::watch) account: Arc<Account>,
    handles: Mutex<Vec<JoinHandle<()>>>,
    stop_dispatch: watch::Sender<bool>,
    /// The range of subscriber process identifiers the hub holds, and
    /// whether it gives it back when it goes (a test keeps one).
    range: (u32, bool),
}

impl<T: TransportPort + 'static> Inner<T> {
    /// Start the scheduler of the device at `address`, which the table
    /// knows as `slot` from now on.
    pub(in crate::watch) fn spawn_device(
        self: &Arc<Self>,
        address: DeviceAddress,
        slot: Arc<DeviceSlot>,
    ) {
        let task = tokio::spawn(device::run(Arc::clone(self), address, slot));
        let mut handles = self.handles.locked();
        handles.retain(|handle| !handle.is_finished());
        handles.push(task);
    }
}

impl<T: TransportPort + 'static> Drop for Inner<T> {
    /// The hub is gone: its last clone dropped and every task that held it
    /// ended, cancels included, so no cancel of its can still go. (A
    /// renewal of its, aborted without a join, may still finish; it is no
    /// cancel.)
    fn drop(&mut self) {
        if let (range, true) = self.range {
            self.client.release_range(range);
        }
    }
}

/// The watch hub over one [`Client`]. Cheap to clone: clones share one hub.
pub struct Hub<T: TransportPort + 'static> {
    pub(in crate::watch) inner: Arc<Inner<T>>,
}

impl<T: TransportPort + 'static> Clone for Hub<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T: TransportPort + 'static> Hub<T> {
    /// A hub over `client`, under checked `options`, in the next range of
    /// subscriber process identifiers no live hub on the client holds,
    /// going round from the one taken last; it gives the range back once it
    /// is gone, its tasks ended. It starts its notification receiver.
    ///
    /// # Errors
    ///
    /// [`WatchError::Settings`] for `options` that
    /// [`validate`](WatchOptions::validate) refuses, [`WatchError::NoRuntime`]
    /// outside a Tokio runtime, which its tasks need, and
    /// [`WatchError::Hubs`] when every range is held: 256 hubs live on the
    /// client, the client's pooled ones included
    /// ([`Client::watch_with`]), or still cancelling. Nothing is taken
    /// then.
    pub fn new(client: Client<T>, options: WatchOptions) -> Result<Self, WatchError> {
        options.validate()?;
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(WatchError::NoRuntime);
        }
        let range = client.take_range().ok_or(WatchError::Hubs {
            max: PROCESS_RANGES as usize,
        })?;
        Ok(Self::in_range(client, options, range, true))
    }

    /// A hub as [`new`](Self::new) makes it, under `options` already
    /// checked, in range `range`, which it gives back when it goes if
    /// `release`: a test makes hubs in one range it keeps.
    pub(in crate::watch) fn in_range(
        client: Client<T>,
        options: WatchOptions,
        range: u32,
        release: bool,
    ) -> Self {
        let registry = Arc::new(Registry::new(
            process_base(range),
            options.coalesce,
            options.reads.object.clone(),
            options.reads.property_flags,
        ));
        let (stop_dispatch, stopped) = watch::channel(false);
        let low = Arc::clone(client.low_level());
        let heard = dispatch::Heard {
            events: client.events(),
            devices: low.device_events(),
            own: Box::new(move || low.network_number()),
        };
        let notifications = client.low_level().cov_notifications();
        let dispatcher = tokio::spawn(dispatch::run(
            Arc::clone(&registry),
            notifications,
            heard,
            stopped,
        ));
        let bucket = options.bucket.clone();
        let account = Arc::new(match options.agent {
            true => Account::agent(bucket),
            false => Account::new(bucket),
        });
        Self {
            inner: Arc::new(Inner {
                client,
                options,
                registry,
                account,
                handles: Mutex::new(vec![dispatcher]),
                stop_dispatch,
                range: (range, release),
            }),
        }
    }

    /// The hub's settings.
    pub fn options(&self) -> &WatchOptions {
        &self.inner.options
    }

    /// A lease on `point`, starting its watch if it has none. A new
    /// watch goes to its device's scheduler, one a device address, started
    /// with it when the device has none. A station on the client's own
    /// network is keyed as the local station at its MAC once the client
    /// knows the network's number, as the device handles key it. A
    /// following point ([`Binding::Follows`]) is
    /// watched where the hub follows its device already, else where the
    /// client's handles following it reach it, else at the one address
    /// claiming it in the discovery table (#2795).
    ///
    /// # Errors
    ///
    /// As [`stream`](Self::stream), and for a following point no address
    /// can be found for, [`WatchError::UnknownDevice`] or
    /// [`WatchError::Ambiguous`].
    ///
    /// # Panics
    ///
    /// Outside a Tokio runtime, when the point's device has no scheduler
    /// yet: the scheduler is a task.
    pub fn watch(&self, point: Point) -> Result<Lease, WatchError> {
        let mut leases = self.watch_all(vec![self.keyed(point)])?;
        Ok(leases.pop().expect("one lease a point"))
    }

    /// A lease on each of `points`, keyed ([`keyed`](Self::keyed)) and
    /// distinct, or none: under one lock of the table, so a call that
    /// would take the hub past [`WatchOptions::max_points`], or names a
    /// following point whose device can't be found, takes nothing and
    /// sends nothing.
    pub(in crate::watch) fn watch_all(&self, points: Vec<Point>) -> Result<Vec<Lease>, WatchError> {
        let registry = &self.inner.registry;
        let client = &self.inner.client;
        // Where each followed instance is, before the table's lock: the
        // client's own locks are never taken under it.
        let mut places = HashMap::new();
        for point in &points {
            if let Some(key) = self.follow_key(point)
                && !places.contains_key(&key)
            {
                let place = client.follow_place(key)?;
                let at = client.place_of(&place);
                places.insert(key, (place, at));
            }
        }
        let mut fresh = Vec::new();
        let shared: Vec<Arc<Shared>> = {
            let mut table = registry.table();
            if table.shut {
                return Err(WatchError::Shut);
            }
            let max = self.inner.options.max_points;
            let new = points
                .iter()
                .filter(|point| !table.entries.contains_key(point))
                .count();
            if new > 0 && *registry.tasks.borrow() + new > max {
                return Err(WatchError::Full { max });
            }
            points
                .into_iter()
                .map(|point| {
                    let follows = self.follow_key(&point);
                    let place = follows.and_then(|key| places.get(&key).cloned());
                    self.take(&mut table, point, place, &mut fresh)
                })
                .collect()
        };
        for (address, slot) in fresh {
            self.inner.spawn_device(address, slot);
        }
        Ok(shared
            .into_iter()
            .map(|shared| {
                let signal = shared.signal.subscribe();
                Lease {
                    shared,
                    registry: Arc::clone(registry),
                    signal,
                }
            })
            .collect())
    }

    /// `point`'s entry, the running one or a new one handed to its
    /// device's scheduler; a device that has none is noted in `fresh`, to
    /// start one once the table's lock is let go. A new following point
    /// starts where the hub follows its device, else where `place` (the
    /// client's handles') reaches it.
    fn take(
        &self,
        table: &mut shared::Table,
        point: Point,
        place: Option<(Followed, DeviceAddress)>,
        fresh: &mut Vec<(DeviceAddress, Arc<DeviceSlot>)>,
    ) -> Arc<Shared> {
        let registry = &self.inner.registry;
        if let Some(slot) = table.entries.get_mut(&point) {
            slot.leases += 1;
            return Arc::clone(&slot.shared);
        }
        let follows = self.follow_key(&point);
        let at = match (&point.device, follows) {
            (Binding::At(address), _) => address.clone(),
            (Binding::Follows { .. }, key) => {
                let key = key.expect("a following point has a key");
                table.follow(key, || place.expect("placed before the table's lock"))
            }
        };
        // Within the hub's own range, never 0 or another hub's.
        let base = registry.process_base;
        let mut process = table.next_process;
        while table.by_process.contains_key(&process) {
            process = next_process(base, process);
        }
        table.next_process = next_process(base, process);
        let shared = Arc::new(Shared::new(
            point.clone(),
            process,
            self.inner.options.samples,
            at.clone(),
        ));
        table.entries.insert(
            point,
            shared::Slot {
                shared: Arc::clone(&shared),
                leases: 1,
                follows,
            },
        );
        table.by_process.insert(process, Arc::clone(&shared));
        let mut started = false;
        let device = Arc::clone(table.devices.entry(at.clone()).or_insert_with(|| {
            started = true;
            Arc::new(DeviceSlot::new())
        }));
        device.inbox().push(Arrival {
            shared: Arc::clone(&shared),
            running: Running::new(registry),
        });
        device.wake.notify_one();
        if started {
            fresh.push((at, device));
        }
        shared
    }

    /// The follow-table key of a following point, as the client keys its
    /// handles' (its network named, or any); `None` for a pinned point.
    pub(in crate::watch) fn follow_key(&self, point: &Point) -> Option<FollowKey> {
        match point.device {
            Binding::At(_) => None,
            Binding::Follows { instance, network } => {
                Some(self.inner.client.key(network, instance))
            }
        }
    }

    /// `point`'s latest values, if it is watched: nothing is sent.
    pub fn peek(&self, point: &Point) -> Option<Snapshot> {
        let point = self.keyed(point.clone());
        let shared = Arc::clone(&self.inner.registry.table().entries.get(&point)?.shared);
        Some(shared.snapshot())
    }

    /// `point` as the hub keys it: a pinned point's address localized, and
    /// a following point's network the client's own as the local one (0),
    /// once the client knows its number.
    pub(in crate::watch) fn keyed(&self, mut point: Point) -> Point {
        point.device = match point.device {
            Binding::At(address) => {
                let own = self.inner.client.low_level().network_number();
                Binding::At(address.localized(own))
            }
            Binding::Follows { instance, network } => {
                let (network, _) = self.inner.client.key(network, instance);
                Binding::Follows { instance, network }
            }
        };
        point
    }

    /// The hub's sequence number: every change takes the next one, so a
    /// point changed since an earlier reading of this when its
    /// [`Snapshot::seq`] is greater.
    pub fn seq(&self) -> u64 {
        self.inner.registry.seq.load(Ordering::Relaxed)
    }

    /// The hub's footprint now.
    pub fn stats(&self) -> HubStats {
        let registry = &self.inner.registry;
        let entries: Vec<Arc<Shared>> = registry
            .table()
            .entries
            .values()
            .map(|slot| Arc::clone(&slot.shared))
            .collect();
        let sources: Vec<Option<Source>> = entries
            .iter()
            .map(|shared| shared.lock().snapshot.source)
            .collect();
        HubStats {
            points: entries.len(),
            tasks: *registry.tasks.borrow(),
            cov: sources.iter().filter(|s| **s == Some(Source::Cov)).count(),
            polled: sources.iter().filter(|s| **s == Some(Source::Poll)).count(),
            max_points: self.inner.options.max_points,
            requests: self.inner.account.requests(),
            throttled: self.inner.account.throttled(),
            lagged: registry.lagged.load(Ordering::Relaxed),
            cov_multiple_ignored: registry.cov_multiple_ignored.load(Ordering::Relaxed),
            cancelling: registry.cancelling.load(Ordering::Relaxed),
        }
    }

    /// Stop every watch, wait up to `wait` for their cancels, and stop the
    /// notification receiver. Later [`watch`](Self::watch) calls fail.
    /// Every [`Watch`](super::Watch) taken from the hub ends first, its
    /// stream giving `None` at once, so none sees a point stop. True when
    /// every cancel finished in time; the subscriptions of those that
    /// didn't lapse with their lifetime.
    pub async fn shutdown(&self, wait: Duration) -> bool {
        let (entries, devices): (Vec<Arc<Shared>>, Vec<Arc<DeviceSlot>>) = {
            let mut table = self.inner.registry.table();
            table.shut = true;
            table.by_process.clear();
            table.follows.clear();
            let entries = table.entries.drain().map(|(_, slot)| slot.shared).collect();
            (entries, table.devices.values().cloned().collect())
        };
        let watches = std::mem::take(&mut *self.inner.registry.watches.locked());
        super::stream::end_all(watches);
        for shared in &entries {
            shared.stop();
        }
        for device in &devices {
            device.wake.notify_one();
        }
        let mut tasks = self.inner.registry.tasks.subscribe();
        let done = tokio::time::timeout(wait, tasks.wait_for(|running| *running == 0))
            .await
            .is_ok();
        self.inner.stop_dispatch.send_replace(true);
        let handles: Vec<JoinHandle<()>> = self.inner.handles.locked().drain(..).collect();
        // Every task is aborted, so the joins below take no time and the
        // call keeps to `wait`. Once every point is done, cancels included,
        // a scheduler can only be running a poll of points no longer
        // watched, which nothing needs; past the deadline, a cancel still
        // waiting is given up and its subscription lapses with its
        // lifetime.
        for handle in &handles {
            handle.abort();
        }
        for handle in handles {
            let _ = handle.await;
        }
        // Points an aborted scheduler never took are counted out.
        let devices: Vec<Arc<DeviceSlot>> = self
            .inner
            .registry
            .table()
            .devices
            .drain()
            .map(|(_, d)| d)
            .collect();
        for device in devices {
            drop(std::mem::take(&mut *device.inbox()));
        }
        if !done {
            // The aborted cancels never finish to count themselves out.
            self.inner.registry.cancelling.store(0, Ordering::Relaxed);
            tracing::warn!(
                "gave up waiting for COV cancels; those subscriptions end with their lifetime"
            );
        }
        done
    }
}
