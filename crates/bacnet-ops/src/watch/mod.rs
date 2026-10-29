//! The watch hub (#2067): one managed BACnet subscription or poller per
//! point, however many listeners share it.
//!
//! [`Hub::watch`] hands out a [`Lease`] on a [`Point`]. The first lease on a
//! point starts its entry, which goes to its device's scheduler (#2115):
//! one task per watched device, running one job at a time. The scheduler
//! reads the point once (what it reads is [`WatchOptions::reads`]), then
//! asks the device for a finite COV subscription (SubscribeCOV for an
//! object, SubscribeCOVProperty for one property) through the client's
//! managed subscriptions, which renew it before it lapses. Points the
//! device won't subscribe are polled instead, all of a device's together
//! in batched ReadPropertyMultiple requests ([`poll`]), never faster than
//! [`WatchOptions::poll_floor`]; a device that stops answering is polled
//! after a backoff. Later leases on the point share the entry; the last
//! one dropped stops it, and the scheduler cancels its subscription the
//! way the TUI's watch list does when a row goes. A listener joining while
//! the last one leaves gets either the running entry or a new one with its
//! own subscriber process identifier: both happen under one lock, so no
//! entry is shared after its stop.
//!
//! The hub keeps each point's latest values ([`Snapshot`]), so reading a
//! watched point sends nothing. COV notifications come from one receiver of
//! the client's channel ([`BACnetClient::cov_notifications`]), the path the
//! TUI's watch list and the managed renewals already use; the hub adds no
//! second notification path. The receiver never blocks the client: a burst
//! it falls behind on is counted as lagged, and changes to one point within
//! [`WatchOptions::coalesce`] are folded into one signal and counted as
//! coalesced, while the values themselves are always the latest. A front
//! end that keeps a history reads every value from [`Lease::samples`].
//!
//! Every request a scheduler sends (first reads, subscriptions, polls)
//! is charged to the hub's own [`Account`], whose token bucket
//! ([`WatchOptions::bucket`]) holds the hub to its own rate besides the
//! engine's limits. A shared point costs the same whatever the number of
//! listeners, and notifications cost nothing. Renewals take the device's
//! gate but no rate slot, as in the TUI; cancels skip the hub's bucket so a
//! shutdown isn't held to it.
//!
//! Lifetimes are finite by construction ([`Lifetime`] can't be zero): a
//! subscription that never lapses is a write in [`crate::class`], and the
//! hub never sends one.
//!
//! [`BACnetClient::cov_notifications`]: bacnet_client::client::BACnetClient::cov_notifications

mod device;
mod dispatch;
pub mod first;
mod jobs;
mod options;
pub mod poll;
mod sample;
mod shared;

use std::num::NonZeroU32;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;

pub use self::options::{
    Backoff, Fallback, MAX_ROWS, MAX_SAMPLES, MIN_POLL, MissingDevice, POLL_FLOOR, Reads,
    WatchOptions,
};
pub use self::sample::{Sample, Samples, Via};
use self::shared::{Arrival, DeviceSlot, Registry, Running, Shared};
use crate::class::{self, Class};
use crate::engine::Engine;
use crate::format::RawRow;
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

/// Subscriber process identifiers start here, clear of the small numbers
/// the TUI's watch rows use.
const PROCESS_BASE: u32 = 0x4D43_0000;

/// A COV subscription's lifetime in seconds: never zero, so never the
/// subscription that doesn't lapse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lifetime(NonZeroU32);

impl Lifetime {
    /// Five minutes, the TUI's default.
    pub const DEFAULT: Self = Self(NonZeroU32::new(300).expect("300 is not zero"));

    /// A lifetime of `secs`; `None` for 0.
    pub fn new(secs: u32) -> Option<Self> {
        let secs = NonZeroU32::new(secs)?;
        debug_assert_eq!(class::of_subscribe(secs.get()), Class::Read);
        Some(Self(secs))
    }

    /// The lifetime in seconds, at least 1.
    pub fn secs(self) -> u32 {
        self.0.get()
    }

    fn duration(self) -> Duration {
        Duration::from_secs(u64::from(self.secs()))
    }
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

/// What a watch follows on an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Watched {
    /// The object: SubscribeCOV, its [`POINT_PROPERTIES`] read and polled.
    Object,
    /// One property, or one element of it: SubscribeCOVProperty.
    Property {
        /// The property.
        property: PropertyIdentifier,
        /// The array element, if one.
        index: Option<u32>,
    },
}

/// A watched point: an object of a device, whole or one property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Point {
    /// The device instance, looked up in the client's discovery table.
    pub device: u32,
    /// The object.
    pub object: ObjectIdentifier,
    /// The object or one property.
    pub watched: Watched,
}

/// Where a point's values come from now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// COV notifications under a managed subscription.
    Cov,
    /// Polls, because the device refused COV or stopped renewing it.
    Poll,
}

/// How a point's watch is going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Its first read hasn't finished.
    Starting,
    /// Subscribed, or the last poll answered.
    Live,
    /// The device answered the last read with an error for the whole
    /// object (see [`Snapshot::note`]).
    Failed,
    /// The device didn't answer the last request.
    Unreachable,
    /// The watch ended (the hub shut down).
    Stopped,
    /// Nothing watches it any more: its device isn't in the discovery
    /// table and [`WatchOptions::missing`] says not to wait for it (see
    /// [`Snapshot::note`]).
    Gone,
}

/// A point's latest values and how its watch is going.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Each property's latest raw value, or why the device gave none.
    pub rows: Vec<RawRow>,
    /// Where the values come from; `None` before the first read.
    pub source: Option<Source>,
    /// How the watch is going.
    pub state: State,
    /// Why the point is polled or not live, as text.
    pub note: Option<String>,
    /// The hub's sequence number at the last change; 0 before any.
    pub seq: u64,
    /// When the values last changed.
    pub updated: Option<Instant>,
    /// COV notifications applied.
    pub notifications: u64,
    /// Changes folded into another's signal.
    pub coalesced: u64,
    /// Polls answered.
    pub polls: u64,
    /// Renewals the device took.
    pub renewals: u64,
    /// Why the point is polled, or was given up, as its start or a failed
    /// renewal found; a poll's error, which goes to [`note`](Self::note),
    /// leaves it. `None` while subscribed, and before the first start ends.
    pub why: Option<Why>,
}

/// Why a point is polled or given up ([`Snapshot::why`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// The device refused the subscription or didn't answer it, with the
    /// error.
    Refused(String),
    /// The first read had no answer, so no subscription was asked for.
    NoAnswer(String),
    /// A renewal failed, with its error, or stopped.
    Lapsed(Option<String>),
    /// The device isn't in the discovery table.
    Missing,
}

impl Default for Snapshot {
    /// A point not read yet: starting, no rows, nothing counted.
    fn default() -> Self {
        Self::new()
    }
}

impl Snapshot {
    pub(super) fn new() -> Self {
        Self {
            rows: Vec::new(),
            source: None,
            state: State::Starting,
            note: None,
            seq: 0,
            updated: None,
            notifications: 0,
            coalesced: 0,
            polls: 0,
            renewals: 0,
            why: None,
        }
    }
}

/// Why a watch wasn't started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchError {
    /// The hub watches its most points already.
    Full {
        /// [`WatchOptions::max_points`].
        max: usize,
    },
    /// The hub has shut down.
    Shut,
}

impl std::fmt::Display for WatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full { max } => write!(f, "the hub watches its most points ({max})"),
            Self::Shut => f.write_str("the hub has shut down"),
        }
    }
}

impl std::error::Error for WatchError {}

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
    /// Cancels queued or running: subscriptions the devices may still hold
    /// for points no longer watched.
    pub cancelling: usize,
}

/// A lease on a watched point: while any lease on it lives, the hub keeps
/// it watched. Dropping the last one stops the watch and cancels the
/// subscription.
pub struct Lease {
    shared: Arc<Shared>,
    registry: Arc<Registry>,
    signal: watch::Receiver<u64>,
}

impl Lease {
    /// The point.
    pub fn point(&self) -> Point {
        self.shared.point
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

struct Inner<T: TransportPort + 'static> {
    engine: Arc<Engine<T>>,
    options: WatchOptions,
    registry: Arc<Registry>,
    account: Arc<Account>,
    handles: Mutex<Vec<JoinHandle<()>>>,
    stop_dispatch: watch::Sender<bool>,
}

/// The watch hub over one engine. Cheap to clone: clones share one hub.
pub struct Hub<T: TransportPort + 'static> {
    inner: Arc<Inner<T>>,
}

impl<T: TransportPort + 'static> Clone for Hub<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T: TransportPort + 'static> Hub<T> {
    /// A hub over `engine`, under checked `options`. It starts its
    /// notification receiver, so it must be made inside a Tokio runtime.
    pub fn new(engine: Arc<Engine<T>>, options: WatchOptions) -> Result<Self, String> {
        options.validate()?;
        let registry = Arc::new(Registry::new(
            PROCESS_BASE,
            options.coalesce,
            options.reads.object.clone(),
            options.reads.property_flags,
        ));
        let (stop_dispatch, stopped) = watch::channel(false);
        let notifications = engine.client().cov_notifications();
        let dispatcher = tokio::spawn(dispatch::run(Arc::clone(&registry), notifications, stopped));
        let account = Arc::new(Account::new(options.bucket.clone()));
        Ok(Self {
            inner: Arc::new(Inner {
                engine,
                options,
                registry,
                account,
                handles: Mutex::new(vec![dispatcher]),
                stop_dispatch,
            }),
        })
    }

    /// The hub's settings.
    pub fn options(&self) -> &WatchOptions {
        &self.inner.options
    }

    /// A lease on `point`, starting its watch if it has none. A new
    /// watch goes to its device's scheduler, started with it when the
    /// device has none.
    pub fn watch(&self, point: Point) -> Result<Lease, WatchError> {
        let registry = &self.inner.registry;
        let (shared, scheduler) = {
            let mut table = registry.table();
            if table.shut {
                return Err(WatchError::Shut);
            }
            if let Some(slot) = table.entries.get_mut(&point) {
                slot.leases += 1;
                (Arc::clone(&slot.shared), None)
            } else {
                let max = self.inner.options.max_points;
                if *registry.tasks.borrow() >= max {
                    return Err(WatchError::Full { max });
                }
                let mut process = table.next_process;
                // 0 is skipped when the count wraps.
                while process == 0 || table.by_process.contains_key(&process) {
                    process = process.wrapping_add(1);
                }
                table.next_process = process.wrapping_add(1);
                let shared = Arc::new(Shared::new(point, process, self.inner.options.samples));
                table.entries.insert(
                    point,
                    shared::Slot {
                        shared: Arc::clone(&shared),
                        leases: 1,
                    },
                );
                table.by_process.insert(process, Arc::clone(&shared));
                let mut fresh = false;
                let device = Arc::clone(table.devices.entry(point.device).or_insert_with(|| {
                    fresh = true;
                    Arc::new(DeviceSlot {
                        inbox: Mutex::new(Vec::new()),
                        wake: tokio::sync::Notify::new(),
                    })
                }));
                device.inbox().push(Arrival {
                    shared: Arc::clone(&shared),
                    running: Running::new(registry),
                });
                device.wake.notify_one();
                (shared, fresh.then_some(device))
            }
        };
        if let Some(slot) = scheduler {
            let task = tokio::spawn(device::run(Arc::clone(&self.inner), point.device, slot));
            let mut handles = self
                .inner
                .handles
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            handles.retain(|handle| !handle.is_finished());
            handles.push(task);
        }
        let signal = shared.signal.subscribe();
        Ok(Lease {
            shared,
            registry: Arc::clone(registry),
            signal,
        })
    }

    /// `point`'s latest values, if it is watched: nothing is sent.
    pub fn peek(&self, point: &Point) -> Option<Snapshot> {
        let shared = Arc::clone(&self.inner.registry.table().entries.get(point)?.shared);
        Some(shared.snapshot())
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
            cancelling: registry.cancelling.load(Ordering::Relaxed),
        }
    }

    /// Stop every watch, wait up to `wait` for their cancels, and stop the
    /// notification receiver. Later [`watch`](Self::watch) calls fail.
    /// True when every cancel finished in time; the subscriptions of those
    /// that didn't lapse with their lifetime.
    pub async fn shutdown(&self, wait: Duration) -> bool {
        let (entries, devices): (Vec<Arc<Shared>>, Vec<Arc<DeviceSlot>>) = {
            let mut table = self.inner.registry.table();
            table.shut = true;
            table.by_process.clear();
            let entries = table.entries.drain().map(|(_, slot)| slot.shared).collect();
            (entries, table.devices.values().cloned().collect())
        };
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
        let handles: Vec<JoinHandle<()>> = self
            .inner
            .handles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect();
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

#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod scheduler_tests;
#[cfg(test)]
mod test_lab;
#[cfg(test)]
mod tests;
