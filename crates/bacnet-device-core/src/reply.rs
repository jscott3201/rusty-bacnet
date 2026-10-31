//! The reply PDUs, and the one map from an error to its Reject or Error PDU
//! (Clauses 18.9, 18.10, 20.1.7 and 21).
use crate::responder::Outcome;
use bacnet_encoding::apdu::{AbortPdu, Apdu, ErrorPdu, RejectPdu, encode_apdu};
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, ErrorClass, ErrorCode, PduType, RejectReason,
};
use bacnet_types::error::{Error, ErrorDetail, RemoteError};
use bytes::{Bytes, BytesMut};

/// The reply to a confirmed request `error` refused.
///
/// An [`Error::reject`] draws a Reject PDU with its reason, for every
/// service, the formal-error ones included: a Reject has no
/// service-specific body. Each handler turns the error from decoding the
/// request into one with [`Error::into_request_reject`], so a syntax fault
/// in the request draws the Reject naming it (Clauses 18.9 and 20.1.8,
/// #1446). Any other refusal is an Error PDU, in the formal Clause 21
/// production for the services that have one; that includes a decoding
/// error met once the service is running, which is no syntax fault of the
/// request.
pub fn error_apdu(invoke_id: u8, service_choice: ConfirmedServiceChoice, error: &Error) -> Apdu {
    if let Error::Remote(RemoteError::Reject { reason }) = error {
        return Apdu::Reject(RejectPdu {
            invoke_id,
            reject_reason: *reason,
        });
    }
    let (error_class, error_code) = error_fields(error);
    let detail = match error {
        Error::Structured { detail, .. } => Some(detail.as_ref()),
        _ => None,
    };
    // Only an element refusal names an element; a refusal of the request or
    // its target (authorization, object, property, index, list-ness, an
    // object the server cannot create) carries zero (Clauses 15.1.1.3.1,
    // 15.2.1.3.1 and 15.3.1.3).
    let first_failed_element_number = match detail {
        Some(ErrorDetail::FirstFailedElementNumber(number)) => *number,
        _ => 0,
    };
    // These services answer every error with their Clause 21 production.
    if service_choice == ConfirmedServiceChoice::ADD_LIST_ELEMENT
        || service_choice == ConfirmedServiceChoice::REMOVE_LIST_ELEMENT
    {
        return Apdu::Error(
            bacnet_services::list_manipulation::ChangeListError {
                error_class,
                error_code,
                first_failed_element_number,
            }
            .to_error_pdu(invoke_id, service_choice),
        );
    }
    if service_choice == ConfirmedServiceChoice::CREATE_OBJECT {
        return Apdu::Error(
            bacnet_services::object_mgmt::CreateObjectError {
                error_class,
                error_code,
                first_failed_element_number,
            }
            .to_error_pdu(invoke_id),
        );
    }
    if service_choice == ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE {
        // A refusal of one COV reference names it; any failure before the
        // references are processed is the general choice (Clause 13.16.2).
        let first_failed_subscription = match detail {
            Some(ErrorDetail::FirstFailedSubscription(reference)) => Some(reference.clone()),
            _ => None,
        };
        return Apdu::Error(
            bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleError {
                error_class,
                error_code,
                first_failed_subscription,
            }
            .to_error_pdu(invoke_id),
        );
    }
    Apdu::Error(ErrorPdu {
        invoke_id,
        service_choice,
        error_class,
        error_code,
        error_data: Bytes::new(),
    })
}

/// The class and code an Error PDU carries for `error`: its own, or
/// SERVICES / OTHER for an error that names none.
pub fn error_fields(error: &Error) -> (ErrorClass, ErrorCode) {
    match error {
        Error::Remote(RemoteError::Error { class, code })
        | Error::Structured { class, code, .. } => (*class, *code),
        _ => (ErrorClass::SERVICES, ErrorCode::OTHER),
    }
}

/// Reject UNRECOGNIZED_SERVICE: the answer to a confirmed request for a
/// service the device doesn't run (Clause 18.9).
pub fn unrecognized_service(invoke_id: u8) -> Apdu {
    Apdu::Reject(RejectPdu {
        invoke_id,
        reject_reason: RejectReason::UNRECOGNIZED_SERVICE,
    })
}

/// Abort SEGMENTATION_NOT_SUPPORTED, sent by the server: the answer to a
/// segmented request, and to a reply that would need segmenting, from a
/// device that segments neither way (Clauses 5.4.5.3 and 18.10).
pub fn refuse_segmented(invoke_id: u8) -> Apdu {
    Apdu::Abort(AbortPdu {
        sent_by_server: true,
        invoke_id,
        abort_reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
    })
}

/// `reply`, unless it is a ComplexACK longer than `max_reply` octets, which
/// would need segmenting: then [`refuse_segmented`] in its place. Every
/// other reply passes through.
pub fn fit(reply: Apdu, max_reply: u16) -> Apdu {
    if let Apdu::ComplexAck(ack) = &reply {
        // The header the APDU encoder writes: type and flags, invoke ID and
        // service choice, with the sequence number and window when segmented.
        let header = if ack.segmented { 5 } else { 3 };
        if header + ack.service_ack.len() > usize::from(max_reply) {
            return refuse_segmented(ack.invoke_id);
        }
    }
    reply
}

/// Answer `apdu`, a request the caller doesn't run, without decoding its
/// service: a confirmed request gets [`refuse_segmented`] when it arrived
/// segmented and [`unrecognized_service`] otherwise, written to `out`;
/// anything else, or an APDU too short to name its invoke ID, is
/// [`Outcome::Silent`].
pub fn refuse(apdu: &[u8], out: &mut BytesMut) -> Outcome {
    let (Some(&first), Some(&invoke_id)) = (apdu.first(), apdu.get(2)) else {
        return Outcome::Silent;
    };
    if first >> 4 != PduType::CONFIRMED_REQUEST.to_raw() {
        return Outcome::Silent;
    }
    let reply = if first & SEGMENTED != 0 {
        refuse_segmented(invoke_id)
    } else {
        unrecognized_service(invoke_id)
    };
    emit(out, &reply, Outcome::Reply)
}

/// The segmented-message flag in a confirmed request's first octet.
pub(crate) const SEGMENTED: u8 = 0x08;

/// Encode `apdu` onto `out` and report `outcome`; [`Outcome::Silent`], with
/// `out` as it was, when it doesn't encode.
pub(crate) fn emit(out: &mut BytesMut, apdu: &Apdu, outcome: Outcome) -> Outcome {
    let start = out.len();
    match encode_apdu(out, apdu) {
        Ok(()) => outcome,
        Err(_) => {
            out.truncate(start);
            Outcome::Silent
        }
    }
}
