//! Storage for the Timer and Channel tests: one snapshot in memory, saves
//! counted, and a switch that makes them fail.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::{WriteConfigPersistence, WriteConfigSnapshot};

#[derive(Default)]
pub(crate) struct MemoryConfig {
    pub(crate) saved: Mutex<Option<WriteConfigSnapshot>>,
    pub(crate) saves: AtomicUsize,
    pub(crate) fail: AtomicBool,
}

impl MemoryConfig {
    pub(crate) fn new() -> Arc<Self> {
        Arc::default()
    }

    /// Storage already holding `snapshot`.
    pub(crate) fn holding(snapshot: WriteConfigSnapshot) -> Arc<Self> {
        let storage = Self::new();
        *storage.saved.lock().unwrap() = Some(snapshot);
        storage
    }

    pub(crate) fn saved(&self) -> Option<WriteConfigSnapshot> {
        self.saved.lock().unwrap().clone()
    }

    pub(crate) fn saves(&self) -> usize {
        self.saves.load(Ordering::SeqCst)
    }

    pub(crate) fn fail(&self, fail: bool) {
        self.fail.store(fail, Ordering::SeqCst);
    }
}

impl WriteConfigPersistence for MemoryConfig {
    fn load(&self, _object: ObjectIdentifier) -> Result<Option<WriteConfigSnapshot>, Error> {
        Ok(self.saved())
    }

    fn save(&self, _object: ObjectIdentifier, snapshot: &WriteConfigSnapshot) -> Result<(), Error> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::Encoding("storage unavailable".into()));
        }
        *self.saved.lock().unwrap() = Some(snapshot.clone());
        self.saves.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
