//! The reply PDUs, and the one map from an error to its Reject or Error PDU
//! (Clauses 18.9, 20.1.7 and 21).
use bacnet_encoding::apdu::{Apdu, ErrorPdu, RejectPdu};
use bacnet_types::enums::{ConfirmedServiceChoice, ErrorClass, ErrorCode};
use bacnet_types::error::{Error, ErrorDetail, RemoteError};
use bytes::Bytes;

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
