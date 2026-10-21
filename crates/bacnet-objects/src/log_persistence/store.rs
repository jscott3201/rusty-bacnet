//! How a durable log keeps its storage: the cadence bookkeeping, the writer
//! thread its commits run on, and the hooks the three log objects share.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};

use super::codec::{self, SnapshotRecord};
use super::{LogAcquisitionSettings, LogCommitCadence, LogPersistence, LogSnapshot};
use crate::durable::{SaveWait, SaveWriter};
use crate::log_buffer::{LogRecordBuffer, ResidentLogRecord};
use crate::log_reporting::{records_between, BufferReadyReporting};
use crate::log_window::LogWindow;

/// Everything a snapshot holds but the records themselves, plus how many
/// records are resident: what tells whether the log changed since the last
/// commit.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LogState {
    buffer_size: u32,
    log_enable: bool,
    stop_when_full: bool,
    total_record_count: u32,
    record_count: usize,
    last_notify_record: u32,
    records_since_notification_from: u32,
    notification_threshold: u32,
    start_time: (Date, Time),
    stop_time: (Date, Time),
    acquisition: Option<LogAcquisitionSettings>,
    log_device_object_property: Vec<BACnetDeviceObjectPropertyReference>,
}

impl LogState {
    /// The state of a log held in these parts; the trend logs add their
    /// acquisition settings and references with [`trend`](Self::trend).
    pub(crate) fn of<R: ResidentLogRecord>(
        buffer: &LogRecordBuffer<R>,
        log_enable: bool,
        stop_when_full: bool,
        window: &LogWindow,
        reporting: &BufferReadyReporting,
    ) -> Self {
        let (start_time, stop_time) = window.ends();
        let (last_notify_record, records_since_notification_from) = reporting.counts();
        Self {
            buffer_size: buffer.capacity(),
            log_enable,
            stop_when_full,
            total_record_count: buffer.total_record_count(),
            record_count: buffer.records().len(),
            last_notify_record,
            records_since_notification_from,
            notification_threshold: reporting.notification_threshold(),
            start_time,
            stop_time,
            acquisition: None,
            log_device_object_property: Vec::new(),
        }
    }

    /// Add a trend log's acquisition settings and references.
    pub(crate) fn trend(
        mut self,
        acquisition: LogAcquisitionSettings,
        references: Vec<BACnetDeviceObjectPropertyReference>,
    ) -> Self {
        self.acquisition = Some(acquisition);
        self.log_device_object_property = references;
        self
    }

    fn snapshot<R: Clone>(&self, oid: ObjectIdentifier, records: &VecDeque<R>) -> LogSnapshot<R> {
        LogSnapshot {
            object_identifier: oid,
            // The writer numbers it as it commits.
            generation: 0,
            buffer_size: self.buffer_size,
            log_enable: self.log_enable,
            stop_when_full: self.stop_when_full,
            total_record_count: self.total_record_count,
            records: records.iter().cloned().collect(),
            last_notify_record: self.last_notify_record,
            records_since_notification_from: self.records_since_notification_from,
            notification_threshold: self.notification_threshold,
            start_time: self.start_time,
            stop_time: self.stop_time,
            acquisition: self.acquisition,
            log_device_object_property: self.log_device_object_property.clone(),
        }
    }
}

/// One durable log's storage: the writer its commits run on and what the
/// cadence needs to decide the next one.
pub(crate) struct LogStore<R: SnapshotRecord> {
    oid: ObjectIdentifier,
    writer: SaveWriter<Mutex<LogSnapshot<R>>>,
    /// The generation of the last snapshot storage took.
    generation: Arc<AtomicU64>,
    cadence: LogCommitCadence,
    /// The state of the last snapshot queued, or loaded; `None` before
    /// either.
    queued: Option<LogState>,
    /// Whether the last commit the writer ran failed. The writer's done
    /// hook sets it after each commit, so it tells the outcome of the latest
    /// one; while it is set the log counts as changed, and the next interval
    /// pass, or the final commit, queues its state again.
    last_failed: Arc<AtomicBool>,
    /// Total_Record_Count in that snapshot.
    queued_total: u32,
    /// The monotonic time the commit interval counts from, once the first
    /// look has set it.
    interval_from: Option<Duration>,
    /// A commit was queued since the last look: the next look restarts the
    /// interval.
    restart_interval: bool,
    reopened: bool,
    interruption_pending: bool,
}

impl<R: SnapshotRecord> LogStore<R> {
    /// Load the log's snapshot from `persistence`, if it holds one, and set
    /// up the writer its commits go through. The caller applies the snapshot
    /// to the log and then calls [`reopened`](Self::reopened).
    ///
    /// `buffer_size` is the log's configured Buffer_Size; one whose
    /// worst-case snapshot could outgrow a slot is refused before anything
    /// is loaded. The loaded snapshot is checked again here, as a guard for
    /// backends other than [`FileLogPersistence`](super::FileLogPersistence),
    /// which checks it as it reads each slot.
    pub(crate) fn open(
        oid: ObjectIdentifier,
        buffer_size: u32,
        persistence: Arc<dyn LogPersistence<R>>,
    ) -> Result<(Self, Option<LogSnapshot<R>>), Error> {
        codec::check_buffer_size::<R>(buffer_size)?;
        let loaded = persistence.load(oid)?;
        if let Some(snapshot) = &loaded {
            if snapshot.object_identifier != oid {
                return Err(Error::Encoding(
                    "log snapshot belongs to another object".into(),
                ));
            }
            codec::validate(snapshot)?;
        }
        let last_failed = Arc::new(AtomicBool::new(false));
        let failed = Arc::clone(&last_failed);
        let generation = Arc::new(AtomicU64::new(
            loaded.as_ref().map_or(0, |snapshot| snapshot.generation),
        ));
        let committed = Arc::clone(&generation);
        let writer = SaveWriter::new(
            format!(
                "bacnet-log-{}-{}-save",
                oid.object_type().to_raw(),
                oid.instance_number()
            ),
            move |pending: &Mutex<LogSnapshot<R>>| {
                let mut snapshot = pending.lock().unwrap_or_else(PoisonError::into_inner);
                snapshot.generation = committed
                    .load(Ordering::Acquire)
                    .checked_add(1)
                    .ok_or_else(|| Error::OutOfRange("log snapshot generation exhausted".into()))?;
                persistence.commit(&snapshot)?;
                committed.store(snapshot.generation, Ordering::Release);
                Ok(())
            },
            move |_, result| {
                failed.store(result.is_err(), Ordering::Release);
                if let Err(error) = result {
                    tracing::warn!(
                        log = %oid,
                        %error,
                        "Failed to commit a log snapshot; the next pass or stop tries again"
                    );
                }
            },
        );
        Ok((
            Self {
                oid,
                writer,
                generation,
                cadence: LogCommitCadence::default(),
                queued: None,
                last_failed,
                queued_total: 0,
                interval_from: None,
                restart_interval: false,
                reopened: false,
                interruption_pending: false,
            },
            loaded,
        ))
    }

    /// The log took its loaded snapshot and now holds `state`, and a
    /// LOG_INTERRUPTED record is owed. `resized` says the configured
    /// Buffer_Size replaced the saved one, so storage no longer matches.
    pub(crate) fn reopened(&mut self, state: LogState, resized: bool) {
        self.queued_total = state.total_record_count;
        self.queued = (!resized).then_some(state);
        self.reopened = true;
        self.interruption_pending = true;
    }

    pub(crate) fn is_reopened(&self) -> bool {
        self.reopened
    }

    pub(crate) fn interruption_pending(&self) -> bool {
        self.interruption_pending
    }

    /// The LOG_INTERRUPTED record is in, from the log or the application.
    pub(crate) fn interruption_recorded(&mut self) {
        self.interruption_pending = false;
    }

    pub(crate) fn cadence(&self) -> LogCommitCadence {
        self.cadence
    }

    pub(crate) fn set_cadence(&mut self, cadence: LogCommitCadence) {
        self.cadence = cadence;
    }

    /// The generation of the last snapshot storage took: the loaded one's
    /// until a commit lands, zero for storage that held none.
    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Whether Total_Record_Count `total` is the cadence's record count past
    /// the last commit.
    pub(crate) fn count_due(&self, total: u32) -> bool {
        self.cadence.records > 0
            && records_between(self.queued_total, total) >= self.cadence.records
    }

    /// Whether the commit interval has run out at monotonic time `now`. The
    /// first look, and the first after a commit, start it instead.
    pub(crate) fn interval_due(&mut self, now: Duration) -> bool {
        match self.interval_from {
            Some(from) if !self.restart_interval => {
                !self.cadence.interval.is_zero()
                    && now.saturating_sub(from) >= self.cadence.interval
            }
            _ => {
                self.interval_from = Some(now);
                self.restart_interval = false;
                false
            }
        }
    }

    /// Whether `state` differs from the last snapshot queued or loaded, or
    /// the last commit failed, so storage may hold an older state.
    pub(crate) fn changed(&self, state: &LogState) -> bool {
        self.last_failed.load(Ordering::Acquire) || self.queued.as_ref() != Some(state)
    }

    /// Queue a commit of `state` with `records` on the writer thread. A
    /// commit still waiting to start is replaced by this one.
    pub(crate) fn queue(&mut self, state: LogState, records: &VecDeque<R>) {
        let snapshot = state.snapshot(self.oid, records);
        self.writer.submit_coalescing(Mutex::new(snapshot));
        self.queued_total = state.total_record_count;
        self.queued = Some(state);
        self.restart_interval = true;
    }

    /// A wait that ends once every commit queued so far has run.
    pub(crate) fn idle(&self) -> SaveWait {
        self.writer.idle()
    }
}

/// Fit a loaded snapshot to the Buffer_Size the application configured,
/// which wins over the saved one: peers can't write it, and Clause 12.25.13
/// leaves what happens to the records on a resize to the device. Returns
/// whether the snapshot changed.
///
/// A larger size keeps every record. A smaller one keeps the newest records
/// that fit; their sequence numbers, Total_Record_Count and the
/// BUFFER_READY counts, which all follow Total_Record_Count, stay as they
/// were. A log with Stop_When_Full and Enable both TRUE keeps one slot fewer,
/// since such a log stops before its last slot is taken, so its next record
/// stops it with the usual LOG_DISABLED record.
pub(crate) fn fit_to_buffer_size<R>(snapshot: &mut LogSnapshot<R>, buffer_size: u32) -> bool {
    if snapshot.buffer_size == buffer_size {
        return false;
    }
    let stops_when_full = snapshot.log_enable && snapshot.stop_when_full;
    let room = if stops_when_full {
        (buffer_size as usize).saturating_sub(1)
    } else {
        buffer_size as usize
    };
    let dropped = snapshot.records.len().saturating_sub(room);
    tracing::warn!(
        log = %snapshot.object_identifier,
        saved = snapshot.buffer_size,
        configured = buffer_size,
        dropped,
        "Reopening a log at the configured Buffer_Size, not the saved one"
    );
    snapshot.records.drain(..dropped);
    snapshot.buffer_size = buffer_size;
    true
}

/// The parts of a log object its storage works with. Each log implements it
/// and gets the cadence hooks below.
pub(crate) trait DurableLog {
    /// The log's record family.
    type Record: SnapshotRecord;

    /// The state a commit would save now.
    fn log_state(&self) -> LogState;

    /// The storage, if the log has one, and the resident records, borrowed
    /// together.
    fn durable_parts(
        &mut self,
    ) -> (
        Option<&mut LogStore<Self::Record>>,
        &LogRecordBuffer<Self::Record>,
    );

    /// Append the owed LOG_INTERRUPTED record, stamped with the clock's time
    /// or, without a valid clock, `fallback`; returns whether it went in.
    fn record_owed_interruption(&mut self, fallback: Option<(Date, Time)>) -> bool;

    /// Record the owed LOG_INTERRUPTED, if any, at the clock's time or else
    /// at `fallback`, the timestamp of a record about to land.
    fn settle_interruption(&mut self, fallback: Option<(Date, Time)>) {
        let owed = matches!(self.durable_parts().0, Some(store) if store.interruption_pending());
        if owed && self.record_owed_interruption(fallback) {
            if let Some(store) = self.durable_parts().0 {
                store.interruption_recorded();
            }
            self.commit_if_counted();
        }
    }

    /// Commit when the records since the last commit reach the cadence's
    /// count.
    fn commit_if_counted(&mut self) {
        let due = {
            let (store, buffer) = self.durable_parts();
            store.is_some_and(|store| store.count_due(buffer.total_record_count()))
        };
        if due {
            let state = self.log_state();
            if let (Some(store), buffer) = self.durable_parts() {
                store.queue(state, buffer.records());
            }
        }
    }

    /// The server's monotonic pass at `now`: commit when the interval has
    /// run out and the state changed.
    fn commit_if_interval(&mut self, now: Duration) {
        self.settle_interruption(None);
        let due = self
            .durable_parts()
            .0
            .is_some_and(|store| store.interval_due(now));
        if due {
            self.commit_if_changed();
        }
    }

    /// Commit when the state changed since the last commit.
    fn commit_if_changed(&mut self) {
        if self.durable_parts().0.is_none() {
            return;
        }
        let state = self.log_state();
        if let (Some(store), buffer) = self.durable_parts() {
            if store.changed(&state) {
                store.queue(state, buffer.records());
            }
        }
    }

    /// The final commit, as the server stops or the log drops: commit what
    /// changed, and return the wait for every queued commit, or `None`
    /// without storage.
    fn commit_final(&mut self) -> Option<SaveWait> {
        self.commit_if_changed();
        self.durable_parts().0.map(|store| store.idle())
    }
}

/// Implement [`DurableWrites`](crate::durable::DurableWrites) and `Drop` for
/// a [`DurableLog`]: the server's stop commits it once more and waits for
/// the commit, and so does dropping it. Nothing is ever staged: a log's
/// commits don't decide any request.
macro_rules! impl_durable_log {
    ($log:ty) => {
        impl $crate::durable::DurableWrites for $log {
            fn stage_write(
                &mut self,
                _property: bacnet_types::enums::PropertyIdentifier,
                _array_index: Option<u32>,
                _value: &bacnet_types::primitives::PropertyValue,
            ) -> $crate::durable::StageStep {
                $crate::durable::StageStep::Skip
            }

            fn release_staged_write(&mut self, _staged: &$crate::durable::SaveWait) {}

            fn settle_forgotten_writes(&mut self) -> Option<$crate::durable::SaveWait> {
                $crate::log_persistence::store::DurableLog::commit_final(self)
            }
        }

        impl Drop for $log {
            fn drop(&mut self) {
                // The writer's own drop, which follows, waits for it.
                let _ = $crate::log_persistence::store::DurableLog::commit_final(self);
            }
        }
    };
}

pub(crate) use impl_durable_log;
