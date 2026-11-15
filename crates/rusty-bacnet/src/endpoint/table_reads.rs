//! The BBMD table reads a `BipEndpoint` has started (#2612).
//!
//! Cancelling a read's asyncio future aborts its Rust task, but the task
//! drops its request later, on a runtime thread, and asyncio runs the
//! future's done callbacks only on a later turn of the loop. A read of the
//! same table from the same BBMD started at once after the cancel would find
//! that request still in flight and be refused. So each read is kept here by
//! table and BBMD: a new one gives up the last one whose future is done
//! before it starts, and a read's done callback gives it up and forgets it.
//! A read whose future isn't done is still wanted, and a second one beside
//! it is refused as `BACnetClient`'s is.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bacnet_transport::bip::BvlcGiveUp;
use pyo3::intern;
use pyo3::prelude::*;

/// Which table a read reads, and the BBMD's B/IP MAC.
pub(super) type TableKey = (Table, [u8; 6]);

/// The two tables an endpoint reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Table {
    Bdt,
    Fdt,
}

/// One read started: its number, its asyncio future and its give-up.
struct Started {
    id: u64,
    future: Py<PyAny>,
    give_up: BvlcGiveUp,
}

#[derive(Default)]
struct Reads {
    next: u64,
    started: HashMap<TableKey, Started>,
}

/// The reads one endpoint has started; clones share them.
#[derive(Clone, Default)]
pub(super) struct TableReads(Arc<Mutex<Reads>>);

impl TableReads {
    fn lock(&self) -> MutexGuard<'_, Reads> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Number a new read of `key`, first giving up the last one, if its
    /// future is done: cancelled, most often.
    pub(super) fn begin(&self, py: Python<'_>, key: TableKey) -> PyResult<(u64, BvlcGiveUp)> {
        let mut reads = self.lock();
        let done = match reads.started.get(&key) {
            Some(last) => last
                .future
                .bind(py)
                .call_method0(intern!(py, "done"))?
                .is_truthy()?,
            None => false,
        };
        if done && let Some(last) = reads.started.remove(&key) {
            last.give_up.give_up();
        }
        reads.next += 1;
        Ok((reads.next, BvlcGiveUp::default()))
    }

    /// Keep read `id` of `key`, unless a read of `key` that is still
    /// wanted is kept already: then this one is refused as in flight, and
    /// the one that runs is the one to give up later.
    pub(super) fn started(
        &self,
        key: TableKey,
        id: u64,
        future: &Bound<'_, PyAny>,
        give_up: BvlcGiveUp,
    ) {
        let started = Started {
            id,
            future: future.clone().unbind(),
            give_up,
        };
        self.lock().started.entry(key).or_insert(started);
    }

    /// What read `id`'s done callback runs: give it up, if it still runs,
    /// and forget it, unless a later read of `key` took its place.
    pub(super) fn on_done(
        &self,
        key: TableKey,
        id: u64,
        give_up: BvlcGiveUp,
    ) -> impl FnOnce() + Send + 'static {
        let reads = self.clone();
        move || {
            give_up.give_up();
            let mut reads = reads.lock();
            if reads.started.get(&key).is_some_and(|last| last.id == id) {
                reads.started.remove(&key);
            }
        }
    }
}
