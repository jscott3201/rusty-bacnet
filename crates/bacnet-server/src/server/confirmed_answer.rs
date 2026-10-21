//! The answer a device gives a confirmed request this server sent: a
//! confirmed notification, a WriteProperty a run makes in another device, or
//! the ReadProperty a Channel sends there to learn a member's datatype
//! (#1342).
//!
//! An answer that turns the request down keeps what it said (#1323): the
//! Error PDU's class and code, or the Reject or Abort reason. Notification
//! senders only ask whether the request was taken; a Channel writing a member
//! in another device needs the code too, since a NULL refused as an invalid
//! datatype still counts as written (Clause 12.53.7).

use bacnet_types::enums::{AbortReason, ErrorClass, ErrorCode, RejectReason};
use bacnet_types::error::Error;
use bytes::Bytes;

/// Result of a confirmed request from the receiving device's side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CovAckResult {
    /// SimpleAck received: the device took the request.
    Ack,
    /// An unsegmented ComplexAck received, with its service data: the device
    /// answered a read.
    Data(Bytes),
    /// Error, Reject or Abort received: the device turned the request down.
    Error(Refusal),
}

/// How a device turned down a confirmed request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// An Error PDU.
    Error {
        /// Its error class.
        class: ErrorClass,
        /// Its error code.
        code: ErrorCode,
    },
    /// A Reject PDU, with its reason.
    Reject(RejectReason),
    /// An Abort PDU, with its reason.
    Abort(AbortReason),
}

/// The error a client's WriteProperty answered this way reports:
/// [`Error::Protocol`], [`Error::Reject`] or [`Error::Abort`]. A `Refusal`
/// keeps only an Error's class and code, so it never becomes
/// [`Error::Structured`], which a client reports for the services whose
/// Error carries more.
impl From<Refusal> for Error {
    fn from(refusal: Refusal) -> Self {
        match refusal {
            Refusal::Error { class, code } => Self::Protocol { class, code },
            Refusal::Reject(reason) => Self::Reject { reason },
            Refusal::Abort(reason) => Self::Abort { reason },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_converts_to_the_error_a_client_request_reports() {
        let error = Refusal::Error {
            class: ErrorClass::PROPERTY,
            code: ErrorCode::INVALID_DATA_TYPE,
        };
        assert!(matches!(
            Error::from(error),
            Error::Protocol {
                class: ErrorClass::PROPERTY,
                code: ErrorCode::INVALID_DATA_TYPE
            }
        ));
        let reject = Refusal::Reject(RejectReason::INVALID_PARAMETER_DATA_TYPE);
        assert!(matches!(
            Error::from(reject),
            Error::Reject {
                reason: RejectReason::INVALID_PARAMETER_DATA_TYPE
            }
        ));
        let abort = Refusal::Abort(AbortReason::SEGMENTATION_NOT_SUPPORTED);
        assert!(matches!(
            Error::from(abort),
            Error::Abort {
                reason: AbortReason::SEGMENTATION_NOT_SUPPORTED
            }
        ));
    }
}
