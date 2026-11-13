//! Borrowed handles shared by the EventNotification send paths.
use super::*;

/// The server handles and delivery settings an EventNotification send reads:
/// object database, network, DCC communication state, learned routers,
/// notification transactions, device bindings, the undelivered-notification
/// counters, the confirmed retry timeout, the local APDU capacity and the
/// Device the notifications name as their initiator. Borrows and plain values
/// only, so it copies.
pub(super) struct EventDelivery<'a, T: TransportPort + 'static> {
    pub(super) db: &'a Arc<RwLock<ObjectDatabase>>,
    pub(super) network: &'a Arc<NetworkLayer<T>>,
    pub(super) comm_state: &'a Arc<CommState>,
    pub(super) learned_routers: &'a Arc<Mutex<LearnedRouterCache>>,
    pub(super) notification_transactions: &'a Arc<NotificationTransactions>,
    pub(super) device_bindings: &'a Arc<RwLock<DeviceBindingTable>>,
    pub(super) suppressions: &'a Arc<super::event_suppression::EventSuppressions>,
    pub(super) retry_timeout_ms: u64,
    pub(super) local_apdu_capacity: u32,
    /// The identity's Device ([`initiating_device`](crate::local_device::initiating_device)),
    /// which every notification names as its initiator and which a recipient
    /// naming this device names (#2217).
    pub(super) device: ObjectIdentifier,
}

impl<T: TransportPort + 'static> Clone for EventDelivery<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: TransportPort + 'static> Copy for EventDelivery<'_, T> {}

/// The Device a hand-built test [`EventDelivery`] names: its database's one
/// Device, as the identity of a server built on that database names it, or
/// Device 0 in a database without one.
#[cfg(test)]
pub(super) async fn fixture_device(db: &Arc<RwLock<ObjectDatabase>>) -> ObjectIdentifier {
    db.read()
        .await
        .selected_device()
        .unwrap_or_else(|| ObjectIdentifier::new(ObjectType::DEVICE, 0).unwrap())
}
