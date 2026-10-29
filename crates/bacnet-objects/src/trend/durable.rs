//! A Trend Log kept in storage (#1541); see [`crate::log_persistence`].

use crate::prelude::*;
use alloc::sync::Arc;

use bacnet_types::constructed::BACnetLogRecord;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, Time};

use super::TrendLogObject;
use crate::log_buffer::LogRecordBuffer;
use crate::log_persistence::store::{
    DurableLog, LogState, LogStore, fit_to_buffer_size, impl_durable_log,
};
use crate::log_persistence::{LogCommitCadence, LogPersistence};

impl TrendLogObject {
    /// Open a Trend Log kept in `persistence`, or start one there when it
    /// holds none yet.
    ///
    /// A log storage holds comes back as it was saved: Enable,
    /// Stop_When_Full, its records and Total_Record_Count, its BUFFER_READY
    /// counting and Notification_Threshold, and the settings a peer can write
    /// (the window, Logging_Type, Log_Interval, Align_Intervals,
    /// Interval_Offset and Log_DeviceObjectProperty). `buffer_size` wins
    /// over the saved Buffer_Size; a smaller one keeps the newest records.
    /// It then owes a LOG_INTERRUPTED record, appended once it can read a
    /// valid clock, unless it reopens full and stopped. See
    /// [`crate::log_persistence`] for when it commits.
    ///
    /// Fails with [`Error::OutOfRange`] for a `buffer_size` a slot can't
    /// hold, with storage's error when the snapshot can't be loaded, and
    /// when it doesn't hold together as a Trend Log's: another object's
    /// identity, a zero generation, more records than Buffer_Size, records
    /// with a Total_Record_Count of zero, a full buffer while Stop_When_Full
    /// and Enable are both TRUE, a window end that isn't an actual moment, a
    /// Logging_Type a Trend Log can't take, or more than one reference.
    pub fn with_persistence(
        instance: u32,
        name: impl Into<String>,
        buffer_size: u32,
        persistence: Arc<dyn LogPersistence<BACnetLogRecord>>,
    ) -> Result<Self, Error> {
        let mut log = Self::new(instance, name, buffer_size)?;
        let (mut store, loaded) = LogStore::open(log.oid, buffer_size, persistence)?;
        if let Some(mut snapshot) = loaded {
            let resized = fit_to_buffer_size(&mut snapshot, buffer_size);
            log.lifecycle().reopen(&mut snapshot)?;
            if let Some(acquisition) = snapshot.acquisition {
                log.acquisition.restore(acquisition);
            }
            log.log_device_object_property = snapshot.log_device_object_property.pop();
            store.reopened(log.log_state(), resized);
        }
        log.store = Some(store);
        Ok(log)
    }

    /// Whether the log came back from a snapshot its storage held, rather
    /// than starting empty. Always false without storage.
    pub fn reopened(&self) -> bool {
        self.store.as_ref().is_some_and(LogStore::is_reopened)
    }

    /// When the log commits to its storage; `None` without storage.
    pub fn commit_cadence(&self) -> Option<LogCommitCadence> {
        self.store.as_ref().map(LogStore::cadence)
    }

    /// Set when the log commits to its storage. Does nothing without
    /// storage.
    pub fn set_commit_cadence(&mut self, cadence: LogCommitCadence) {
        if let Some(store) = &mut self.store {
            store.set_cadence(cadence);
        }
    }

    /// The generation of the last snapshot storage took: the loaded one's
    /// until a commit lands, zero for storage that held none, and `None`
    /// without storage.
    pub fn storage_generation(&self) -> Option<u64> {
        self.store.as_ref().map(LogStore::generation)
    }
}

impl DurableLog for TrendLogObject {
    type Record = BACnetLogRecord;

    fn log_state(&self) -> LogState {
        LogState::of(
            &self.log_buffer,
            self.log_enable,
            self.stop_when_full,
            &self.window,
            &self.reporting,
        )
        .trend(
            self.acquisition.settings(),
            self.log_device_object_property.iter().cloned().collect(),
        )
    }

    fn durable_parts(
        &mut self,
    ) -> (
        Option<&mut LogStore<BACnetLogRecord>>,
        &LogRecordBuffer<BACnetLogRecord>,
    ) {
        (self.store.as_mut(), &self.log_buffer)
    }

    fn record_owed_interruption(&mut self, fallback: Option<(Date, Time)>) -> bool {
        self.lifecycle().record_interruption_now(fallback)
    }
}

impl_durable_log!(TrendLogObject);
