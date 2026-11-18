use super::*;
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;
use bacnet_types::primitives::ObjectIdentifier;
use std::fmt;

/// Delivery mode used for a received COV notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum COVNotificationDelivery {
    /// Notification arrived as a confirmed request: ConfirmedCOVNotification
    /// or ConfirmedCOVNotificationMultiple.
    Confirmed,
    /// Notification arrived as an unconfirmed request:
    /// UnconfirmedCOVNotification or UnconfirmedCOVNotificationMultiple.
    Unconfirmed,
}

/// Application response policy for a decoded confirmed COV notification,
/// either form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmedCOVNotificationResponse {
    /// Send the standard SimpleAck response.
    Ack,
    /// Send a Reject PDU with the supplied reason.
    Reject(RejectReason),
    /// Do not respond; the remote sender will time out or retry.
    NoResponse,
}

/// Callback used to choose how to answer a confirmed COV notification, of
/// either form, after decode.
pub type ConfirmedCOVNotificationAckPolicy =
    Arc<dyn Fn(&COVEvent) -> ConfirmedCOVNotificationResponse + Send + Sync>;

/// A decoded inbound COV notification about one object
/// (ConfirmedCOVNotification or UnconfirmedCOVNotification) plus
/// transport-observed metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct ReceivedCOVNotification {
    /// Decoded COV notification service payload.
    pub notification: COVNotificationRequest,
    /// Immediate transport source MAC that delivered the NPDU.
    pub source_mac: MacAddr,
    /// NPDU source network, when supplied by a routed sender.
    pub source_network: Option<u16>,
    /// NPDU source MAC/address, when supplied by a routed sender.
    pub source_address: Option<MacAddr>,
    /// Whether the notification arrived through the confirmed or unconfirmed service.
    pub delivery: COVNotificationDelivery,
}

/// A decoded inbound COV notification about several objects
/// (ConfirmedCOVNotificationMultiple or UnconfirmedCOVNotificationMultiple,
/// Clauses 13.17 and 13.18), the kind a SubscribeCOVPropertyMultiple
/// subscription brings, plus the same transport-observed metadata as
/// [`ReceivedCOVNotification`].
#[derive(Debug, Clone, PartialEq)]
pub struct ReceivedCOVNotificationMultiple {
    /// Decoded service payload: the subscriber process, the initiating
    /// device, the time remaining, the optional timestamp of the last change
    /// it conveys, and each object's changed values, a value's time of
    /// change with it when the subscription asked for timestamps.
    pub notification: COVNotificationMultipleRequest,
    /// Immediate transport source MAC that delivered the NPDU.
    pub source_mac: MacAddr,
    /// NPDU source network, when supplied by a routed sender.
    pub source_network: Option<u16>,
    /// NPDU source MAC/address, when supplied by a routed sender.
    pub source_address: Option<MacAddr>,
    /// Whether the notification arrived through the confirmed or unconfirmed service.
    pub delivery: COVNotificationDelivery,
}

/// One item of [`BACnetClient::cov_notifications`]: a COV notification of
/// either form, in the order the client received them.
#[derive(Debug, Clone, PartialEq)]
pub enum COVEvent {
    /// About one object: what SubscribeCOV and SubscribeCOVProperty
    /// subscriptions bring.
    Object(ReceivedCOVNotification),
    /// About several objects and properties: what SubscribeCOVPropertyMultiple
    /// subscriptions bring.
    Multiple(ReceivedCOVNotificationMultiple),
}

/// The NPDU source a routed sender supplied, as the received notifications
/// keep it: none unless both a network and an address came.
fn npdu_source(source_network: &Option<NpduAddress>) -> (Option<u16>, Option<MacAddr>) {
    match source_network {
        Some(npdu_addr) if !npdu_addr.mac_address.is_empty() => {
            (Some(npdu_addr.network), Some(npdu_addr.mac_address.clone()))
        }
        _ => (None, None),
    }
}

impl ReceivedCOVNotification {
    pub(crate) fn new(
        notification: COVNotificationRequest,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        delivery: COVNotificationDelivery,
    ) -> Self {
        let (source_network, source_address) = npdu_source(source_network);
        Self {
            notification,
            source_mac: MacAddr::from_slice(source_mac),
            source_network,
            source_address,
            delivery,
        }
    }
}

impl ReceivedCOVNotificationMultiple {
    pub(crate) fn new(
        notification: COVNotificationMultipleRequest,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        delivery: COVNotificationDelivery,
    ) -> Self {
        let (source_network, source_address) = npdu_source(source_network);
        Self {
            notification,
            source_mac: MacAddr::from_slice(source_mac),
            source_network,
            source_address,
            delivery,
        }
    }
}

impl COVEvent {
    /// The single-object notification, if this is one.
    pub fn object(&self) -> Option<&ReceivedCOVNotification> {
        match self {
            Self::Object(received) => Some(received),
            Self::Multiple(_) => None,
        }
    }

    /// The single-object notification, if this is one, by value.
    pub fn into_object(self) -> Option<ReceivedCOVNotification> {
        match self {
            Self::Object(received) => Some(received),
            Self::Multiple(_) => None,
        }
    }

    /// The several-object notification, if this is one, by value.
    pub fn into_multiple(self) -> Option<ReceivedCOVNotificationMultiple> {
        match self {
            Self::Object(_) => None,
            Self::Multiple(received) => Some(received),
        }
    }

    /// The several-object notification, if this is one.
    pub fn multiple(&self) -> Option<&ReceivedCOVNotificationMultiple> {
        match self {
            Self::Object(_) => None,
            Self::Multiple(received) => Some(received),
        }
    }

    /// The subscriber process identifier the notification carries.
    pub fn subscriber_process_identifier(&self) -> u32 {
        match self {
            Self::Object(received) => received.notification.subscriber_process_identifier,
            Self::Multiple(received) => received.notification.subscriber_process_identifier,
        }
    }

    /// The sender's own Device object identifier.
    pub fn initiating_device_identifier(&self) -> ObjectIdentifier {
        match self {
            Self::Object(received) => received.notification.initiating_device_identifier,
            Self::Multiple(received) => received.notification.initiating_device_identifier,
        }
    }

    /// Seconds the subscription has left; 0 for one that never expires.
    pub fn time_remaining(&self) -> u32 {
        match self {
            Self::Object(received) => received.notification.time_remaining,
            Self::Multiple(received) => received.notification.time_remaining,
        }
    }

    /// Whether the notification came as a confirmed or an unconfirmed request.
    pub fn delivery(&self) -> COVNotificationDelivery {
        match self {
            Self::Object(received) => received.delivery,
            Self::Multiple(received) => received.delivery,
        }
    }

    /// Immediate transport source MAC that delivered the NPDU.
    pub fn source_mac(&self) -> &MacAddr {
        match self {
            Self::Object(received) => &received.source_mac,
            Self::Multiple(received) => &received.source_mac,
        }
    }

    /// NPDU source network, when supplied by a routed sender.
    pub fn source_network(&self) -> Option<u16> {
        match self {
            Self::Object(received) => received.source_network,
            Self::Multiple(received) => received.source_network,
        }
    }

    /// NPDU source MAC/address, when supplied by a routed sender.
    pub fn source_address(&self) -> Option<&MacAddr> {
        match self {
            Self::Object(received) => received.source_address.as_ref(),
            Self::Multiple(received) => received.source_address.as_ref(),
        }
    }

    /// Decode a several-object COV notification
    /// (ConfirmedCOVNotificationMultiple or UnconfirmedCOVNotificationMultiple).
    /// A failure carries the Reject reason a confirmed request answers it
    /// with, the one the server gives the same syntax fault (#1446); an
    /// unconfirmed request's is unused.
    fn decode_multiple(
        data: &[u8],
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        delivery: COVNotificationDelivery,
    ) -> Result<Self, (Error, RejectReason)> {
        let notification = COVNotificationMultipleRequest::decode(data).map_err(|error| {
            let reason = error
                .reject_reason()
                .unwrap_or(RejectReason::INVALID_DATA_ENCODING);
            (error, reason)
        })?;
        Ok(Self::Multiple(ReceivedCOVNotificationMultiple::new(
            notification,
            source_mac,
            source_network,
            delivery,
        )))
    }

    /// Decode a one-object COV notification, as [`Self::decode_multiple`]
    /// does the several-object kind.
    fn decode_object(
        data: &[u8],
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        delivery: COVNotificationDelivery,
    ) -> Result<Self, (Error, RejectReason)> {
        let notification = COVNotificationRequest::decode_detailed(data).map_err(|error| {
            let reason = error.reject_reason();
            (error.into_error(), reason)
        })?;
        Ok(Self::Object(ReceivedCOVNotification::new(
            notification,
            source_mac,
            source_network,
            delivery,
        )))
    }
}

pub(crate) fn default_confirmed_cov_notification_ack_policy() -> ConfirmedCOVNotificationAckPolicy {
    Arc::new(|_| ConfirmedCOVNotificationResponse::Ack)
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Execute a ConfirmedCOVNotification or a
    /// ConfirmedCOVNotificationMultiple (Clauses 13.7 and 13.17), as `req`'s
    /// service choice says: publish it
    /// on `cov_tx` and answer as the policy says, or reject it when it
    /// doesn't decode. The client keeps no list of its subscriptions, so a
    /// notification for one it doesn't know is published and acknowledged
    /// like any other: both clauses let a recipient ignore that case and
    /// acknowledge, and the policy may answer otherwise. A retransmitted
    /// request is executed again, so it is acknowledged and published again.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn receive_confirmed_cov_notification(
        network: &Arc<NetworkLayer<T>>,
        cov_tx: &broadcast::Sender<COVEvent>,
        policy: &ConfirmedCOVNotificationAckPolicy,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        reply: InboundReply,
        req: ConfirmedRequestPdu,
        decode_warning: &WarnThrottle,
    ) {
        let multiple = super::dispatch::is_confirmed_cov_multiple(req.service_choice);
        let decode = if multiple {
            COVEvent::decode_multiple
        } else {
            COVEvent::decode_object
        };
        let response = match decode(
            &req.service_request,
            source_mac,
            source_network,
            COVNotificationDelivery::Confirmed,
        ) {
            Ok(event) => {
                debug!(
                    process = event.subscriber_process_identifier(),
                    multiple, "Received a confirmed COV notification"
                );
                let response = policy(&event);
                let _ = cov_tx.send(event);
                response
            }
            Err((error, reason)) => {
                warn_throttled!(
                    decode_warning,
                    %error,
                    multiple,
                    "Failed to decode a confirmed COV notification"
                );
                ConfirmedCOVNotificationResponse::Reject(reason)
            }
        };
        Self::send_confirmed_cov_notification_response(
            network,
            source_mac,
            source_network,
            req.invoke_id,
            req.service_choice,
            response,
            reply,
        )
        .await;
    }
}

/// Execute an UnconfirmedCOVNotification or an
/// UnconfirmedCOVNotificationMultiple (Clauses 13.8 and 13.18), as
/// `service_choice` says: publish it on `cov_tx`, or drop it with a warning
/// when it doesn't decode.
pub(super) fn receive_unconfirmed_cov_notification(
    cov_tx: &broadcast::Sender<COVEvent>,
    source_mac: &[u8],
    source_network: &Option<NpduAddress>,
    data: &[u8],
    service_choice: UnconfirmedServiceChoice,
    decode_warning: &WarnThrottle,
) {
    let multiple = super::dispatch::is_unconfirmed_cov_multiple(service_choice);
    let decode = if multiple {
        COVEvent::decode_multiple
    } else {
        COVEvent::decode_object
    };
    match decode(
        data,
        source_mac,
        source_network,
        COVNotificationDelivery::Unconfirmed,
    ) {
        Ok(event) => {
            debug!(
                process = event.subscriber_process_identifier(),
                multiple, "Received an unconfirmed COV notification"
            );
            let _ = cov_tx.send(event);
        }
        Err((error, _)) => {
            warn_throttled!(
                decode_warning,
                %error,
                multiple,
                "Failed to decode an unconfirmed COV notification"
            );
        }
    }
}

impl ClientOptions {
    /// Set a callback that chooses how to answer decoded confirmed COV
    /// notifications, ConfirmedCOVNotification and
    /// ConfirmedCOVNotificationMultiple alike.
    pub fn with_confirmed_cov_notification_ack_policy<F>(mut self, policy: F) -> Self
    where
        F: Fn(&COVEvent) -> ConfirmedCOVNotificationResponse + Send + Sync + 'static,
    {
        self.confirmed_cov_notification_ack_policy = Arc::new(policy);
        self
    }
}

impl fmt::Debug for ClientOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientOptions")
            .field("cov_channel_capacity", &self.cov_channel_capacity)
            .field("confirmed_cov_notification_ack_policy", &"<callback>")
            .field("node_dcc_override", &self.node_dcc_override)
            .finish()
    }
}
