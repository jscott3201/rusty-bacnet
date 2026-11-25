//! The BBMD table reads a `BipEndpoint` has started (#2612).
//!
//! Cancelling a read's asyncio future aborts its Rust task, but the task
//! drops its request later, on a runtime thread, and asyncio runs the
//! future's done callbacks only on a later turn of the loop. A read of the
//! same table from the same BBMD started in the same turn as the cancel
//! would find that request still in flight and be refused. So every read is
//! kept here, by table and BBMD, until its done callback forgets it, and a
//! new read first gives up and forgets each kept read whose future is done
//! or whose event loop is closed, so it can never complete. A read whose
//! future isn't done is still wanted, and a second one beside it is refused
//! as `BACnetClient`'s is.
//!
//! The done callbacks reach the kept reads through a [`Weak`]: a future
//! that never completes holds its callback, and the callback must not keep
//! the reads, and so the future itself, alive.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use bacnet_transport::bip::BvlcGiveUp;
use bacnet_types::sync::MutexExt;
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
    started: HashMap<TableKey, Vec<Started>>,
}

impl Reads {
    /// Take read `id` of `key` out, if it is kept.
    fn take(&mut self, key: &TableKey, id: u64) -> Option<Started> {
        let list = self.started.get_mut(key)?;
        let at = list.iter().position(|read| read.id == id)?;
        let read = list.swap_remove(at);
        if list.is_empty() {
            self.started.remove(key);
        }
        Some(read)
    }
}

fn lock(reads: &Mutex<Reads>) -> MutexGuard<'_, Reads> {
    reads.locked()
}

/// The reads one endpoint has started; clones share them.
#[derive(Clone, Default)]
pub(super) struct TableReads(Arc<Mutex<Reads>>);

impl TableReads {
    /// Number a new read, first giving up and forgetting every kept read
    /// whose future is done, cancelled most often, or can't complete
    /// because its event loop is closed. Python runs with the lock released,
    /// and the forgotten reads are dropped after it.
    pub(super) fn begin(&self, py: Python<'_>) -> PyResult<(u64, BvlcGiveUp)> {
        let kept: Vec<(TableKey, u64, Py<PyAny>)> = lock(&self.0)
            .started
            .iter()
            .flat_map(|(key, list)| {
                list.iter()
                    .map(|read| (*key, read.id, read.future.clone_ref(py)))
            })
            .collect();
        let mut finished = Vec::new();
        for (key, id, future) in kept {
            let future = future.bind(py);
            let done = future.call_method0(intern!(py, "done"))?.is_truthy()?
                || future
                    .call_method0(intern!(py, "get_loop"))?
                    .call_method0(intern!(py, "is_closed"))?
                    .is_truthy()?;
            if done {
                finished.push((key, id));
            }
        }
        let (forgotten, id) = {
            let mut reads = lock(&self.0);
            let forgotten: Vec<Started> = finished
                .iter()
                .filter_map(|(key, id)| reads.take(key, *id))
                .collect();
            reads.next += 1;
            (forgotten, reads.next)
        };
        for read in forgotten {
            read.give_up.give_up();
        }
        Ok((id, BvlcGiveUp::default()))
    }

    /// Keep read `id` of `key` until its done callback, or a later
    /// [`begin`](Self::begin), forgets it.
    pub(super) fn started(
        &self,
        key: TableKey,
        id: u64,
        future: &Bound<'_, PyAny>,
        give_up: BvlcGiveUp,
    ) {
        let read = Started {
            id,
            future: future.clone().unbind(),
            give_up,
        };
        lock(&self.0).started.entry(key).or_default().push(read);
    }

    /// What read `id`'s done callback runs: give it up, if it still runs,
    /// and forget it.
    pub(super) fn on_done(
        &self,
        key: TableKey,
        id: u64,
        give_up: BvlcGiveUp,
    ) -> impl FnOnce() + Send + 'static {
        let reads: Weak<Mutex<Reads>> = Arc::downgrade(&self.0);
        move || {
            give_up.give_up();
            if let Some(reads) = reads.upgrade() {
                // Taken out under the lock, dropped after it.
                let forgotten = lock(&reads).take(&key, id);
                drop(forgotten);
            }
        }
    }
}
