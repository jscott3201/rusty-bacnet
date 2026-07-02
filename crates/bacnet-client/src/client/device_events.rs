use super::*;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Get a receiver for device discovery events. Each call returns a new
    /// independent receiver.
    ///
    /// Events are notification-only; `discovered_devices()` remains the
    /// authoritative snapshot of the current discovery table.
    pub fn device_events(&self) -> broadcast::Receiver<DeviceEvent> {
        self.device_tx.subscribe()
    }
}
