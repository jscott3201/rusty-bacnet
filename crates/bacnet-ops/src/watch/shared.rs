//! What a point's leases, its device's scheduler and the notification
//! receiver share: the entry's snapshot and signals, and the hub's table
//! of entries and devices. Every lock here is a std mutex held for a few
//! field updates, never across an await. The order is the table's, then a
//! device inbox's, then an entry's.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use bacnet_client::client::{CovTarget, ReceivedCOVNotification};
use bacnet_types::enums::PropertyIdentifier;
use tokio::sync::{Notify, broadcast, watch};
use tokio::time::Instant;

use super::options::MAX_ROWS;
use super::sample::{Sample, Via};
use super::{Point, Snapshot, Source, State, dispatch};
use crate::format::RawRow;

/// One entry's state, shared by its scheduler, its leases and the
/// dispatcher.
pub(super) struct Shared {
    pub(super) point: Point,
    pub(super) process: u32,
    state: Mutex<EntryState>,
    /// Carries the hub's counter at each change it signals.
    pub(super) signal: watch::Sender<u64>,
    /// Every value the point gets, for [`Lease::samples`](super::Lease::samples).
    pub(super) samples: broadcast::Sender<Sample>,
    /// Set once to stop the entry.
    stop: watch::Sender<bool>,
}

pub(super) struct EntryState {
    pub(super) snapshot: Snapshot,
    /// Where the device is, once looked up: notifications must come from
    /// there.
    pub(super) target: Option<CovTarget>,
    last_signal: Option<Instant>,
    /// A change waits for the end of the coalescing window.
    pending: bool,
}

impl Shared {
    pub(super) fn new(point: Point, process: u32, samples: usize) -> Self {
        Self {
            point,
            process,
            state: Mutex::new(EntryState {
                snapshot: Snapshot::new(),
                target: None,
                last_signal: None,
                pending: false,
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
        }
    }

    /// Change the watch's state and note without a value change; signalled
    /// at once, since the state matters to listeners.
    pub(super) fn set_state(
        &self,
        seq: &AtomicU64,
        source: Option<Source>,
        state: State,
        note: Option<String>,
    ) {
        let mut entry = self.lock();
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
    }

    /// Values read (a first read, or a poll when `poll`): fold them into
    /// the rows, mark the watch live, signal at once, and sample a poll's.
    /// A first read's sample waits for its start's outcome
    /// ([`FirstSample`]).
    pub(super) fn read(&self, seq: &AtomicU64, rows: Vec<RawRow>, poll: bool) {
        let mut state = self.lock();
        fold(&mut state.snapshot.rows, &rows);
        state.snapshot.updated = Some(Instant::now());
        state.snapshot.state = State::Live;
        if poll {
            state.snapshot.polls += 1;
            state.snapshot.source = Some(Source::Poll);
        }
        self.changed(&mut state, seq, Duration::ZERO);
        drop(state);
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

/// An entry on its way to its device's scheduler.
pub(super) struct Arrival {
    pub(super) shared: Arc<Shared>,
    pub(super) running: Running,
}

/// A device's scheduler, as the table knows it: entries not yet taken, and
/// its wake-up.
pub(super) struct DeviceSlot {
    pub(super) inbox: Mutex<Vec<Arrival>>,
    pub(super) wake: Notify,
}

impl DeviceSlot {
    pub(super) fn inbox(&self) -> MutexGuard<'_, Vec<Arrival>> {
        self.inbox.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The entries, by point and by subscriber process identifier, and the
/// devices' schedulers.
pub(super) struct Table {
    pub(super) entries: HashMap<Point, Slot>,
    pub(super) by_process: HashMap<u32, Arc<Shared>>,
    pub(super) devices: HashMap<u32, Arc<DeviceSlot>>,
    pub(super) next_process: u32,
    pub(super) shut: bool,
}

pub(super) struct Slot {
    pub(super) shared: Arc<Shared>,
    pub(super) leases: usize,
}

/// What leases, schedulers and the dispatcher share without the engine.
pub(super) struct Registry {
    pub(super) table: Mutex<Table>,
    pub(super) seq: AtomicU64,
    pub(super) lagged: AtomicU64,
    pub(super) coalesce: Duration,
    /// What an object watch keeps from a notification.
    pub(super) object_reads: Vec<PropertyIdentifier>,
    /// A property watch keeps a notification's Status_Flags too.
    pub(super) property_flags: bool,
    /// Entries not yet finished, those cancelling included.
    pub(super) tasks: watch::Sender<usize>,
    /// Cancels queued or running.
    pub(super) cancelling: AtomicUsize,
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
                next_process: first_process,
                shut: false,
            }),
            seq: AtomicU64::new(0),
            lagged: AtomicU64::new(0),
            coalesce,
            object_reads,
            property_flags,
            tasks: watch::Sender::new(0),
            cancelling: AtomicUsize::new(0),
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

    /// Apply a COV notification to the entry it names; the instant a
    /// held-back signal is due, the first time one is held back.
    pub(super) fn deliver(
        &self,
        received: &ReceivedCOVNotification,
    ) -> Option<(Instant, Arc<Shared>)> {
        let process = received.notification.subscriber_process_identifier;
        let shared = Arc::clone(self.table().by_process.get(&process)?);
        let mut state = shared.lock();
        if !dispatch::belongs(&shared.point, state.target.as_ref(), received) {
            return None;
        }
        let merged = dispatch::merge(
            &mut state.snapshot.rows,
            &shared.point,
            &self.object_reads,
            self.property_flags,
            received,
        );
        state.snapshot.notifications += 1;
        state.snapshot.updated = Some(Instant::now());
        let due = shared.changed(&mut state, &self.seq, self.coalesce);
        drop(state);
        if !merged.is_empty() {
            shared.sample(Ok(merged), Via::Notification);
        }
        due.map(|at| (at, shared))
    }
}
