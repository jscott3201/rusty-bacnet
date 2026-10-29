//! How a refusal PDU reads as a [`RemoteError`] (#2026).
//!
//! The client's dispatch, its endpoint requester and the server's
//! notification transactions all turn an Error, Reject or Abort PDU into the
//! same [`RemoteError`]; these helpers are the one place that mapping lives.

use bacnet_types::error::RemoteError;

use super::{AbortPdu, Apdu, ErrorPdu, RejectPdu};

impl ErrorPdu {
    /// The refusal as a [`RemoteError::Error`]: the class and code.
    ///
    /// Any service-specific body after the class and code (`error_data`) is
    /// not part of it; a caller that reports a structured error reads that
    /// body itself.
    #[must_use]
    pub const fn remote_error(&self) -> RemoteError {
        RemoteError::Error {
            class: self.error_class,
            code: self.error_code,
        }
    }
}

impl RejectPdu {
    /// The refusal as a [`RemoteError::Reject`] carrying the reason.
    #[must_use]
    pub const fn remote_error(&self) -> RemoteError {
        RemoteError::Reject {
            reason: self.reject_reason,
        }
    }
}

impl AbortPdu {
    /// The refusal as a [`RemoteError::Abort`] carrying the reason.
    ///
    /// It doesn't look at `sent_by_server`: whether an Abort may end a
    /// transaction is the caller's decision.
    #[must_use]
    pub const fn remote_error(&self) -> RemoteError {
        RemoteError::Abort {
            reason: self.abort_reason,
        }
    }
}

impl Apdu {
    /// How the peer turned the request down, for an Error, Reject or Abort
    /// PDU; `None` for every other PDU type.
    ///
    /// A convenience for code holding an `Apdu` of unknown type; the stack's
    /// own dispatch has already matched the PDU type and calls the
    /// per-PDU `remote_error()` instead.
    #[must_use]
    pub const fn remote_error(&self) -> Option<RemoteError> {
        match self {
            Self::Error(pdu) => Some(pdu.remote_error()),
            Self::Reject(pdu) => Some(pdu.remote_error()),
            Self::Abort(pdu) => Some(pdu.remote_error()),
            Self::ConfirmedRequest(_)
            | Self::UnconfirmedRequest(_)
            | Self::SimpleAck(_)
            | Self::ComplexAck(_)
            | Self::SegmentAck(_) => None,
        }
    }
}
