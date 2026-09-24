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
            max_apdu_length: max_apdu as u16,
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
    comm_state: &AtomicU8,
    route: &ConfirmedRecipientRoute,
    bytes: &[u8],
    reserved: Option<NotificationReservation>,
    deadline: tokio::time::Instant,
) -> bool {
    deliver_observed(network, comm_state, route, bytes, reserved, deadline, None).await
}

pub(in crate::server) async fn deliver_observed<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    comm_state: &AtomicU8,
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
        if comm_state.load(Ordering::Acquire) != 0 {
            return Err(Error::Encoding("audit initiation disabled".into()));
        }
        match (&route.local_target, &route.remote) {
            (Some(mac), None) => {
                network
                    .send_apdu(bytes, mac, confirmed, NetworkPriority::NORMAL)
                    .await
            }
            (None, Some((net, mac, Some(router)))) => {
                network
                    .send_apdu_routed(bytes, *net, mac, router, confirmed, NetworkPriority::NORMAL)
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
