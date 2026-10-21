//! Shared one-attempt wire delivery and configuration-fenced health.
use super::*;

pub(in crate::server) fn encode_notification(
    notification: &BACnetAuditNotification,
    confirmed: bool,
    max_apdu: u32,
    invoke_id: u8,
) -> Option<BytesMut> {
    let mut service = BytesMut::new();
    AuditNotificationRequest {
        notifications: vec![notification.clone()],
    }
    .try_encode(&mut service)
    .ok()?;
    let pdu = if confirmed {
        Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: apdu::max_apdu_header_at_or_below(max_apdu).ok()?,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
            service_request: service.freeze(),
        })
    } else {
        Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION,
            service_request: service.freeze(),
        })
    };
    let mut bytes = BytesMut::new();
    encode_apdu(&mut bytes, &pdu).ok()?;
    (bytes.len() <= max_apdu as usize).then_some(bytes)
}

pub(in crate::server) async fn deliver<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    route: &ConfirmedRecipientRoute,
    bytes: &[u8],
    reserved: Option<NotificationReservation>,
    deadline: tokio::time::Instant,
) -> bool {
    deliver_observed(network, route, bytes, reserved, deadline, None).await
}

/// Send one encoded UnconfirmedAuditNotification by global broadcast
/// (DNET 0xFFFF) and report whether the link took it. A recipient change
/// whose old recipient has no route goes out this way too, so it reaches
/// every listening logger (Clause 12.11.66). Like every audit send, no DCC
/// state holds it back ([`deliver_observed`]).
pub(in crate::server) async fn deliver_global_broadcast<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    bytes: &[u8],
    deadline: tokio::time::Instant,
) -> bool {
    tokio::time::timeout_at(
        deadline,
        network.broadcast_global_apdu(bytes, false, NetworkPriority::Normal),
    )
    .await
    .is_ok_and(|sent| sent.is_ok())
}

/// Send one audit notification and report whether it was delivered.
///
/// No DeviceCommunicationControl state holds it back. Clause 16.1 leaves
/// Confirmed- and UnconfirmedAuditNotification running under
/// DISABLE_INITIATION, and the server refuses the deprecated DISABLE, so no
/// state a peer can set stops audit traffic. Every audit sender in the server
/// funnels through here or follows the same rule.
pub(in crate::server) async fn deliver_observed<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    route: &ConfirmedRecipientRoute,
    bytes: &[u8],
    reserved: Option<NotificationReservation>,
    deadline: tokio::time::Instant,
    local: Option<super::audit_batch_queue::LocalDisposition>,
) -> bool {
    let local = std::sync::Mutex::new(local);
    let confirmed = reserved.is_some();
    let send = || async {
        let _local = local.lock().unwrap().take();
        match (&route.local_target, &route.remote) {
            (Some(mac), None) => {
                network
                    .send_apdu(bytes, mac, confirmed, NetworkPriority::Normal)
                    .await
            }
            (None, Some((net, mac, Some(router)))) => {
                network
                    .send_apdu_routed(bytes, *net, mac, router, confirmed, NetworkPriority::Normal)
                    .await
            }
            _ => Err(Error::Encoding("audit destination is unavailable".into())),
        }
    };
    tokio::time::timeout_at(deadline, async {
        if let Some((operation, receiver)) = reserved {
            run_notification_worker(operation, receiver, DELIVERY_TIMEOUT, 0, |_| send()).await
                == NotificationWorkerResult::Ack
        } else {
            send().await.is_ok()
        }
    })
    .await
    .unwrap_or(false)
}

/// Cancellation, rejected worker admission and panic also leave visible failure.
pub(in crate::server) struct DeliveryCompletion {
    pub(in crate::server) status: Arc<AuditReporterStatus>,
    pub(in crate::server) epoch: bacnet_objects::audit::AuditDeliveryToken,
    pub(in crate::server) finished: bool,
}

impl DeliveryCompletion {
    pub(in crate::server) fn auditing_failure(
        status: Arc<AuditReporterStatus>,
        expected: u64,
    ) -> Option<Self> {
        let epoch = status.begin_auditing_failure_delivery(expected)?;
        Some(Self {
            status,
            epoch,
            finished: false,
        })
    }
    pub(in crate::server) fn finish(mut self, delivered: bool) {
        self.status.complete_delivery(self.epoch, delivered);
        self.finished = true;
    }
}

impl Drop for DeliveryCompletion {
    fn drop(&mut self) {
        if !self.finished {
            self.status.complete_delivery(self.epoch, false);
        }
    }
}
