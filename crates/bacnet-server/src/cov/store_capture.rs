//! The server's captures of timestamped COV-multiple changes: the core's
//! plain [`TimedCapture`] with the store its changes are queued in.
//!
//! Producers collect the affected references under a short COV table read
//! and read objects after releasing it, still under the database write
//! guard that committed the change; the admission capture runs inside the
//! subscribe service, which already holds the table. The store is locked
//! last (database, then table, then store), only to queue what was read:
//! no object is read under it. Its lock is where the server reads the
//! capture's instant, once for all the changes one capture queues.

use bacnet_objects::clock::ClockFrame;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::primitives::ObjectIdentifier;

use super::timed_capture::{CaptureQueue, CaptureRead, TimedCapture, TimedWriteCapture};
use super::timed_store::HistoryStore;
use super::{CovSubscriptionSnapshot, CovSubscriptionTable};
use crate::runtime_clock;
use bacnet_device_core::cov::CovSubscriptionKey;
use bacnet_device_core::cov::life_safety::LifeSafetyCovChange;
use bacnet_device_core::write::WriteCommitObserver;

/// References a producer must capture, with the store that takes their
/// changes.
pub(crate) struct StoreCapture {
    store: HistoryStore,
    capture: TimedCapture,
}

impl CovSubscriptionTable {
    fn store_capture(&self, capture: TimedCapture) -> StoreCapture {
        StoreCapture {
            store: self.timed.clone(),
            capture,
        }
    }

    /// See [`TimedCapture::object`].
    pub(crate) fn timed_capture(&self, oid: ObjectIdentifier) -> StoreCapture {
        self.store_capture(TimedCapture::object(
            &self.core,
            oid,
            runtime_clock::core_now(),
        ))
    }

    /// See [`TimedCapture::exact`].
    pub(crate) fn timed_capture_exact(&self, changes: &[LifeSafetyCovChange]) -> StoreCapture {
        self.store_capture(TimedCapture::exact(
            &self.core,
            changes,
            runtime_clock::core_now(),
        ))
    }

    /// See [`TimedCapture::all`].
    pub(crate) fn timed_capture_all(&self) -> StoreCapture {
        self.store_capture(TimedCapture::all(&self.core, runtime_clock::core_now()))
    }

    /// See [`TimedCapture::initial`].
    pub(crate) fn initial_timed_capture(
        &self,
        snapshots: &[CovSubscriptionSnapshot],
        frame: ClockFrame,
    ) -> StoreCapture {
        self.store_capture(TimedCapture::initial(
            &self.core,
            snapshots,
            frame,
            runtime_clock::core_now(),
        ))
    }
}

impl StoreCapture {
    /// Queue each qualifying change under the committing mutation's database
    /// guard, and return the references that queued one; see
    /// [`TimedCapture::read`].
    pub(crate) fn run(mut self, db: &ObjectDatabase) -> Vec<CovSubscriptionKey> {
        match self.capture.read(db) {
            Some(read) => self.store.queue(read),
            None => Vec::new(),
        }
    }

    /// Capture an intermediate event state; see
    /// [`TimedCapture::read_snapshot`].
    pub(crate) fn run_from_snapshot(
        mut self,
        snapshot: &dyn BACnetObject,
        frame: Option<ClockFrame>,
    ) -> Vec<CovSubscriptionKey> {
        match self.capture.read_snapshot(snapshot, frame) {
            Some(read) => self.store.queue(read),
            None => Vec::new(),
        }
    }

    /// The per-attempt capture of a write request, around its own observer
    /// `inner`; see [`TimedWriteCapture`].
    pub(crate) fn for_write<'a>(
        self,
        inner: Option<&'a mut dyn WriteCommitObserver>,
    ) -> TimedWriteCapture<'a, HistoryStore> {
        TimedWriteCapture::new(self.capture, self.store, inner)
    }
}

/// The store queues a capture's read under its lock, at the instant it reads
/// there.
impl CaptureQueue for HistoryStore {
    fn queue(&mut self, read: CaptureRead) -> Vec<CovSubscriptionKey> {
        let mut timed = self.lock();
        let now = runtime_clock::core_now();
        read.queue(&mut timed, now)
    }
}
