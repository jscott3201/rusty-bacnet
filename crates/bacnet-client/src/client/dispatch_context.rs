//! Parameter bundles for the inbound APDU dispatcher (#902).

use super::*;

/// Shared client state the dispatcher reads and notifies.
pub(super) struct DispatchContext<'a, T: TransportPort + 'static> {
    /// Transaction state machine.
    pub(super) tsm: &'a Arc<Mutex<Tsm>>,
    /// Discovered-device table.
    pub(super) device_table: &'a Arc<Mutex<DeviceTable>>,
    /// Network layer used to send replies.
    pub(super) network: &'a Arc<NetworkLayer<T>>,
    /// Broadcast channel for COV notifications.
    pub(super) cov_tx: &'a broadcast::Sender<ReceivedCOVNotification>,
    /// Broadcast channel for event notifications.
    pub(super) event_tx: &'a broadcast::Sender<ReceivedEventNotification>,
    /// How confirmed COV notifications are acknowledged.
    pub(super) confirmed_cov_ack_policy: &'a ConfirmedCOVNotificationAckPolicy,
    /// Broadcast channel for device discovery events.
    pub(super) device_tx: &'a broadcast::Sender<DeviceEvent>,
    /// Broadcast channel for device address collision events.
    pub(super) device_collision_tx: &'a broadcast::Sender<DeviceCollisionEvent>,
    /// Broadcast channel for received I-Haves.
    pub(super) i_have_tx: &'a broadcast::Sender<ReceivedIHave>,
    /// Routes for SegmentAcks awaiting an outgoing segmented request.
    pub(super) seg_ack_senders: &'a Arc<Mutex<HashMap<SegAckKey, SegmentAckRoute>>>,
    /// The client's throttle on warnings for received APDUs and service
    /// requests that fail to decode (#1950).
    pub(super) decode_warning: &'a WarnThrottle,
    /// The client's throttle on warnings for responses it refuses: segments
    /// it aborts or NAKs, and acknowledgments for another service (#1950).
    pub(super) response_warning: &'a WarnThrottle,
}

/// Where an inbound APDU came from and how to reply to it.
pub(super) struct InboundApdu<'a> {
    /// Immediate MAC the APDU arrived from.
    pub(super) source_mac: &'a [u8],
    /// The peer's SNET/SADR when the APDU arrived through a router.
    pub(super) source_network: &'a Option<NpduAddress>,
    /// Transport provenance snapshot for this APDU.
    pub(super) provenance: TransportProvenance,
    /// Complete original response context for an accepted request.
    pub(super) response_route: bacnet_network::response_route::ResponseRoute,
    /// Whether the APDU was addressed to a group (broadcast or multicast).
    pub(super) is_group: bool,
    /// Channel for handing a reply to the transport, when it supplied one.
    pub(super) reply_tx: Option<oneshot::Sender<Bytes>>,
}
