//! A strong handle on the object database that lets go off the runtime
//! (#1561).
//!
//! The last handle on the database to go drops every object, and a durable
//! object (a Notification Forwarder, Notification Class, Access Rights object
//! or Audit Log with storage) then waits for the saves it has queued. Each
//! endpoint holder that may go last lets go through
//! [`drop_database_off_runtime`], so in async code that wait runs on Tokio's
//! blocking pool rather than on a runtime worker. This type does that for a
//! holder that can go by being dropped anywhere, a task aborted mid-await
//! included.

use std::ops::Deref;
use std::sync::Arc;

use bacnet_objects::database::ObjectDatabase;
use bacnet_server::server::drop_database_off_runtime;
use tokio::sync::RwLock;

/// A strong database handle released through [`drop_database_off_runtime`]
/// however it goes.
pub(crate) struct HeldDatabase(Option<Arc<RwLock<ObjectDatabase>>>);

impl HeldDatabase {
    pub(crate) fn new(db: Arc<RwLock<ObjectDatabase>>) -> Self {
        Self(Some(db))
    }
}

impl Deref for HeldDatabase {
    type Target = RwLock<ObjectDatabase>;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("held until dropped")
    }
}

impl Drop for HeldDatabase {
    fn drop(&mut self) {
        if let Some(db) = self.0.take() {
            drop(drop_database_off_runtime(db));
        }
    }
}
