//! An Event Log kept in storage (#1541); see [`crate::log_persistence`].

use crate::prelude::*;
use alloc::sync::Arc;

use bacnet_types::constructed::BACnetEventLogRecord;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, Time};

use super::EventLogObject;
use crate::log_buffer::LogRecordBuffer;
use crate::log_persistence::store::{
    DurableLog, LogState, LogStore, fit_to_buffer_size, impl_durable_log, staging,
};
use crate::log_persistence::{LogCommitCadence, LogPersistence, LogReportingSettings};
use crate::log_reporting::BufferReadyReporting;
use crate::trend::CovSettings;

impl EventLogObject {
    /// Open an Event Log kept in `persistence`, or start one there when it
    /// holds none yet, as
    /// [`TrendLogObject::with_persistence`](crate::trend::TrendLogObject::with_persistence)
    /// does. An Event Log's snapshot has no acquisition settings,
    /// references or COV settings, and whether it logs received
    /// notifications is the application's to set on each start.
    pub fn with_persistence(
        instance: u32,
        name: impl Into<String>,
        buffer_size: u32,
        persistence: Arc<dyn LogPersistence<BACnetEventLogRecord>>,
    ) -> Result<Self, Error> {
        let mut log = Self::new(instance, name, buffer_size)?;
        let (mut store, loaded) = LogStore::open(log.oid, buffer_size, persistence)?;
        if let Some(mut snapshot) = loaded {
            let resized = fit_to_buffer_size(&mut snapshot, buffer_size);
            log.lifecycle().reopen(&mut snapshot)?;
            staging::restore(&mut log, &snapshot.reporting);
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

    /// The generation of the last snapshot storage took, as
    /// [`TrendLogObject::storage_generation`](crate::trend::TrendLogObject::storage_generation)
    /// gives it.
    pub fn storage_generation(&self) -> Option<u64> {
        self.store.as_ref().map(LogStore::generation)
    }

    /// The reporting settings storage keeps because a write set them, now
    /// or before a restart, as
    /// [`TrendLogObject::written_reporting`](crate::trend::TrendLogObject::written_reporting)
    /// gives them (#1952).
    pub fn written_reporting(&self) -> LogReportingSettings {
        staging::written(self)
    }

    /// Drop every reporting setting a write saved, so the setters apply
    /// again, as
    /// [`TrendLogObject::clear_written_reporting`](crate::trend::TrendLogObject::clear_written_reporting)
    /// does (#1952).
    pub fn clear_written_reporting(&mut self) -> Result<(), Error> {
        staging::clear(self)
    }

    /// Whether a setter of `property` leaves it alone, as a write set it.
    pub(super) fn kept_over_setter(&self, property: PropertyIdentifier) -> bool {
        let kept = staging::kept_over_setter(self, property);
        if kept {
            log_debug!(log = %self.oid, ?property, "A written reporting setting kept over a setter");
        }
        kept
    }
}

impl DurableLog for EventLogObject {
    type Record = BACnetEventLogRecord;

    fn log_state(&self) -> LogState {
        LogState::of(
            &self.log_buffer,
            self.log_enable,
            self.stop_when_full,
            &self.window,
            &self.reporting,
        )
    }

    fn store(&self) -> Option<&LogStore<BACnetEventLogRecord>> {
        self.store.as_ref()
    }

    fn reporting(&self) -> (&BufferReadyReporting, Option<&CovSettings>) {
        (&self.reporting, None)
    }

    fn reporting_mut(&mut self) -> (&mut BufferReadyReporting, Option<&mut CovSettings>) {
        (&mut self.reporting, None)
    }

    fn description_mut(&mut self) -> &mut String {
        &mut self.description
    }

    fn durable_parts(
        &mut self,
    ) -> (
        Option<&mut LogStore<BACnetEventLogRecord>>,
        &LogRecordBuffer<BACnetEventLogRecord>,
    ) {
        (self.store.as_mut(), &self.log_buffer)
    }

    fn record_owed_interruption(&mut self, fallback: Option<(Date, Time)>) -> bool {
        self.lifecycle().record_interruption_now(fallback)
    }
}

impl_durable_log!(EventLogObject);
