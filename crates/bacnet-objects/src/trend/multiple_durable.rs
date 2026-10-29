//! A Trend Log Multiple kept in storage (#1541); see
//! [`crate::log_persistence`].

use crate::prelude::*;
use alloc::sync::Arc;

use bacnet_types::constructed::BACnetLogMultipleRecord;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, Time};

use super::TrendLogMultipleObject;
use crate::log_buffer::LogRecordBuffer;
use crate::log_persistence::store::{
    DurableLog, LogState, LogStore, fit_to_buffer_size, impl_durable_log,
};
use crate::log_persistence::{LogCommitCadence, LogPersistence};

impl TrendLogMultipleObject {
    /// Open a Trend Log Multiple kept in `persistence`, or start one there
    /// when it holds none yet, as
    /// [`TrendLogObject::with_persistence`](crate::trend::TrendLogObject::with_persistence)
    /// does. Its snapshot holds the whole Log_DeviceObjectProperty array, up
    /// to [`MAX_LOG_DEVICE_OBJECT_PROPERTIES`](crate::trend::MAX_LOG_DEVICE_OBJECT_PROPERTIES)
    /// references, and its Logging_Type is POLLED or TRIGGERED.
    pub fn with_persistence(
        instance: u32,
        name: impl Into<String>,
        buffer_size: u32,
        persistence: Arc<dyn LogPersistence<BACnetLogMultipleRecord>>,
    ) -> Result<Self, Error> {
        let mut log = Self::new(instance, name, buffer_size)?;
        let (mut store, loaded) = LogStore::open(log.oid, buffer_size, persistence)?;
        if let Some(mut snapshot) = loaded {
            let resized = fit_to_buffer_size(&mut snapshot, buffer_size);
            log.lifecycle().reopen(&mut snapshot)?;
            if let Some(acquisition) = snapshot.acquisition {
                log.acquisition.restore(acquisition);
            }
            log.log_device_object_property = snapshot.log_device_object_property;
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
}

impl DurableLog for TrendLogMultipleObject {
    type Record = BACnetLogMultipleRecord;

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
            self.log_device_object_property.clone(),
        )
    }

    fn durable_parts(
        &mut self,
    ) -> (
        Option<&mut LogStore<BACnetLogMultipleRecord>>,
        &LogRecordBuffer<BACnetLogMultipleRecord>,
    ) {
        (self.store.as_mut(), &self.log_buffer)
    }

    fn record_owed_interruption(&mut self, fallback: Option<(Date, Time)>) -> bool {
        self.lifecycle().record_interruption_now(fallback)
    }
}

impl_durable_log!(TrendLogMultipleObject);
