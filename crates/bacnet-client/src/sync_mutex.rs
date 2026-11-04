//! A blocking mutex for state the client only touches in short, synchronous
//! steps (#1978 item 2a).
//!
//! The transaction table, the device table and the SegmentACK routes were
//! `tokio::sync::Mutex`es, whose every lock is a semaphore acquire and
//! release: about 770 instructions per confirmed round trip. No guard of
//! theirs is held across an `.await` (`clippy::await_holding_lock` checks
//! that), so a `std::sync::Mutex` does the same work for a fraction of it.
//! A blocking lock is safe in async code as long as nothing waits while
//! holding it, which is the rule this type relies on.

use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};

/// A `std::sync::Mutex` whose lock ignores poisoning. A panic while holding
/// it leaves the state as the panicking step left it, which is what the
/// `tokio::sync::Mutex` it replaces did too.
#[derive(Debug)]
pub(crate) struct SyncMutex<T>(Mutex<T>);

impl<T> SyncMutex<T> {
    /// A mutex holding `value`.
    pub(crate) fn new(value: T) -> Self {
        Self(Mutex::new(value))
    }

    /// Lock it. Never hold the guard across an `.await`.
    pub(crate) fn lock(&self) -> MutexGuard<'_, T> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// Lock it unless another thread holds it now. A `Drop` impl uses this,
    /// as it did with the async mutex, so a drop never waits.
    pub(crate) fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        match self.0.try_lock() {
            Ok(guard) => Some(guard),
            Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
    }
}
