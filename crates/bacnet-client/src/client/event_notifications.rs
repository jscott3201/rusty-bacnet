use super::*;
use bacnet_encoding::{primitives, tags};
use bacnet_services::alarm_event::EventNotificationRequest;
use std::ops::Range;

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
        let notification = decode_event_notification(data)?;
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
        reply_tx: Option<oneshot::Sender<Bytes>>,
        req: ConfirmedRequestPdu,
    ) {
        let received = match ReceivedEventNotification::decode(
            &req.service_request,
            source_mac,
            source_network,
            EventNotificationDelivery::Confirmed,
        ) {
            Ok(received) => received,
            Err(error) => {
                warn!(%error, "Failed to decode ConfirmedEventNotification");
                Self::send_confirmed_request_reject(
                    network,
                    source_mac,
                    source_network,
                    reply_tx,
                    req.invoke_id,
                    RejectReason::INVALID_PARAMETER_DATA_TYPE,
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
            Self::send_received_reply_apdu(network, &buf, source_mac, source_network, reply_tx)
                .await
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
        Err(error) => warn!(%error, "Failed to decode UnconfirmedEventNotification"),
    }
}

// Tolerance is client-local: remove only a structurally complete messageText
// whose character-string content fails decoding, then validate the entire
// remaining request with the unchanged shared decoder. A missing charset byte,
// malformed framing, or any other invalid field must not gain acceptance.
fn decode_event_notification(data: &[u8]) -> Result<EventNotificationRequest, Error> {
    let original_error = match EventNotificationRequest::decode(data) {
        Ok(notification) => return Ok(notification),
        Err(error) => error,
    };
    let Some(text) = invalid_message_text_range(data) else {
        return Err(original_error);
    };
    // One retry and one scratch buffer, strictly smaller than the input.
    let mut without_text = Vec::with_capacity(data.len() - text.len());
    without_text.extend_from_slice(&data[..text.start]);
    without_text.extend_from_slice(&data[text.end..]);
    EventNotificationRequest::decode(&without_text)
}

fn invalid_message_text_range(data: &[u8]) -> Option<Range<usize>> {
    let mut offset = 0;
    // Walk only the fixed prefix, never searching bytes inside another value.
    for number in 0..7 {
        let (tag, content) = tags::decode_tag(data, offset).ok()?;
        if number == 3 {
            if !tag.is_opening_tag(number) {
                return None;
            }
            offset = tags::extract_context_value(data, content, number).ok()?.1;
        } else {
            if !tag.is_context(number) {
                return None;
            }
            offset = content.checked_add(tag.length as usize)?;
            data.get(content..offset)?;
        }
    }
    let (tag, content) = tags::decode_tag(data, offset).ok()?;
    if !tag.is_context(7) || tag.length == 0 {
        return None;
    }
    let end = content.checked_add(tag.length as usize)?;
    let text = data.get(content..end)?;
    // Removal must not make a duplicate text field become the optional field.
    // The next field must already be notifyType in the original request.
    if !tags::decode_tag(data, end).ok()?.0.is_context(8) {
        return None;
    }
    primitives::decode_character_string(text)
        .is_err()
        .then_some(offset..end)
}
