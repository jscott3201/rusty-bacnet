//! What a point's leases, its device's scheduler and the notification
//! receiver share: the entry's snapshot and signals, and the hub's table
//! of entries and devices. Every lock here is a std mutex held for a few
//! field updates, never across an await. The order is the table's, then a
//! device's inbox or what it heard ([`slot`](super::slot)), then an
//! entry's, then a [`Watch`](super::Watch)'s mailbox
//! ([`control`](super::control)), which
//! takes no other; the registry's list of watches is a leaf. A watch is
//! woken only once the entry's lock is let go ([`release`]).

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::task::Waker;
use std::time::{Duration, SystemTime};

use bacnet_client::client::{COVEvent, Station};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::sync::MutexExt;
use tokio::sync::{broadcast, watch};
use tokio::time::Instant;

use super::control::{Core, Listener};
use super::hub::{Phase, Snapshot, Source};
use super::options::MAX_ROWS;
use super::sample::{Sample, Via};
pub(super) use super::slot::{Alias, DeviceSlot};
use super::{Moved, Point, Why, dispatch};
use crate::device::{DeviceAddress, FollowKey, Followed};
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
    /// The address the point is watched at now: a pinned point's own, or
    /// where the hub follows a following point's device (#2795). Changed
    /// only under the table's lock, with the scheduler holding the point.
    pub(super) at: DeviceAddress,
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
    /// The instance of the point's device (#2603): what the device last
    /// named itself to the client, else what the discovery table named at
    /// its address when the watch started, else the first sender its
    /// notifications named.
    pub(super) identity: Option<u32>,
    /// The device itself named `identity` during this watch: a
    /// notification's sender, a poll's or a read's answer, or what it last
    /// said to the client. Until then the instance is only the table's,
    /// and the device's first word of another replaces it; after, another
    /// instance at the address is another device. A following point's
    /// instance is its own and never replaced: another is another device
    /// (`fixed`).
    pub(super) confirmed: bool,
    /// The point follows `identity`, its own instance (#2795).
    fixed: bool,
    /// The note the point had before a claim of another device at its
    /// address put its own: given back once the claim is settled.
    held: Option<Option<String>>,
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
    /// The entry of `point`, watched at `at` (its own address, or where its
    /// followed device is): a following point's device is its instance
    /// from the start, unconfirmed until the device names itself so.
    pub(super) fn new(point: Point, process: u32, samples: usize, at: DeviceAddress) -> Self {
        let follows = point.device.instance();
        Self {
            point,
            process,
            state: Mutex::new(EntryState {
                snapshot: Snapshot::new(),
                at,
                target: None,
                notified: HashMap::new(),
                last_signal: None,
                pending: false,
                failure: None,
                wall: None,
                identity: follows,
                confirmed: false,
                fixed: follows.is_some(),
                held: None,
                listeners: Vec::new(),
                wakers: Vec::new(),
            }),
            signal: watch::Sender::new(0),
            samples: broadcast::Sender::new(samples),
            stop: watch::Sender::new(false),
        }
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, EntryState> {
        self.state.locked()
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

    /// The address the point is watched at now.
    pub(super) fn at(&self) -> DeviceAddress {
        self.lock().at.clone()
    }

    /// The instance of the point's device: `found` when none was known, or
    /// when the device itself named `found` (`confirmed`, an answer to a
    /// request) and nothing it answered named the point's device before. A
    /// following point's own instance is never replaced.
    pub(super) fn learn(&self, found: Option<u32>, confirmed: bool) -> Option<u32> {
        let mut entry = self.lock();
        let take = match (entry.identity, found) {
            (_, None) => false,
            // Its own instance: the device's word confirms it, and no other
            // replaces it.
            (Some(own), Some(found)) if entry.fixed => {
                entry.confirmed |= confirmed && own == found;
                false
            }
            (None, Some(_)) => true,
            (Some(_), Some(_)) => confirmed && !entry.confirmed,
        };
        if take {
            entry.identity = found;
            entry.confirmed = confirmed;
        }
        entry.identity
    }

    /// The instance of the point's device, if known.
    pub(super) fn identity(&self) -> Option<u32> {
        self.lock().identity
    }

    /// The point's device, when the device itself named it in an answer.
    pub(super) fn confirmed(&self) -> Option<u32> {
        let entry = self.lock();
        entry.identity.filter(|_| entry.confirmed)
    }

    /// The device at the address named itself `found`: the point's device
    /// from now on unless the device itself had named another for it, or
    /// the point follows another, which is returned (another device is
    /// there now).
    pub(super) fn named(&self, found: u32, confirm: bool) -> Option<u32> {
        let mut entry = self.lock();
        match entry.identity {
            Some(own) if own != found && (entry.confirmed || entry.fixed) => Some(own),
            own => {
                if own != Some(found) {
                    entry.confirmed = false;
                }
                entry.identity = Some(found);
                entry.confirmed |= confirm;
                None
            }
        }
    }

    /// Note why the point's values may be held, keeping the note it had,
    /// unless one is held already.
    pub(super) fn doubt(&self, seq: &AtomicU64, note: String) {
        let mut entry = self.lock();
        if entry.held.is_some() || entry.ended() {
            return;
        }
        entry.held = Some(entry.snapshot.note.replace(note));
        self.changed(&mut entry, seq, Duration::ZERO);
        release(entry);
    }

    /// Give back the note a doubt replaced.
    pub(super) fn undoubt(&self, seq: &AtomicU64) {
        let mut entry = self.lock();
        let Some(note) = entry.held.take() else {
            return;
        };
        if !entry.ended() {
            entry.snapshot.note = note;
            self.changed(&mut entry, seq, Duration::ZERO);
        }
        release(entry);
    }

    /// Give the watch up `why` at its address, unless it stopped: its
    /// notifications are no longer taken (nothing from its address is the
    /// point's now), and a point given up before takes the newer reason. A
    /// pinned point is [`Phase::Gone`]; a following one
    /// [`Phase::Unreachable`] while the hub looks for its device (#2795).
    pub(super) fn gone(&self, seq: &AtomicU64, why: Why, note: String) {
        let phase = match self.point.device.instance() {
            Some(_) => Phase::Unreachable,
            None => Phase::Gone,
        };
        let mut entry = self.lock();
        entry.target = None;
        let snapshot = &entry.snapshot;
        let same = snapshot.state == phase && snapshot.why.as_ref() == Some(&why);
        if same || snapshot.state == Phase::Stopped {
            return;
        }
        // Values read before another device was found at the address may
        // be that device's: none is kept as the point's.
        if matches!(why, Why::Replaced { .. }) {
            entry.snapshot.rows.clear();
            entry.failure = None;
        }
        // A claim's note held back is no longer the point's.
        entry.held = None;
        let snapshot = &mut entry.snapshot;
        snapshot.state = phase;
        snapshot.why = Some(why);
        snapshot.note = Some(note);
        self.changed(&mut entry, seq, Duration::ZERO);
        release(entry);
    }

    /// The point, placed at its device's old address but never started
    /// there, starts at `to`, where the hub follows its device now (#2795).
    /// Called under the table's lock.
    pub(super) fn rehome(&self, to: &DeviceAddress) {
        self.lock().at = to.clone();
    }

    /// The hub follows the point's device from `from` to `to` (#2795):
    /// nothing from `from` is the point's from now on, and it starts again
    /// at `to`, keeping its values (the same device) and its subscriber
    /// process identifier. Signalled at once. Called under the table's
    /// lock.
    pub(super) fn handed(&self, seq: &AtomicU64, from: &DeviceAddress, to: &DeviceAddress) {
        let mut entry = self.lock();
        if entry.snapshot.state == Phase::Stopped {
            return;
        }
        entry.at = to.clone();
        entry.target = None;
        entry.held = None;
        entry.confirmed = false;
        entry.notified.clear();
        let moved = Moved {
            from: from.clone(),
            to: to.clone(),
            when: SystemTime::now(),
            count: entry
                .snapshot
                .moved
                .as_ref()
                .map_or(1, |moved| moved.count + 1),
        };
        let snapshot = &mut entry.snapshot;
        snapshot.state = Phase::Starting;
        snapshot.source = None;
        snapshot.why = None;
        snapshot.note = Some(format!("the device answers at {to} now, moved from {from}"));
        snapshot.moved = Some(moved);
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

/// Where the hub follows one instance on one network (#2795), for its
/// following points: new ones start there, and a move its own reads bore
/// out takes them all, and the client's handles, to the new address.
pub(super) struct Follow {
    /// Where the hub watches the device now.
    pub(super) at: DeviceAddress,
    /// The place the client's handles following it share, held while the
    /// hub follows it, so the handles' way of looking for it (the paced
    /// Who-Is) outlives the application's handles.
    pub(super) place: Followed,
    /// Following points of it watched.
    entries: usize,
}

/// The entries, by point and by subscriber process identifier, and the
/// devices' schedulers, by address.
pub(super) struct Table {
    pub(super) entries: HashMap<Point, Slot>,
    pub(super) by_process: HashMap<u32, Arc<Shared>>,
    pub(super) devices: HashMap<DeviceAddress, Arc<DeviceSlot>>,
    /// Where the hub follows each instance its following points follow.
    pub(super) follows: HashMap<FollowKey, Follow>,
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
    /// The key it follows its device by, for a following point.
    pub(super) follows: Option<FollowKey>,
}

impl Table {
    /// A following point of `key` starts: where the hub follows its
    /// device, from `new` (the client's handles' place, and where it
    /// reaches the device) when it follows it for no other point.
    pub(super) fn follow(
        &mut self,
        key: FollowKey,
        new: impl FnOnce() -> (Followed, DeviceAddress),
    ) -> DeviceAddress {
        let follow = self.follows.entry(key).or_insert_with(|| {
            let (place, at) = new();
            Follow {
                at,
                place,
                entries: 0,
            }
        });
        follow.entries += 1;
        follow.at.clone()
    }

    /// A following point of `key` ended: the last lets the place go.
    fn unfollow(&mut self, key: FollowKey) {
        if let Some(follow) = self.follows.get_mut(&key) {
            follow.entries = follow.entries.saturating_sub(1);
            if follow.entries == 0 {
                self.follows.remove(&key);
            }
        }
    }
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
    /// The [`Watch`](super::Watch)es taken from the hub, which its
    /// shutdown ends. A leaf lock, held only to add or take them.
    pub(super) watches: Mutex<Vec<Weak<Core>>>,
    /// Runs once when a scheduler with nothing left is about to take the
    /// table's lock to end: a test lands an arrival in that window.
    #[cfg(test)]
    pub(super) before_end: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    /// Runs once when a scheduler is about to hand its following points
    /// on for a move: a test adds a point in that window.
    #[cfg(test)]
    pub(super) before_move: Mutex<Option<Box<dyn FnOnce() + Send>>>,
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
                follows: HashMap::new(),
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
            watches: Mutex::new(Vec::new()),
            #[cfg(test)]
            before_end: Mutex::new(None),
            #[cfg(test)]
            before_move: Mutex::new(None),
        }
    }

    pub(super) fn table(&self) -> MutexGuard<'_, Table> {
        self.table.locked()
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
            let follows = slot.follows;
            table.entries.remove(&shared.point);
            table.by_process.remove(&shared.process);
            if let Some(key) = follows {
                table.unfollow(key);
            }
            // Where the point is watched now: its scheduler there takes it
            // or hands it on under this lock, so the address and the
            // scheduler agree.
            let device = table.devices.get(&shared.at()).cloned();
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
            *device.moved.locked() = Some((instance, to));
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
        if let Some(device) = device
            && device.hear(instance, when)
        {
            device.wake.notify_one();
        }
        for other in others {
            other.elsewhere(instance);
            other.wake.notify_one();
        }
    }

    /// The device at `at`, `instance`, said it restarted.
    pub(super) fn restarted(&self, at: &DeviceAddress, instance: u32) {
        let device = self.table().devices.get(at).cloned();
        if let Some(device) = device.filter(|device| device.restarted(instance)) {
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
        let (shared, device) = {
            let table = self.table();
            let shared = Arc::clone(table.by_process.get(&process)?);
            let device = table.devices.get(&shared.at()).cloned();
            (shared, device)
        };
        let identity = {
            let state = shared.lock();
            if !dispatch::belongs(&shared.point, state.target.as_ref(), event, own) {
                return None;
            }
            state.identity
        };
        // Every notification names the device that sent it (#2603), and
        // none was asked for: one naming the point's device, or the name a
        // read showed that device uses ([`Alias`]), is the point's; with no
        // device known at all, its sender is taken, unconfirmed. Any other
        // isn't the point's, under its process identifier or not: dropped,
        // and its scheduler asks the device there which it is, in time.
        // Only that answer may change or confirm the point's device.
        let sender = received.notification.initiating_device_identifier;
        let named = sender.instance_number();
        let alias = match (&device, identity) {
            (Some(device), Some(own)) => device.takes(own, named),
            _ => false,
        };
        let ours = identity == Some(named) || alias;
        let adopt = identity.is_none();
        if sender.object_type() != ObjectType::DEVICE || !(ours || adopt) {
            self.foreign.fetch_add(1, Ordering::Relaxed);
            if let Some(device) = device.filter(|device| device.sender(named)) {
                device.wake.notify_one();
            }
            return None;
        }
        let mut state = shared.lock();
        if state.identity != identity
            || !dispatch::belongs(&shared.point, state.target.as_ref(), event, own)
        {
            return None;
        }
        if adopt {
            state.identity = Some(named);
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
