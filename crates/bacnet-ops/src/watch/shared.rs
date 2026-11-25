//! What a point's leases, its device's scheduler and the notification
//! receiver share: the entry's snapshot and signals, and the hub's table
//! of entries and devices. Every lock here is a std mutex held for a few
//! field updates, never across an await. The order is the table's, then a
//! device's inbox or what it heard ([`Heard`]), then an entry's, then a
//! [`Watch`](super::Watch)'s mailbox, which takes no other. A watch is
//! woken only once the entry's lock is let go ([`release`]).

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::Waker;
use std::time::{Duration, SystemTime};

use bacnet_client::client::{COVEvent, Station};
use bacnet_types::enums::PropertyIdentifier;
use tokio::sync::{Notify, broadcast, watch};
use tokio::time::Instant;

use super::hub::{Phase, Snapshot, Source};
use super::options::MAX_ROWS;
use super::sample::{Sample, Via};
use super::stream::Listener;
use super::{Point, Why, dispatch};
use crate::device::DeviceAddress;
use crate::format::{PropertyError, RawRow};

/// One entry's state, shared by its scheduler, its leases and the
/// dispatcher.
pub(super) struct Shared {
    pub(super) point: Point,
    pub(super) process: u32,
    state: Mutex<EntryState>,
    /// Carries the hub's counter at each change it signals.
    pub(super) signal: watch::Sender<u64>,
    /// Every value the point gets, for [`Lease::samples`](super::hub::Lease::samples).
    pub(super) samples: broadcast::Sender<Sample>,
    /// Set once to stop the entry.
    stop: watch::Sender<bool>,
}

pub(super) struct EntryState {
    pub(super) snapshot: Snapshot,
    /// Where the device is, once looked up: notifications must come from
    /// there.
    pub(super) target: Option<Station>,
    /// When a notification last set each property's row since the last
    /// first read was folded. A first read folded in later skips the rows
    /// notified at or after it ended: those values are newer (#2169).
    /// Bounded by the rows, as only a row's property is merged, and emptied
    /// by each first read's fold.
    notified: HashMap<PropertyIdentifier, Instant>,
    last_signal: Option<Instant>,
    /// A change waits for the end of the coalescing window.
    pending: bool,
    /// The device's error for the whole object at the last read that
    /// failed so ([`Phase::Failed`]), for an update's `error`; `None` once
    /// a read answers, a notification carries the watched property, or a
    /// subscription is taken.
    pub(super) failure: Option<PropertyError>,
    /// When the values last changed, by the wall clock, for an update.
    pub(super) wall: Option<SystemTime>,
    /// The instance of the point's device: what its address named when
    /// the watch started (the discovery table, or one read by the wildcard
    /// instance), else the first instance announced there. Nothing from
    /// the address counts as the point's while it names another (#2603).
    pub(super) identity: Option<u32>,
    /// The [`Watch`](super::Watch)es told of each signalled change: at
    /// most one a watch holding the point, and those dropped until the
    /// next change prunes them.
    pub(super) listeners: Vec<Listener>,
    /// The wakers of the watches told of a change, woken by [`release`]
    /// once this lock is let go.
    wakers: Vec<Waker>,
}

impl EntryState {
    /// Whether the watch has ended, given up or stopped: only a stop, or
    /// a revival by the scheduler ([`Shared::revive`]), leaves it.
    fn ended(&self) -> bool {
        matches!(self.snapshot.state, Phase::Gone | Phase::Stopped)
    }
}

/// Let `guard` go, then wake the watches a change under it told.
pub(super) fn release(mut guard: MutexGuard<'_, EntryState>) {
    let wakers = std::mem::take(&mut guard.wakers);
    drop(guard);
    wakers.into_iter().for_each(Waker::wake);
}

impl EntryState {
    /// A test notes a notification's arrival.
    #[cfg(test)]
    pub(super) fn mark_notified(&mut self, property: PropertyIdentifier, at: Instant) {
        self.notified.insert(property, at);
    }
}

impl Shared {
    pub(super) fn new(point: Point, process: u32, samples: usize) -> Self {
        Self {
            point,
            process,
            state: Mutex::new(EntryState {
                snapshot: Snapshot::new(),
                target: None,
                notified: HashMap::new(),
                last_signal: None,
                pending: false,
                failure: None,
                wall: None,
                identity: None,
                listeners: Vec::new(),
                wakers: Vec::new(),
            }),
            signal: watch::Sender::new(0),
            samples: broadcast::Sender::new(samples),
            stop: watch::Sender::new(false),
        }
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, EntryState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn snapshot(&self) -> Snapshot {
        self.lock().snapshot.clone()
    }

    pub(super) fn stop(&self) {
        self.stop.send_replace(true);
    }

    pub(super) fn is_stopped(&self) -> bool {
        *self.stop.borrow()
    }

    /// Hand a sample to the leases that asked for samples, if any.
    pub(super) fn sample(&self, values: Result<Vec<RawRow>, String>, via: Via) {
        if self.samples.receiver_count() > 0 {
            let _ = self.samples.send(Sample::new(values, via));
        }
    }

    /// Record a change made under `state` and signal it at once, unless one
    /// went within `window`: then the change waits, and the instant it is
    /// due is returned the first time.
    pub(super) fn changed(
        &self,
        state: &mut EntryState,
        seq: &AtomicU64,
        window: Duration,
    ) -> Option<Instant> {
        let now = Instant::now();
        state.snapshot.seq = seq.fetch_add(1, Ordering::Relaxed) + 1;
        match state.last_signal {
            Some(at) if now.saturating_duration_since(at) < window => {
                if state.pending {
                    state.snapshot.coalesced += 1;
                    None
                } else {
                    state.pending = true;
                    Some(at + window)
                }
            }
            _ => {
                if state.pending {
                    state.snapshot.coalesced += 1;
                }
                state.pending = false;
                state.last_signal = Some(now);
                self.signal.send_replace(state.snapshot.seq);
                post(state);
                None
            }
        }
    }

    /// Signal a change that waited for its window.
    pub(super) fn flush(&self) {
        let mut state = self.lock();
        if state.pending {
            state.pending = false;
            state.last_signal = Some(Instant::now());
            self.signal.send_replace(state.snapshot.seq);
            post(&mut state);
        }
        release(state);
    }

    /// Change the watch's state and note without a value change; signalled
    /// at once, since the state matters to listeners. A watch that has
    /// ended stays so: only [`Phase::Stopped`] follows [`Phase::Gone`].
    pub(super) fn set_state(
        &self,
        seq: &AtomicU64,
        source: Option<Source>,
        state: Phase,
        note: Option<String>,
    ) {
        let mut entry = self.lock();
        let leaves =
            entry.ended() && !(entry.snapshot.state == Phase::Gone && state == Phase::Stopped);
        if leaves {
            return;
        }
        let snapshot = &mut entry.snapshot;
        let new_source = source.is_some() && snapshot.source != source;
        if !new_source && snapshot.state == state && snapshot.note == note {
            return;
        }
        if source.is_some() {
            snapshot.source = source;
        }
        snapshot.state = state;
        snapshot.note = note;
        self.changed(&mut entry, seq, Duration::ZERO);
        release(entry);
    }

    /// The instance of the point's device, `found` when none was known.
    pub(super) fn learn(&self, found: Option<u32>) -> Option<u32> {
        let mut entry = self.lock();
        if entry.identity.is_none() {
            entry.identity = found;
        }
        entry.identity
    }

    /// The instance of the point's device, if known.
    pub(super) fn identity(&self) -> Option<u32> {
        self.lock().identity
    }

    /// Give the watch up `why`, unless it stopped: its notifications are
    /// no longer taken (nothing from its address is the point's now), and
    /// a point given up before takes the newer reason.
    pub(super) fn gone(&self, seq: &AtomicU64, why: Why, note: String) {
        let mut entry = self.lock();
        entry.target = None;
        let snapshot = &entry.snapshot;
        let same = snapshot.state == Phase::Gone && snapshot.why.as_ref() == Some(&why);
        if same || snapshot.state == Phase::Stopped {
            return;
        }
        // Values read before another device was found at the address may
        // be that device's: none is kept as the point's.
        if matches!(why, Why::Replaced { .. }) {
            entry.snapshot.rows.clear();
            entry.failure = None;
        }
        let snapshot = &mut entry.snapshot;
        snapshot.state = Phase::Gone;
        snapshot.why = Some(why);
        snapshot.note = Some(note);
        self.changed(&mut entry, seq, Duration::ZERO);
        release(entry);
    }

    /// Start the watch again after it was given up, its reason cleared:
    /// its device's address answers for its instance again.
    pub(super) fn revive(&self, seq: &AtomicU64) {
        let mut entry = self.lock();
        let snapshot = &mut entry.snapshot;
        snapshot.state = Phase::Starting;
        snapshot.source = None;
        snapshot.why = None;
        snapshot.note = None;
        self.changed(&mut entry, seq, Duration::ZERO);
        release(entry);
    }

    /// Values read (a first read, or a poll when `poll`): fold them into
    /// the rows, mark the watch live, signal at once, and sample a poll's.
    /// A first read's sample waits for its start's outcome
    /// ([`FirstSample`]). Nothing changes for a watch that has ended.
    ///
    /// A first read passes the instant it ended as `first_at`: a property a
    /// notification set at or after then keeps the notification's value
    /// (#2169). The read ended before its SubscribeCOV was sent, so a
    /// notification of that subscription can't be older, and a tie counts
    /// the read as older, as the TUI's writer does. A notification that
    /// came in while the read was out can only be from an earlier attempt's
    /// subscription; assuming the link delivers in order, the device sent
    /// it before its answer to the read, so the read wins. A datagram
    /// reordered past that answer can at worst leave the read's value
    /// shown until the next change. The notes are then dropped: a later
    /// start's read is newer than all of them.
    pub(super) fn read(
        &self,
        seq: &AtomicU64,
        rows: Vec<RawRow>,
        poll: bool,
        first_at: Option<Instant>,
    ) {
        let mut guard = self.lock();
        if guard.ended() {
            return;
        }
        let state = &mut *guard;
        match first_at {
            Some(at) => {
                let kept: Vec<RawRow> = rows
                    .iter()
                    .filter(|(p, _)| !state.notified.get(p).is_some_and(|n| *n >= at))
                    .cloned()
                    .collect();
                fold(&mut state.snapshot.rows, &kept);
                state.notified.clear();
            }
            None => fold(&mut state.snapshot.rows, &rows),
        }
        state.snapshot.updated = Some(Instant::now());
        state.wall = Some(SystemTime::now());
        state.snapshot.state = Phase::Live;
        state.snapshot.via = Some(if poll { Via::Poll } else { Via::First });
        state.failure = None;
        if poll {
            state.snapshot.polls += 1;
            state.snapshot.source = Some(Source::Poll);
        }
        self.changed(state, seq, Duration::ZERO);
        release(guard);
        if poll {
            self.sample(Ok(rows), Via::Poll);
        }
    }
}

/// A start's first-read sample, handed on when dropped: at the end of the
/// start's handling, once the snapshot shows its outcome (subscribed,
/// polled or given up), so a reader labelling the sample by the snapshot
/// sees the mode the read led to.
pub(super) struct FirstSample<'a> {
    pub(super) shared: &'a Shared,
    pub(super) values: Option<Result<Vec<RawRow>, String>>,
    /// When the read ended: the sample keeps that time, so a notification
    /// that came in meanwhile still sorts after it.
    pub(super) read_at: (Instant, SystemTime),
}

impl Drop for FirstSample<'_> {
    fn drop(&mut self) {
        if let Some(values) = self.values.take()
            && self.shared.samples.receiver_count() > 0
        {
            let (at, time) = self.read_at;
            let _ = self
                .shared
                .samples
                .send(Sample::at(values, Via::First, at, time));
        }
    }
}

/// Fold `read` into `rows`: each replaces the row of its property, and a
/// new property joins while there is room ([`MAX_ROWS`]).
pub(super) fn fold(rows: &mut Vec<RawRow>, read: &[RawRow]) {
    for (property, value) in read {
        if let Some(slot) = rows.iter_mut().find(|(p, _)| p == property) {
            slot.1 = value.clone();
        } else if rows.len() < MAX_ROWS {
            rows.push((*property, value.clone()));
        }
    }
}

/// A count of entries not yet finished, held by the entry's scheduler (or
/// its inbox) until the entry is done, cancel included, however it ends.
pub(super) struct Running(Arc<Registry>);

impl Running {
    pub(super) fn new(registry: &Arc<Registry>) -> Self {
        registry.tasks.send_modify(|tasks| *tasks += 1);
        Self(Arc::clone(registry))
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.0
            .tasks
            .send_modify(|tasks| *tasks = tasks.saturating_sub(1));
    }
}

/// Tell each live listener of a signalled change, dropping those whose
/// watch is gone; their wakers wait for [`release`].
fn post(state: &mut EntryState) {
    let EntryState {
        listeners, wakers, ..
    } = state;
    listeners.retain(|listener| listener.post(wakers));
}

/// An entry on its way to its device's scheduler.
pub(super) struct Arrival {
    pub(super) shared: Arc<Shared>,
    pub(super) running: Running,
}

/// Announcements, and senders, a scheduler keeps between two looks, each
/// once in a row: more mark announcements missed, and the scheduler reads
/// the table instead.
const HEARD_KEPT: usize = 8;

/// What the notification receiver noted at a device's address since its
/// scheduler last looked (#2603).
#[derive(Debug, Default)]
pub(super) struct Notes {
    /// Instances announced there, in order, each with whether its I-Am is
    /// newer than any seen there before: only a new one may be a restart,
    /// not a row the table moved when the client learned its network.
    pub(super) announced: Vec<(u32, bool)>,
    /// Announcements were missed: the receiver fell behind, or more came
    /// between two looks than are kept.
    pub(super) missed: bool,
    /// Instances COV notifications from the address named as their sender
    /// although their points' device is another.
    pub(super) senders: Vec<u32>,
    /// The device said it restarted (a restart notification), with its
    /// Time_Of_Device_Restart when the notification carried one.
    pub(super) restart: Option<Option<Vec<u8>>>,
    /// Instances given up here for another device at the address, heard
    /// announcing themselves at another address since.
    pub(super) elsewhere: Vec<u32>,
}

/// What the receiver keeps of a device's address.
#[derive(Debug)]
struct Heard {
    notes: Notes,
    /// The newest I-Am seen at the address, on the discovery table's
    /// clock: the slot's making at first.
    seen: std::time::Instant,
    /// The instance the address's notifications name as their sender in
    /// place of its device's, once a read showed the device there.
    alias: Option<u32>,
}

/// Push `value` onto `list` unless it ends it already, keeping at most
/// [`HEARD_KEPT`]; false when one had to go.
fn keep<V: PartialEq>(list: &mut Vec<V>, value: V) -> bool {
    if list.last() == Some(&value) {
        return true;
    }
    let room = list.len() < HEARD_KEPT;
    if !room {
        list.remove(0);
    }
    list.push(value);
    room
}

/// A device's scheduler, as the table knows it: entries not yet taken,
/// where a handle following its instance moved it to, what was announced
/// at its address, and its wake-up.
pub(super) struct DeviceSlot {
    pub(super) inbox: Mutex<Vec<Arrival>>,
    /// Set by the notification receiver when a handle following an
    /// instance moved it from the device's address
    /// ([`ClientEvent::Moved`]): the instance, and where it went.
    ///
    /// [`ClientEvent::Moved`]: crate::device::ClientEvent::Moved
    pub(super) moved: Mutex<Option<(u32, DeviceAddress)>>,
    /// What the notification receiver noted at the address.
    heard: Mutex<Heard>,
    pub(super) wake: Notify,
}

impl DeviceSlot {
    pub(super) fn new() -> Self {
        Self {
            inbox: Mutex::new(Vec::new()),
            moved: Mutex::new(None),
            heard: Mutex::new(Heard {
                notes: Notes::default(),
                seen: tokio::time::Instant::now().into_std(),
                alias: None,
            }),
            wake: Notify::new(),
        }
    }

    pub(super) fn inbox(&self) -> MutexGuard<'_, Vec<Arrival>> {
        self.inbox.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn heard(&self) -> MutexGuard<'_, Heard> {
        self.heard.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A device at the address announced itself as `instance`, by an I-Am
    /// the table took at `at`.
    fn hear(&self, instance: u32, at: Option<std::time::Instant>) {
        let mut heard = self.heard();
        let new = at.is_some_and(|at| at > heard.seen);
        if let Some(at) = at.filter(|_| new) {
            heard.seen = at;
        }
        let announced = &mut heard.notes.announced;
        match announced.last_mut() {
            Some((last, fresh)) if *last == instance => *fresh |= new,
            _ => {
                if !keep(announced, (instance, new)) {
                    heard.notes.missed = true;
                }
            }
        }
    }

    /// Announcements at the address may have been missed.
    fn miss(&self) {
        self.heard().notes.missed = true;
    }

    /// A notification from the address named `sender` as its device,
    /// another than its point's.
    fn sender(&self, sender: u32) {
        keep(&mut self.heard().notes.senders, sender);
    }

    /// The device at the address said it restarted.
    fn restarted(&self, time: Option<Vec<u8>>) {
        self.heard().notes.restart = Some(time);
    }

    /// `instance`, given up here, announced itself elsewhere.
    fn elsewhere(&self, instance: u32) {
        keep(&mut self.heard().notes.elsewhere, instance);
    }

    /// What was noted since the last look.
    pub(super) fn take_notes(&self) -> Notes {
        std::mem::take(&mut self.heard().notes)
    }

    /// The instance the address's notifications name in place of their
    /// device's, once a read showed that.
    pub(super) fn alias(&self) -> Option<u32> {
        self.heard().alias
    }

    /// Take `sender` as the address's notifications' sender name for its
    /// device from now on (`None` for none but the device's own).
    pub(super) fn set_alias(&self, sender: Option<u32>) {
        self.heard().alias = sender;
    }

    /// The instance a following handle moved from the device's address,
    /// and where to, once.
    pub(super) fn take_moved(&self) -> Option<(u32, DeviceAddress)> {
        self.moved
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

/// The entries, by point and by subscriber process identifier, and the
/// devices' schedulers, by address.
pub(super) struct Table {
    pub(super) entries: HashMap<Point, Slot>,
    pub(super) by_process: HashMap<u32, Arc<Shared>>,
    pub(super) devices: HashMap<DeviceAddress, Arc<DeviceSlot>>,
    /// The addresses whose points are given up for another device there,
    /// by the instance given up: one announcing itself anywhere else tells
    /// those schedulers to look for it there (#2603).
    pub(super) replaced: HashMap<u32, BTreeSet<DeviceAddress>>,
    pub(super) next_process: u32,
    pub(super) shut: bool,
}

pub(super) struct Slot {
    pub(super) shared: Arc<Shared>,
    pub(super) leases: usize,
}

/// What leases, schedulers and the dispatcher share without the client.
pub(super) struct Registry {
    pub(super) table: Mutex<Table>,
    pub(super) seq: AtomicU64,
    pub(super) lagged: AtomicU64,
    /// COV-multiple notifications the receiver left alone
    /// ([`HubStats::cov_multiple_ignored`](super::hub::HubStats::cov_multiple_ignored)).
    pub(super) cov_multiple_ignored: AtomicU64,
    /// Notifications dropped for naming another sender than their point's
    /// device (#2603).
    pub(super) foreign: AtomicU64,
    /// Claims of another device at an address that the device there
    /// refuted when asked.
    pub(super) refuted: AtomicU64,
    pub(super) coalesce: Duration,
    /// What an object watch keeps from a notification.
    pub(super) object_reads: Vec<PropertyIdentifier>,
    /// A property watch keeps a notification's Status_Flags too.
    pub(super) property_flags: bool,
    /// Entries not yet finished, those cancelling included.
    pub(super) tasks: watch::Sender<usize>,
    /// Cancels queued or running.
    pub(super) cancelling: AtomicUsize,
    /// The first subscriber process identifier of the hub's range.
    pub(super) process_base: u32,
    /// Runs once when a scheduler with nothing left is about to take the
    /// table's lock to end: a test lands an arrival in that window.
    #[cfg(test)]
    pub(super) before_end: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl Registry {
    pub(super) fn new(
        first_process: u32,
        coalesce: Duration,
        object_reads: Vec<PropertyIdentifier>,
        property_flags: bool,
    ) -> Self {
        Self {
            table: Mutex::new(Table {
                entries: HashMap::new(),
                by_process: HashMap::new(),
                devices: HashMap::new(),
                replaced: HashMap::new(),
                next_process: first_process,
                shut: false,
            }),
            seq: AtomicU64::new(0),
            lagged: AtomicU64::new(0),
            cov_multiple_ignored: AtomicU64::new(0),
            foreign: AtomicU64::new(0),
            refuted: AtomicU64::new(0),
            coalesce,
            object_reads,
            property_flags,
            tasks: watch::Sender::new(0),
            cancelling: AtomicUsize::new(0),
            process_base: first_process,
            #[cfg(test)]
            before_end: Mutex::new(None),
        }
    }

    pub(super) fn table(&self) -> MutexGuard<'_, Table> {
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// One lease on `shared` ends: the last one stops the entry and wakes
    /// its device's scheduler, which cancels what it holds.
    pub(super) fn release(&self, shared: &Arc<Shared>) {
        let mut table = self.table();
        let Some(slot) = table.entries.get_mut(&shared.point) else {
            return;
        };
        if !Arc::ptr_eq(&slot.shared, shared) {
            return;
        }
        slot.leases -= 1;
        if slot.leases == 0 {
            table.entries.remove(&shared.point);
            table.by_process.remove(&shared.process);
            let device = table.devices.get(&shared.point.device).cloned();
            drop(table);
            shared.stop();
            if let Some(device) = device {
                device.wake.notify_one();
            }
        }
    }

    /// A handle following `instance` moved it from `from` to `to`: tell
    /// the scheduler of the device at `from`, if it has one.
    pub(super) fn moved(&self, instance: u32, from: &DeviceAddress, to: DeviceAddress) {
        let device = self.table().devices.get(from).cloned();
        if let Some(device) = device {
            *device.moved.lock().unwrap_or_else(PoisonError::into_inner) = Some((instance, to));
            device.wake.notify_one();
        }
    }

    /// A device at `at` announced itself as `instance`, by an I-Am the
    /// table took at `when`: tell that address's scheduler, if it has one,
    /// and those that gave points up for `instance` at another address.
    pub(super) fn announced(
        &self,
        at: &DeviceAddress,
        instance: u32,
        when: Option<std::time::Instant>,
    ) {
        let (device, others) = {
            let table = self.table();
            let others: Vec<Arc<DeviceSlot>> = table
                .replaced
                .get(&instance)
                .into_iter()
                .flatten()
                .filter(|address| *address != at)
                .filter_map(|address| table.devices.get(address).cloned())
                .collect();
            (table.devices.get(at).cloned(), others)
        };
        if let Some(device) = device {
            device.hear(instance, when);
            device.wake.notify_one();
        }
        for other in others {
            other.elsewhere(instance);
            other.wake.notify_one();
        }
    }

    /// The device at `at` said it restarted.
    pub(super) fn restarted(&self, at: &DeviceAddress, time: Option<Vec<u8>>) {
        let device = self.table().devices.get(at).cloned();
        if let Some(device) = device {
            device.restarted(time);
            device.wake.notify_one();
        }
    }

    /// Points at `at` are given up for another device there: `instance`
    /// announcing itself elsewhere is told to `at`'s scheduler.
    pub(super) fn index_replaced(&self, instance: u32, at: &DeviceAddress) {
        let mut table = self.table();
        table
            .replaced
            .entry(instance)
            .or_default()
            .insert(at.clone());
    }

    /// `at` has no point given up for `instance` any more (`None`: for any).
    pub(super) fn unindex_replaced(&self, instance: Option<u32>, at: &DeviceAddress) {
        let mut table = self.table();
        table.replaced.retain(|known, addresses| {
            if instance.is_none_or(|instance| instance == *known) {
                addresses.remove(at);
            }
            !addresses.is_empty()
        });
    }

    /// Announcements may have been missed (the receiver fell behind): every
    /// scheduler reads the discovery table at its address again.
    pub(super) fn missed(&self) {
        let devices: Vec<Arc<DeviceSlot>> = self.table().devices.values().cloned().collect();
        for device in devices {
            device.miss();
            device.wake.notify_one();
        }
    }

    /// Apply a single-object COV notification to the entry it names; the
    /// instant a held-back signal is due, the first time one is held back.
    /// `own` is the client's network number, once known.
    pub(super) fn deliver(
        &self,
        event: &COVEvent,
        own: Option<u16>,
    ) -> Option<(Instant, Arc<Shared>)> {
        let received = event.object()?;
        let process = received.notification.subscriber_process_identifier;
        let (shared, device, alias) = {
            let table = self.table();
            let shared = Arc::clone(table.by_process.get(&process)?);
            let device = table.devices.get(&shared.point.device).cloned();
            let alias = device.as_ref().and_then(|device| device.alias());
            (shared, device, alias)
        };
        let mut state = shared.lock();
        if !dispatch::belongs(&shared.point, state.target.as_ref(), event, own) {
            return None;
        }
        // Every notification names the device that sent it (#2603): one
        // naming another than the point's device isn't the point's, under
        // its process identifier or not, unless a read showed the device
        // there names itself so. Its scheduler asks who answers there.
        let sender = received
            .notification
            .initiating_device_identifier
            .instance_number();
        if state
            .identity
            .is_some_and(|own| own != sender && alias != Some(sender))
        {
            drop(state);
            self.foreign.fetch_add(1, Ordering::Relaxed);
            if let Some(device) = device {
                device.sender(sender);
                device.wake.notify_one();
            }
            return None;
        }
        let merged = dispatch::merge(
            &mut state.snapshot.rows,
            &shared.point,
            &self.object_reads,
            self.property_flags,
            received,
        );
        let now = Instant::now();
        let watched = shared.point.property().0;
        for (property, _) in &merged {
            state.notified.insert(*property, now);
            if *property == watched {
                // A value of the watched property: the last read's error
                // no longer stands.
                state.failure = None;
            }
        }
        state.snapshot.notifications += 1;
        state.snapshot.updated = Some(now);
        state.wall = Some(SystemTime::now());
        state.snapshot.via = Some(Via::Notification);
        let due = shared.changed(&mut state, &self.seq, self.coalesce);
        release(state);
        if !merged.is_empty() {
            shared.sample(Ok(merged), Via::Notification);
        }
        due.map(|at| (at, shared))
    }
}
