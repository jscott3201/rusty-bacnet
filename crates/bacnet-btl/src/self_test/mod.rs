//! Self-test infrastructure — testing our own BACnet server.

pub mod in_process;

use std::sync::Arc;

use bacnet_objects::database::ObjectDatabase;
use tokio::sync::RwLock;

/// Self-test server handle — provides DB access for MAKE steps.
pub enum SelfTestServer {
    InProcess(in_process::InProcessServerHandle),
}

impl SelfTestServer {
    /// Get a reference to the database (for MAKE Direct steps).
    pub fn database(&self) -> &Arc<RwLock<ObjectDatabase>> {
        match self {
            Self::InProcess(h) => &h.db,
        }
    }
}
