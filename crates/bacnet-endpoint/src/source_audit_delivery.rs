//! Bounded source notification delivery and local communication-result mapping.
use super::*;
use bacnet_encoding::apdu::{Apdu, ConfirmedRequest, UnconfirmedRequest, encode_apdu};
use bacnet_endpoint_core::coordinator::CanonicalPeer;
use bacnet_endpoint_core::endpoint_ingress::EndpointEgressAdmissionError;
use bacnet_npdu::NetworkPriority;
use bacnet_server::server::{
    __endpoint_NotificationWorkerResult as NotificationWorkerResult,
    __endpoint_run_notification_worker as run_notification_worker,
};
use bacnet_services::audit::AuditNotificationRequest;
use bacnet_types::MacAddr;
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, ErrorClass, ErrorCode, RejectReason,
    UnconfirmedServiceChoice,
};
use bacnet_types::error::RemoteError;
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
    source: &Arc<SourceAudit>,
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
    // A source record carries the source timestamp; a record of an
    // operation on this device, the target timestamp.
    let timestamp = notification
        .source_timestamp
        .clone()
        .or_else(|| notification.target_timestamp.clone())
        .expect("record timestamp");
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
        if matches!(sent, Err(EndpointEgressAdmissionError::QueueFull))
            && let (Some(source), Some(owner)) = (source.upgrade(), weak_owner.upgrade())
        {
            failures::record_drop(&source, &owner, failure, timestamp);
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
        NetworkPriority::Normal,
        Vec::new(),
        Some(deadline),
    )
}

/// Queue one encoded UnconfirmedAuditNotification for global broadcast
/// (DNET 0xFFFF): a recipient change whose old recipient has no route goes
/// out this way too, so it reaches every listening logger (Clause 12.11.66).
pub(super) fn admit_global_broadcast(
    egress: &EndpointEgress,
    encoded: Vec<u8>,
    deadline: Instant,
) -> Result<bacnet_endpoint_core::endpoint_ingress::EndpointSend, EndpointEgressAdmissionError> {
    egress.admit_apdu(
        encoded,
        EndpointApduDestination::GlobalBroadcast,
        false,
        NetworkPriority::Normal,
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
        Error::Remote(RemoteError::Error { class, code }) => return Some((*class, *code)),
        Error::Timeout(_) => ErrorCode::TIMEOUT,
        Error::Remote(RemoteError::Abort { reason }) => match *reason {
            AbortReason::OTHER => ErrorCode::ABORT_OTHER,
            AbortReason::BUFFER_OVERFLOW => ErrorCode::ABORT_BUFFER_OVERFLOW,
            AbortReason::INVALID_APDU_IN_THIS_STATE => ErrorCode::ABORT_INVALID_APDU_IN_THIS_STATE,
            AbortReason::PREEMPTED_BY_HIGHER_PRIORITY_TASK => {
                ErrorCode::ABORT_PREEMPTED_BY_HIGHER_PRIORITY_TASK
            }
            AbortReason::SEGMENTATION_NOT_SUPPORTED => ErrorCode::ABORT_SEGMENTATION_NOT_SUPPORTED,
            AbortReason::SECURITY_ERROR => ErrorCode::ABORT_SECURITY_ERROR,
            AbortReason::INSUFFICIENT_SECURITY => ErrorCode::ABORT_INSUFFICIENT_SECURITY,
            AbortReason::WINDOW_SIZE_OUT_OF_RANGE => ErrorCode::ABORT_WINDOW_SIZE_OUT_OF_RANGE,
            AbortReason::APPLICATION_EXCEEDED_REPLY_TIME => {
                ErrorCode::ABORT_APPLICATION_EXCEEDED_REPLY_TIME
            }
            AbortReason::OUT_OF_RESOURCES => ErrorCode::ABORT_OUT_OF_RESOURCES,
            AbortReason::TSM_TIMEOUT => ErrorCode::ABORT_TSM_TIMEOUT,
            AbortReason::APDU_TOO_LONG => ErrorCode::ABORT_APDU_TOO_LONG,
            reason if reason.to_raw() >= 64 => ErrorCode::ABORT_PROPRIETARY,
            _ => ErrorCode::OTHER,
        },
        Error::Remote(RemoteError::Reject { reason }) => match *reason {
            RejectReason::OTHER => ErrorCode::REJECT_OTHER,
            RejectReason::BUFFER_OVERFLOW => ErrorCode::REJECT_BUFFER_OVERFLOW,
            RejectReason::INCONSISTENT_PARAMETERS => ErrorCode::REJECT_INCONSISTENT_PARAMETERS,
            RejectReason::INVALID_PARAMETER_DATA_TYPE => {
                ErrorCode::REJECT_INVALID_PARAMETER_DATA_TYPE
            }
            RejectReason::INVALID_TAG => ErrorCode::REJECT_INVALID_TAG,
            RejectReason::MISSING_REQUIRED_PARAMETER => {
                ErrorCode::REJECT_MISSING_REQUIRED_PARAMETER
            }
            RejectReason::PARAMETER_OUT_OF_RANGE => ErrorCode::REJECT_PARAMETER_OUT_OF_RANGE,
            RejectReason::TOO_MANY_ARGUMENTS => ErrorCode::REJECT_TOO_MANY_ARGUMENTS,
            RejectReason::UNDEFINED_ENUMERATION => ErrorCode::REJECT_UNDEFINED_ENUMERATION,
            RejectReason::UNRECOGNIZED_SERVICE => ErrorCode::REJECT_UNRECOGNIZED_SERVICE,
            reason if reason.to_raw() >= 64 => ErrorCode::REJECT_PROPRIETARY,
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
    fn source_audit_result_codes_preserve_peer_errors_and_cover_reason_boundaries() {
        let abort_codes = [56, 51, 52, 53, 54, 136, 135, 127, 124, 125, 126, 123];
        let reject_codes = [69, 59, 60, 61, 62, 63, 64, 65, 66, 67];
        for (reason, code) in abort_codes.into_iter().enumerate() {
            assert_eq!(
                result::<()>(&Err(Error::abort(AbortReason::from_raw(reason as u8)))),
                Some((ErrorClass::COMMUNICATION, ErrorCode::from_raw(code)))
            );
        }
        for (reason, code) in reject_codes.into_iter().enumerate() {
            assert_eq!(
                result::<()>(&Err(Error::reject(RejectReason::from_raw(reason as u8)))),
                Some((ErrorClass::COMMUNICATION, ErrorCode::from_raw(code)))
            );
        }
        for reason in [12, 63] {
            assert_eq!(
                result::<()>(&Err(Error::abort(AbortReason::from_raw(reason)))),
                Some((ErrorClass::COMMUNICATION, ErrorCode::OTHER))
            );
        }
        for reason in [10, 11, 63] {
            assert_eq!(
                result::<()>(&Err(Error::reject(RejectReason::from_raw(reason)))),
                Some((ErrorClass::COMMUNICATION, ErrorCode::OTHER))
            );
        }
        for reason in [64, 255] {
            assert_eq!(
                result::<()>(&Err(Error::abort(AbortReason::from_raw(reason)))),
                Some((ErrorClass::COMMUNICATION, ErrorCode::ABORT_PROPRIETARY))
            );
            assert_eq!(
                result::<()>(&Err(Error::reject(RejectReason::from_raw(reason)))),
                Some((ErrorClass::COMMUNICATION, ErrorCode::REJECT_PROPRIETARY))
            );
        }
        assert_eq!(
            result::<()>(&Err(Error::protocol(
                ErrorClass::from_raw(512),
                ErrorCode::from_raw(513)
            ))),
            Some((ErrorClass::from_raw(512), ErrorCode::from_raw(513)))
        );
        assert_eq!(result(&Ok(())), None);
    }
}
