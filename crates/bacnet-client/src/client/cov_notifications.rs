use super::*;
use std::fmt;

/// Delivery mode used for a received COV notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum COVNotificationDelivery {
    /// Notification arrived as a ConfirmedCOVNotification request.
    Confirmed,
    /// Notification arrived as an UnconfirmedCOVNotification request.
    Unconfirmed,
}

/// Application response policy for a decoded ConfirmedCOVNotification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmedCOVNotificationResponse {
    /// Send the standard SimpleAck response.
    Ack,
    /// Send a Reject PDU with the supplied reason.
    Reject(RejectReason),
    /// Do not respond; the remote sender will time out or retry.
    NoResponse,
}

/// Callback used to choose a ConfirmedCOVNotification response after decode.
pub type ConfirmedCOVNotificationAckPolicy =
    Arc<dyn Fn(&ReceivedCOVNotification) -> ConfirmedCOVNotificationResponse + Send + Sync>;

/// A decoded inbound COV notification plus transport-observed metadata.
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

impl ReceivedCOVNotification {
    pub(crate) fn new(
        notification: COVNotificationRequest,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        delivery: COVNotificationDelivery,
    ) -> Self {
        let (network, address) = match source_network {
            Some(npdu_addr) if !npdu_addr.mac_address.is_empty() => {
                (Some(npdu_addr.network), Some(npdu_addr.mac_address.clone()))
            }
            _ => (None, None),
        };

        Self {
            notification,
            source_mac: MacAddr::from_slice(source_mac),
            source_network: network,
            source_address: address,
            delivery,
        }
    }
}

pub(crate) fn default_confirmed_cov_notification_ack_policy() -> ConfirmedCOVNotificationAckPolicy {
    Arc::new(|_| ConfirmedCOVNotificationResponse::Ack)
}

impl ClientOptions {
    /// Set a callback that chooses how to answer decoded ConfirmedCOVNotifications.
    pub fn with_confirmed_cov_notification_ack_policy<F>(mut self, policy: F) -> Self
    where
        F: Fn(&ReceivedCOVNotification) -> ConfirmedCOVNotificationResponse + Send + Sync + 'static,
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
            .finish()
    }
}
