use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;

use tokio::sync::{Mutex, Notify};

use super::heartbeat::HubHeartbeat;
use super::{DeviceUuid, WsSink};

/// Per-client state tracked by the hub.
pub(super) struct HubClient {
    pub(super) sink: Arc<Mutex<WsSink>>,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) close_notify: Arc<Notify>,
    pub(super) device_uuid: DeviceUuid,
    pub(super) max_bvlc: u16,
    pub(super) max_npdu: u16,
    pub(super) last_activity: Arc<AtomicU64>,
    pub(super) heartbeat: HubHeartbeat,
}

impl HubClient {
    pub(super) fn new(
        sink: Arc<Mutex<WsSink>>,
        closed: Arc<AtomicBool>,
        close_notify: Arc<Notify>,
        device_uuid: DeviceUuid,
        max_bvlc: u16,
        max_npdu: u16,
        last_activity: Arc<AtomicU64>,
    ) -> Self {
        Self {
            sink,
            closed,
            close_notify,
            device_uuid,
            max_bvlc,
            max_npdu,
            last_activity,
            heartbeat: HubHeartbeat::default(),
        }
    }
}
