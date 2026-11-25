//! A Python `Device`'s tie to its `BACnetClient` (#2561): the client's
//! state and the id of the handle the started client keeps for it.
//!
//! The Python object holds no high-level client between calls, so a
//! `stop()` never waits on, or fails for, a `Device` still alive: each call
//! takes the read lease the other calls take, and finds its handle among
//! the started client's, or makes it again after a restart.

use std::future::Future;

use pyo3::PyResult;

use super::ClientInner;
use super::state::{self, HandleId, Held};
use super::watches::WatchTie;
use crate::high_level::{OpsClient, OpsDevice, WatchShared};

/// One Python `Device`'s place among its client's handles.
pub(crate) struct DeviceTie {
    inner: ClientInner,
    id: HandleId,
}

impl DeviceTie {
    /// A tie to the client `inner` belongs to, with no handle made yet.
    pub(super) fn new(inner: ClientInner) -> Self {
        let id = inner.new_handle();
        Self { inner, id }
    }

    /// The handle, held with the client's lease for one call; `make` makes
    /// it when the client now started has none for this tie.
    ///
    /// # Errors
    ///
    /// RuntimeError when the client isn't started or is closing, as every
    /// other call raises, and whatever `make` fails with.
    pub(crate) async fn hold<F>(&self, make: impl FnOnce(OpsClient) -> F) -> PyResult<Held<'_>>
    where
        F: Future<Output = PyResult<OpsDevice>>,
    {
        state::held(&self.inner, self.id, make).await
    }

    /// Keep `watch`, made on this handle's device, among the client's
    /// watches until it ends; called under the lease of the call that made
    /// it, so a stop never misses it (see [`super::watches`]).
    pub(crate) fn keep_watch(&self, watch: std::sync::Arc<WatchShared>) -> WatchTie {
        WatchTie::new(std::sync::Arc::clone(&self.inner), watch)
    }

    /// What `read` makes of the handle, when the client now started has
    /// made it: for a getter, which can't wait for a lease. `read` gets no
    /// clone to keep, so a stop is never left waiting on a getter.
    pub(crate) fn peek<R>(&self, read: impl FnOnce(&OpsDevice) -> R) -> Option<R> {
        self.inner.with_handle(self.id, read)
    }
}

impl Drop for DeviceTie {
    fn drop(&mut self) {
        self.inner.drop_handle(self.id);
    }
}
