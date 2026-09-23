//! Bounded source notification delivery and local communication-result mapping.
use super::*;
use bacnet_encoding::apdu::{encode_apdu, Apdu, ConfirmedRequest, UnconfirmedRequest};
use bacnet_endpoint_core::coordinator::CanonicalPeer;
use bacnet_server::server::{
    __endpoint_NotificationWorkerResult as NotificationWorkerResult,
    __endpoint_run_notification_worker as run_notification_worker,
};
use bacnet_services::audit::AuditNotificationRequest;
use bacnet_transport::bvll::encode_bip_mac;
use bacnet_types::enums::{
    ConfirmedServiceChoice, ErrorClass, ErrorCode, NetworkPriority, UnconfirmedServiceChoice,
};
use bacnet_types::MacAddr;
use bytes::BytesMut;
use tokio::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(3);

struct Completion {
    status: Arc<AuditReporterStatus>,
    epoch: u64,
    finished: bool,
}
impl Completion {
    fn new(status: Arc<AuditReporterStatus>) -> Self {
        let epoch = status.begin_delivery();
        Self {
            status,
            epoch,
            finished: false,
        }
    }
    fn finish(mut self, delivered: bool) {
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
    owner: &NotificationTransactions,
    egress: EndpointEgress,
    recipient: StaticSourceAuditRecipient,
    max_apdu: u16,
    confirmed: bool,
    notification: BACnetAuditNotification,
    status: Arc<AuditReporterStatus>,
) {
    let completion = Completion::new(status);
    let Some(permit) = owner.try_admit_audit() else {
        return;
    };
    let mac = MacAddr::from_slice(&encode_bip_mac(
        recipient.address.ip().octets(),
        recipient.address.port(),
    ));
    let reserved = if confirmed {
        match owner.reserve(
            CanonicalPeer::direct(mac.as_slice()),
            ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
        ) {
            Ok(reserved) => Some(reserved),
            Err(_) => return,
        }
    } else {
        None
    };
    let mut service = BytesMut::new();
    if (AuditNotificationRequest {
        notifications: vec![notification],
    })
    .try_encode(&mut service)
    .is_err()
    {
        return;
    }
    let pdu = if let Some((operation, _)) = &reserved {
        Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: max_apdu,
            invoke_id: operation.invoke_id(),
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
    if encode_apdu(&mut encoded, &pdu).is_err() || encoded.len() > usize::from(max_apdu) {
        return;
    }
    let deadline = Instant::now() + DEADLINE;
    owner.spawn(async move {
        let _permit = permit;
        let send = || async {
            egress
                .admit_apdu(
                    encoded.to_vec(),
                    EndpointApduDestination::Direct {
                        destination_mac: mac.clone(),
                    },
                    confirmed,
                    NetworkPriority::NORMAL,
                    Vec::new(),
                    Some(deadline),
                )?
                .complete()
                .await
                .result
        };
        let delivered = tokio::time::timeout_at(deadline, async {
            if let Some((operation, receiver)) = reserved {
                run_notification_worker(operation, receiver, DEADLINE, 0, |_| send()).await
                    == NotificationWorkerResult::Ack
            } else {
                send().await.is_ok()
            }
        })
        .await
        .unwrap_or(false);
        completion.finish(delivered);
    });
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
