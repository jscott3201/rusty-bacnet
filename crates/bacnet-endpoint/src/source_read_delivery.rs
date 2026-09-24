//! Bounded source notification delivery and local communication-result mapping.
use super::*;
use bacnet_encoding::apdu::{encode_apdu, Apdu, ConfirmedRequest, UnconfirmedRequest};
use bacnet_endpoint_core::coordinator::CanonicalPeer;
use bacnet_endpoint_core::endpoint_ingress::EndpointEgressAdmissionError;
use bacnet_server::server::{
    __endpoint_NotificationWorkerResult as NotificationWorkerResult,
    __endpoint_run_notification_worker as run_notification_worker,
};
use bacnet_services::audit::AuditNotificationRequest;
use bacnet_types::enums::{
    ConfirmedServiceChoice, ErrorClass, ErrorCode, NetworkPriority, UnconfirmedServiceChoice,
};
use bacnet_types::MacAddr;
use bytes::BytesMut;
use tokio::time::{Duration, Instant};

pub(super) const DEADLINE: Duration = Duration::from_secs(3);

pub(super) struct Completion {
    status: Arc<AuditReporterStatus>,
    epoch: bacnet_objects::audit::AuditDeliveryToken,
    finished: bool,
}
impl Completion {
    pub(super) fn new(
        status: Arc<AuditReporterStatus>,
        epoch: bacnet_objects::audit::AuditDeliveryToken,
    ) -> Self {
        Self {
            status,
            epoch,
            finished: false,
        }
    }

    pub(super) fn auditing_failure(
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
    pub(super) fn finish(mut self, delivered: bool) {
        self.status.complete_delivery(self.epoch, delivered);
        self.finished = true;
    }
}
impl Drop for Completion {
    fn drop(&mut self) {
        if !self.finished {
            self.status.complete_delivery(self.epoch, false);
        }
    }
}

pub(super) fn admit(
    source: &Arc<SourceRead>,
    owner: &NotificationTransactions,
    confirmed: bool,
    mac: MacAddr,
    notification: BACnetAuditNotification,
    completion: Completion,
    failure: Option<AuditFailureTicket<MacAddr>>,
) {
    // Invalid/oversized records are not resource losses, even at saturation.
    let Some(mut encoded) = encode(&notification, confirmed, source.max_apdu, 0) else {
        return;
    };
    let timestamp = notification
        .source_timestamp
        .clone()
        .expect("source record timestamp");
    let permit = match owner.try_admit_audit() {
        Ok(permit) => permit,
        Err(tokio::sync::TryAcquireError::NoPermits) => {
            failures::record_drop(source, owner, failure, timestamp);
            return;
        }
        Err(tokio::sync::TryAcquireError::Closed) => return,
    };
    let reserved = if confirmed {
        match owner.reserve(
            CanonicalPeer::direct(mac.as_slice()),
            ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
        ) {
            Ok(reserved) => Some(reserved),
            Err(bacnet_server::server::__endpoint_NotificationReserveError::Coordinator(
                bacnet_endpoint_core::coordinator::ReserveError::Exhausted,
            )) => {
                failures::record_drop(source, owner, failure, timestamp);
                return;
            }
            Err(_) => return,
        }
    } else {
        None
    };
    if let Some((operation, _)) = &reserved {
        let Some(bytes) = encode(&notification, true, source.max_apdu, operation.invoke_id())
        else {
            return;
        };
        encoded = bytes;
    }
    let deadline = Instant::now() + DEADLINE;
    let egress = source.egress.clone();
    let weak_owner = source.notifications.clone();
    let source = Arc::downgrade(source);
    owner.spawn(async move {
        let _permit = permit;
        let sent = admit_encoded(&egress, mac, encoded, confirmed, deadline);
        if matches!(sent, Err(EndpointEgressAdmissionError::QueueFull)) {
            if let (Some(source), Some(owner)) = (source.upgrade(), weak_owner.upgrade()) {
                failures::record_drop(&source, &owner, failure, timestamp);
            }
        }
        completion.finish(finish_send(sent, reserved, deadline).await);
    });
}

pub(super) fn encode(
    notification: &BACnetAuditNotification,
    confirmed: bool,
    max_apdu: u16,
    invoke_id: u8,
) -> Option<Vec<u8>> {
    let mut service = BytesMut::new();
    AuditNotificationRequest {
        notifications: vec![notification.clone()],
    }
    .try_encode(&mut service)
    .ok()?;
    let pdu = if confirmed {
        Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: max_apdu,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
            service_request: service.freeze(),
        })
    } else {
        Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION,
            service_request: service.freeze(),
        })
    };
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, &pdu).ok()?;
    (encoded.len() <= usize::from(max_apdu)).then(|| encoded.to_vec())
}

pub(super) fn admit_encoded(
    egress: &EndpointEgress,
    mac: MacAddr,
    encoded: Vec<u8>,
    confirmed: bool,
    deadline: Instant,
) -> Result<bacnet_endpoint_core::endpoint_ingress::EndpointSend, EndpointEgressAdmissionError> {
    egress.admit_apdu(
        encoded,
        EndpointApduDestination::Direct {
            destination_mac: mac,
        },
        confirmed,
        NetworkPriority::NORMAL,
        Vec::new(),
        Some(deadline),
    )
}

type Reservation = (
    bacnet_server::server::__endpoint_NotificationOperation,
    oneshot::Receiver<bacnet_server::server::CovAckResult>,
);

pub(super) async fn finish_send(
    sent: Result<
        bacnet_endpoint_core::endpoint_ingress::EndpointSend,
        EndpointEgressAdmissionError,
    >,
    reserved: Option<Reservation>,
    deadline: Instant,
) -> bool {
    let mut sent = Some(sent);
    let send = |_| {
        let sent = sent.take().expect("notifications have no retries");
        async move { sent?.complete().await.result }
    };
    tokio::time::timeout_at(deadline, async {
        if let Some((operation, receiver)) = reserved {
            run_notification_worker(operation, receiver, DEADLINE, 0, send).await
                == NotificationWorkerResult::Ack
        } else {
            let mut send = send;
            send(0).await.is_ok()
        }
    })
    .await
    .unwrap_or(false)
}

/// Clause 18.7 local record codes; never communication-class Error PDUs.
/// Reserved/unmapped reasons, malformed/mismatching ACKs and local transport
/// failures use OTHER because they do not establish a more specific wire error.
pub(super) fn result<T>(outcome: &Result<T, Error>) -> Option<(ErrorClass, ErrorCode)> {
    let error = outcome.as_ref().err()?;
    let code = match error {
        Error::Protocol { class, code } => {
            return Some(match (u16::try_from(*class), u16::try_from(*code)) {
                (Ok(class), Ok(code)) => (ErrorClass::from_raw(class), ErrorCode::from_raw(code)),
                _ => (ErrorClass::COMMUNICATION, ErrorCode::OTHER),
            })
        }
        Error::Timeout(_) => ErrorCode::TIMEOUT,
        Error::Abort { reason } => match reason {
            0 => ErrorCode::ABORT_OTHER,
            1 => ErrorCode::ABORT_BUFFER_OVERFLOW,
            2 => ErrorCode::ABORT_INVALID_APDU_IN_THIS_STATE,
            3 => ErrorCode::ABORT_PREEMPTED_BY_HIGHER_PRIORITY_TASK,
            4 => ErrorCode::ABORT_SEGMENTATION_NOT_SUPPORTED,
            5 => ErrorCode::ABORT_SECURITY_ERROR,
            6 => ErrorCode::ABORT_INSUFFICIENT_SECURITY,
            7 => ErrorCode::ABORT_WINDOW_SIZE_OUT_OF_RANGE,
            8 => ErrorCode::ABORT_APPLICATION_EXCEEDED_REPLY_TIME,
            9 => ErrorCode::ABORT_OUT_OF_RESOURCES,
            10 => ErrorCode::ABORT_TSM_TIMEOUT,
            11 => ErrorCode::ABORT_APDU_TOO_LONG,
            64..=255 => ErrorCode::ABORT_PROPRIETARY,
            _ => ErrorCode::OTHER,
        },
        Error::Reject { reason } => match reason {
            0 => ErrorCode::REJECT_OTHER,
            1 => ErrorCode::REJECT_BUFFER_OVERFLOW,
            2 => ErrorCode::REJECT_INCONSISTENT_PARAMETERS,
            3 => ErrorCode::REJECT_INVALID_PARAMETER_DATA_TYPE,
            4 => ErrorCode::REJECT_INVALID_TAG,
            5 => ErrorCode::REJECT_MISSING_REQUIRED_PARAMETER,
            6 => ErrorCode::REJECT_PARAMETER_OUT_OF_RANGE,
            7 => ErrorCode::REJECT_TOO_MANY_ARGUMENTS,
            8 => ErrorCode::REJECT_UNDEFINED_ENUMERATION,
            9 => ErrorCode::REJECT_UNRECOGNIZED_SERVICE,
            64..=255 => ErrorCode::REJECT_PROPRIETARY,
            _ => ErrorCode::OTHER,
        },
        _ => ErrorCode::OTHER,
    };
    Some((ErrorClass::COMMUNICATION, code))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_read_result_codes_preserve_peer_errors_and_cover_reason_boundaries() {
        let abort_codes = [56, 51, 52, 53, 54, 136, 135, 127, 124, 125, 126, 123];
        let reject_codes = [69, 59, 60, 61, 62, 63, 64, 65, 66, 67];
        for (reason, code) in abort_codes.into_iter().enumerate() {
            assert_eq!(
                result::<()>(&Err(Error::Abort {
                    reason: reason as u8
                })),
                Some((ErrorClass::COMMUNICATION, ErrorCode::from_raw(code)))
            );
        }
        for (reason, code) in reject_codes.into_iter().enumerate() {
            assert_eq!(
                result::<()>(&Err(Error::Reject {
                    reason: reason as u8
                })),
                Some((ErrorClass::COMMUNICATION, ErrorCode::from_raw(code)))
            );
        }
        for reason in [12, 63] {
            assert_eq!(
                result::<()>(&Err(Error::Abort { reason })),
                Some((ErrorClass::COMMUNICATION, ErrorCode::OTHER))
            );
        }
        for reason in [10, 11, 63] {
            assert_eq!(
                result::<()>(&Err(Error::Reject { reason })),
                Some((ErrorClass::COMMUNICATION, ErrorCode::OTHER))
            );
        }
        for reason in [64, 255] {
            assert_eq!(
                result::<()>(&Err(Error::Abort { reason })),
                Some((ErrorClass::COMMUNICATION, ErrorCode::ABORT_PROPRIETARY))
            );
            assert_eq!(
                result::<()>(&Err(Error::Reject { reason })),
                Some((ErrorClass::COMMUNICATION, ErrorCode::REJECT_PROPRIETARY))
            );
        }
        assert_eq!(
            result::<()>(&Err(Error::Protocol {
                class: 512,
                code: 513
            })),
            Some((ErrorClass::from_raw(512), ErrorCode::from_raw(513)))
        );
        assert_eq!(
            result::<()>(&Err(Error::Protocol {
                class: u32::MAX,
                code: 513
            })),
            Some((ErrorClass::COMMUNICATION, ErrorCode::OTHER))
        );
        assert_eq!(result(&Ok(())), None);
    }
}
