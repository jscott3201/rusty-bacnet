use super::*;

impl DeviceCollisionEvent {
    /// The rows that claim `instance` in `table` now.
    pub(super) fn of(table: &DeviceTable, instance: u32) -> Self {
        Self {
            instance,
            claimants: table.claiming(instance).cloned().collect(),
        }
    }

    /// True when at most one address claims the instance now, so the
    /// contest has ended and a lookup by the instance works again.
    pub fn is_resolved(&self) -> bool {
        self.claimants.len() < 2
    }
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Get a receiver for device discovery events. Each call returns a new
    /// independent receiver.
    ///
    /// Events are notification-only; `discovered_devices()` remains the
    /// authoritative snapshot of the current discovery table.
    pub fn device_events(&self) -> broadcast::Receiver<DeviceEvent> {
        self.device_tx.subscribe()
    }

    /// Get a receiver for Device-instance collision notifications. Each call
    /// returns a new independent receiver.
    ///
    /// Delivery is notification-only and best-effort: receivers can lag and
    /// missed notifications do not affect collision handling. Each notice
    /// lists the rows that claim the instance after a change, and the table
    /// holds every one of them; `discovered_devices()` remains the
    /// authoritative snapshot.
    pub fn device_collision_events(&self) -> broadcast::Receiver<DeviceCollisionEvent> {
        self.device_collision_tx.subscribe()
    }
}
