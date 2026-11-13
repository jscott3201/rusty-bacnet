//! How a durable log keeps its storage: the cadence bookkeeping, the writer
//! thread its commits run on, the staged writes of its reporting settings
//! ([`staging`]), and the hooks the three log objects share.

use crate::prelude::*;
use alloc::collections::VecDeque;
use core::sync::atomic::{AtomicBool, Ordering};

use alloc::sync::Arc;

use crate::sync::{AtomicU64, Mutex};
use core::time::Duration;

use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};

use super::codec::{self, SnapshotRecord};
use super::{
    LogAcquisitionSettings, LogCommitCadence, LogPersistence, LogReportingSettings, LogSnapshot,
};
use crate::durable::staged::StagedSaves;
use crate::durable::{SaveWait, SaveWriter};
use crate::log_buffer::{LogRecordBuffer, ResidentLogRecord};
use crate::log_reporting::{BufferReadyReporting, records_between};
use crate::log_window::LogWindow;
use crate::traits::BACnetObject;
use crate::trend::CovSettings;

pub(crate) mod staging;

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
    reporting: LogReportingSettings,
}

impl LogState {
    /// The state of a log held in these parts; the trend logs add their
    /// acquisition settings and references with [`trend`](Self::trend), and
    /// a Trend Log its COV settings with [`cov`](Self::cov).
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
            reporting: reporting.saved(),
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

    /// Add the COV settings a write set on a Trend Log.
    pub(crate) fn cov(mut self, cov: &CovSettings) -> Self {
        cov.save_into(&mut self.reporting);
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
            reporting: self.reporting.clone(),
        }
    }
}

/// What a log's writer saves: a snapshot it numbers as it commits.
type Commit<R> = Mutex<LogSnapshot<R>>;

/// One durable log's storage: the writer its commits run on, with the write
/// of a reporting setting staged on it, and what the cadence needs to
/// decide the next commit.
pub(crate) struct LogStore<R: SnapshotRecord> {
    oid: ObjectIdentifier,
    /// The writer, and a staged write's steps (#1952): each holds only its
    /// write (#2431).
    saves: StagedSaves<Commit<R>, ()>,
    /// Writes of the reporting settings the log keeps made so far, so a
    /// staged step is taken only if no other came between (#1952).
    writes: u64,
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
    /// The storage is the in-memory test backend, whose final commit may
    /// run on the dropping thread ([`closing`](Self::closing)).
    #[cfg(feature = "test-support")]
    commits_inline: bool,
}

impl<R: SnapshotRecord> LogStore<R> {
    /// Load the log's snapshot from `persistence`, if it holds one, and set
    /// up the writer its commits go through. The caller applies the snapshot
    /// to the log and then calls [`reopened`](Self::reopened).
    ///
    /// `buffer_size` is the log's configured Buffer_Size; one whose
    /// worst-case snapshot could outgrow a slot is refused before anything
    /// is loaded. The loaded snapshot is checked again here, as a guard for
    /// backends other than `FileLogPersistence`, the `std` file backend,
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
            codec::validate(snapshot).map_err(|error| Error::from(error.in_format(codec::WHAT)))?;
        }
        let last_failed = Arc::new(AtomicBool::new(false));
        let failed = Arc::clone(&last_failed);
        let generation = Arc::new(AtomicU64::new(
            loaded.as_ref().map_or(0, |snapshot| snapshot.generation),
        ));
        let committed = Arc::clone(&generation);
        #[cfg(feature = "test-support")]
        let commits_inline = persistence.commits_inline();
        let writer = SaveWriter::new(
            format!(
                "bacnet-log-{}-{}-save",
                oid.object_type().to_raw(),
                oid.instance_number()
            ),
            move |pending: &Commit<R>| {
                let mut snapshot = pending.lock();
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
                    log_warn!(
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
                saves: StagedSaves::new(writer),
                writes: 0,
                generation,
                cadence: LogCommitCadence::default(),
                queued: None,
                last_failed,
                queued_total: 0,
                interval_from: None,
                restart_interval: false,
                reopened: false,
                interruption_pending: false,
                #[cfg(feature = "test-support")]
                commits_inline,
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
        self.saves.submit_coalescing(Mutex::new(snapshot));
        self.note_queued(state);
    }

    /// `state` is the latest one handed to the writer.
    fn note_queued(&mut self, state: LogState) {
        self.queued_total = state.total_record_count;
        self.queued = Some(state);
        self.restart_interval = true;
    }

    /// Whether a write of a reporting setting is staged, its request still
    /// to take or release it. The cadence's commits wait meanwhile: one of
    /// the served state would leave storage behind a write about to be
    /// answered (#1952).
    pub(crate) fn staged(&self) -> bool {
        self.saves.is_staged()
    }

    /// Drop a staged write, as when the log stops or drops with its request
    /// gone, and count the log changed if its commit may have left storage
    /// holding settings the log never served, so the next commit puts
    /// storage back.
    fn drop_staged(&mut self) {
        self.saves.drop_staged();
        if self.saves.take_correction() {
            self.queued = None;
        }
    }

    /// A wait that ends once every commit queued so far has run.
    pub(crate) fn idle(&self) -> SaveWait {
        self.saves.idle()
    }

    /// The log is dropping. With the in-memory test backend, its final
    /// commit may run on the caller's thread when no writer thread is
    /// running (`SaveWriter::closing`), sparing the fuzz targets a thread
    /// per input (#2233). Any other storage keeps the writer thread: a
    /// `LogPersistence::commit` is documented to run there, outside any
    /// async runtime.
    pub(crate) fn closing(&mut self) {
        #[cfg(feature = "test-support")]
        if self.commits_inline {
            self.saves.closing();
        }
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
    log_warn!(
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
pub(crate) trait DurableLog: BACnetObject {
    /// The log's record family.
    type Record: SnapshotRecord;

    /// The state a commit would save now.
    fn log_state(&self) -> LogState;

    /// The storage, if the log has one.
    fn store(&self) -> Option<&LogStore<Self::Record>>;

    /// The log's reporting settings: its BUFFER_READY state, and a Trend
    /// Log's COV settings.
    fn reporting(&self) -> (&BufferReadyReporting, Option<&CovSettings>);

    /// [`reporting`](Self::reporting), to change.
    fn reporting_mut(&mut self) -> (&mut BufferReadyReporting, Option<&mut CovSettings>);

    /// The Description the log serves, for a written one it keeps (#2364).
    fn description_mut(&mut self) -> &mut String;

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
    /// count, unless a write is staged.
    fn commit_if_counted(&mut self) {
        let due = {
            let (store, buffer) = self.durable_parts();
            store.is_some_and(|store| {
                !store.staged() && store.count_due(buffer.total_record_count())
            })
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

    /// Commit when the state changed since the last commit, unless a
    /// write is staged.
    fn commit_if_changed(&mut self) {
        if self.store().is_none_or(LogStore::staged) {
            return;
        }
        let state = self.log_state();
        if let (Some(store), buffer) = self.durable_parts()
            && store.changed(&state)
        {
            store.queue(state, buffer.records());
        }
    }

    /// The final commit, as the server stops or the log drops: drop a
    /// staged write no request is left to take, commit what changed, and
    /// return the wait for every queued commit, or `None` without storage.
    fn commit_final(&mut self) -> Option<SaveWait> {
        if let Some(store) = self.durable_parts().0 {
            store.drop_staged();
        }
        self.commit_if_changed();
        self.durable_parts().0.map(|store| store.idle())
    }
}

/// Implement [`DurableWrites`](crate::durable::DurableWrites) and `Drop` for
/// a [`DurableLog`]. A write of a reporting setting the log keeps stages
/// its commit ([`staging`]); the server's stop drops a staged write no
/// request is left to take, commits the log once more and waits for the
/// commit, and so does dropping it.
macro_rules! impl_durable_log {
    ($log:ty) => {
        impl $crate::durable::DurableWrites for $log {
            fn stage_write(
                &mut self,
                property: bacnet_types::enums::PropertyIdentifier,
                array_index: Option<u32>,
                value: &bacnet_types::primitives::PropertyValue,
            ) -> $crate::durable::StageStep {
                self.stage_writes(&[$crate::durable::PendingWrite {
                    property,
                    array_index,
                    value: value.clone(),
                }])
            }

            fn stage_writes(
                &mut self,
                writes: &[$crate::durable::PendingWrite],
            ) -> $crate::durable::StageStep {
                $crate::log_persistence::store::staging::stage_writes(self, writes)
            }

            fn release_staged_write(&mut self, staged: &$crate::durable::SaveWait) {
                $crate::log_persistence::store::staging::release(self, staged);
            }

            fn has_staged_write(&self) -> bool {
                $crate::log_persistence::store::DurableLog::store(self)
                    .is_some_and($crate::log_persistence::store::LogStore::staged)
            }

            fn settle_forgotten_writes(&mut self) -> Option<$crate::durable::SaveWait> {
                $crate::log_persistence::store::DurableLog::commit_final(self)
            }

            fn expire_staged(&mut self, now: core::time::Duration) {
                $crate::log_persistence::store::staging::expire(self, now);
            }
        }

        impl Drop for $log {
            fn drop(&mut self) {
                if let Some(store) =
                    $crate::log_persistence::store::DurableLog::durable_parts(self).0
                {
                    store.closing();
                }
                // The writer's own drop, which follows, waits for it.
                let _ = $crate::log_persistence::store::DurableLog::commit_final(self);
            }
        }
    };
}

pub(crate) use impl_durable_log;
