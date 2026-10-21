use super::*;
use bacnet_encoding::constructed::decode_event_notification_tolerant;
use bacnet_services::alarm_event::EventNotificationRequest;

/// Default event notification broadcast channel capacity, independent of COV.
pub const DEFAULT_EVENT_CHANNEL_CAPACITY: usize = 64;

/// Maximum event notification broadcast channel capacity accepted at startup.
pub const MAX_EVENT_CHANNEL_CAPACITY: usize = 65_536;

/// Delivery mode used for a received event notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventNotificationDelivery {
    /// Notification arrived as a ConfirmedEventNotification request.
    Confirmed,
    /// Notification arrived as an UnconfirmedEventNotification request.
    Unconfirmed,
}

/// A decoded inbound event notification plus transport-observed metadata.
#[derive(Debug, Clone)]
pub struct ReceivedEventNotification {
    /// Decoded service payload. Unsupported or invalid message text is discarded
    /// as `None`; all other fields retain the shared decoder's validation.
    pub notification: EventNotificationRequest,
    /// Immediate transport source MAC that delivered the NPDU.
    pub source_mac: MacAddr,
    /// NPDU source network, when supplied by a routed sender.
    pub source_network: Option<u16>,
    /// NPDU source MAC/address, when supplied by a routed sender.
    pub source_address: Option<MacAddr>,
    /// Whether the notification used the confirmed or unconfirmed service.
    pub delivery: EventNotificationDelivery,
}

impl ReceivedEventNotification {
    fn decode(
        data: &[u8],
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        delivery: EventNotificationDelivery,
    ) -> Result<Self, Error> {
        let notification = decode_event_notification_tolerant(data)?;
        let (network, address) = match source_network {
            Some(source) if !source.mac_address.is_empty() => {
                (Some(source.network), Some(source.mac_address.clone()))
            }
            _ => (None, None),
        };
        Ok(Self {
            notification,
            source_mac: MacAddr::from_slice(source_mac),
            source_network: network,
            source_address: address,
            delivery,
        })
    }
}

impl ClientOptions {
    /// Set the independent event notification channel capacity.
    /// Values outside `1..=MAX_EVENT_CHANNEL_CAPACITY` fail at startup.
    pub fn with_event_channel_capacity(mut self, capacity: usize) -> Self {
        self.event_channel_capacity = capacity;
        self
    }
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Subscribe to incoming event notifications with a new independent receiver.
    ///
    /// Only subsequent notifications are delivered. Slow receivers observe
    /// broadcast lag; no receivers or lag never prevent a valid confirmed request
    /// from being acknowledged. The wire acknowledgment confirms receipt, not
    /// operator acknowledgment or durable application processing. Stopping the
    /// client ends dispatch; dropping it closes the channel after queued messages.
    pub fn event_notifications(&self) -> broadcast::Receiver<ReceivedEventNotification> {
        self.event_tx.subscribe()
    }

    pub(super) async fn receive_confirmed_event_notification(
        network: &Arc<NetworkLayer<T>>,
        event_tx: &broadcast::Sender<ReceivedEventNotification>,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        reply: InboundReply,
        req: ConfirmedRequestPdu,
        decode_warning: &WarnThrottle,
    ) {
        let received = match ReceivedEventNotification::decode(
            &req.service_request,
            source_mac,
            source_network,
            EventNotificationDelivery::Confirmed,
        ) {
            Ok(received) => received,
            Err(error) => {
                warn_throttled!(
                    decode_warning,
                    %error,
                    "Failed to decode ConfirmedEventNotification"
                );
                // The reason names the syntax fault, as the server's does
                // for any confirmed request (#1446).
                let reject_reason = error
                    .reject_reason()
                    .unwrap_or(RejectReason::INVALID_DATA_ENCODING);
                Self::send_confirmed_request_reject(
                    network,
                    source_mac,
                    source_network,
                    reply,
                    req.invoke_id,
                    reject_reason,
                )
                .await;
                return;
            }
        };
        let _ = event_tx.send(received);
        let ack = Apdu::SimpleAck(SimpleAck {
            invoke_id: req.invoke_id,
            service_choice: req.service_choice,
        });
        let mut buf = BytesMut::with_capacity(3);
        if let Err(error) = encode_apdu(&mut buf, &ack) {
            warn!(%error, "Failed to encode event notification acknowledgment");
            return;
        }
        if let Err(error) =
            Self::send_received_reply_apdu(network, &buf, source_mac, source_network, reply).await
        {
            warn!(%error, "Failed to send event notification acknowledgment");
        }
    }
}

pub(super) fn receive_unconfirmed_event_notification(
    event_tx: &broadcast::Sender<ReceivedEventNotification>,
    source_mac: &[u8],
    source_network: &Option<NpduAddress>,
    data: &[u8],
    decode_warning: &WarnThrottle,
) {
    match ReceivedEventNotification::decode(
        data,
        source_mac,
        source_network,
        EventNotificationDelivery::Unconfirmed,
    ) {
        Ok(received) => {
            let _ = event_tx.send(received);
        }
        Err(error) => {
            warn_throttled!(
                decode_warning,
                %error,
                "Failed to decode UnconfirmedEventNotification"
            );
        }
    }
}
