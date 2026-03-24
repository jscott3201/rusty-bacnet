//! BACnet error types.
//!
//! Provides the top-level [`Error`] type used throughout the library,
//! covering protocol errors, encoding/decoding failures, transport issues,
//! and timeouts.

#[cfg(not(feature = "std"))]
use alloc::string::String;
#[cfg(feature = "std")]
use std::time::Duration;

use crate::enums::{ErrorClass, ErrorCode};

fn format_protocol_error(class: u32, code: u32) -> String {
    let class_name = ErrorClass::ALL_NAMED
        .iter()
        .find(|(_, v)| v.to_raw() as u32 == class)
        .map(|(n, _)| n.to_lowercase().replace('_', "-"));
    let code_name = ErrorCode::ALL_NAMED
        .iter()
        .find(|(_, v)| v.to_raw() as u32 == code)
        .map(|(n, _)| n.to_lowercase().replace('_', "-"));

    match (class_name, code_name) {
        (Some(cn), Some(co)) => format!("BACnet error: {cn} / {co}"),
        (Some(cn), None) => format!("BACnet error: {cn} / code={code}"),
        (None, Some(co)) => format!("BACnet error: class={class} / {co}"),
        (None, None) => format!("BACnet error: class={class} / code={code}"),
    }
}

/// Top-level error type for the BACnet library.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// BACnet protocol error response (Clause 20.1.7).
    #[error("{}", format_protocol_error(*.class, *.code))]
    Protocol {
        /// Error class value.
        class: u32,
        /// Error code value.
        code: u32,
    },

    /// BACnet reject PDU (Clause 20.1.5).
    #[error("BACnet reject: reason={reason}")]
    Reject {
        /// Reject reason value.
        reason: u8,
    },

    /// BACnet abort PDU (Clause 20.1.6).
    #[error("BACnet abort: reason={reason}")]
    Abort {
        /// Abort reason value.
        reason: u8,
    },

    /// Error encoding a PDU.
    #[error("encoding error: {0}")]
    Encoding(String),

    /// Error decoding received data.
    #[error("decoding error at offset {offset}: {message}")]
    Decoding {
        /// Byte offset where the error occurred.
        offset: usize,
        /// Description of what went wrong.
        message: String,
    },

    /// Transport-level I/O error.
    #[cfg(feature = "std")]
    #[error("transport error: {0}")]
    Transport(#[from] std::io::Error),

    /// Request timed out.
    #[cfg(feature = "std")]
    #[error("request timed out after {0:?}")]
    Timeout(Duration),

    /// Segmentation assembly error.
    #[error("segmentation error: {0}")]
    Segmentation(String),

    /// Buffer too short for the expected data.
    #[error("buffer too short: need {need} bytes, have {have}")]
    BufferTooShort {
        /// Minimum bytes needed.
        need: usize,
        /// Bytes actually available.
        have: usize,
    },

    /// Invalid tag encountered during decode.
    #[error("invalid tag: {0}")]
    InvalidTag(String),

    /// Value out of valid range.
    #[error("value out of range: {0}")]
    OutOfRange(String),
}

/// Convenience alias for `Result<T, Error>`.
pub type Result<T> = core::result::Result<T, Error>;

impl Error {
    /// Create a decoding error at the given byte offset.
    pub fn decoding(offset: usize, message: impl Into<String>) -> Self {
        Self::Decoding {
            offset,
            message: message.into(),
        }
    }

    /// Create a buffer-too-short error.
    pub fn buffer_too_short(need: usize, have: usize) -> Self {
        Self::BufferTooShort { need, have }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_error_display() {
        let err = Error::Protocol { class: 2, code: 31 };
        assert!(err.to_string().contains("property"));
        assert!(err.to_string().contains("unknown-object"));

        // Unknown class/code falls back to numeric
        let err2 = Error::Protocol {
            class: 999,
            code: 999,
        };
        assert!(err2.to_string().contains("class=999"));
        assert!(err2.to_string().contains("code=999"));
    }

    #[test]
    fn decoding_error_display() {
        let err = Error::decoding(42, "unexpected tag");
        assert!(err.to_string().contains("offset 42"));
        assert!(err.to_string().contains("unexpected tag"));
    }

    #[test]
    fn buffer_too_short_display() {
        let err = Error::buffer_too_short(10, 3);
        assert!(err.to_string().contains("need 10"));
        assert!(err.to_string().contains("have 3"));
    }

    #[cfg(feature = "std")]
    #[test]
    fn timeout_error_display() {
        let err = Error::Timeout(Duration::from_secs(3));
        assert!(err.to_string().contains("3s"));
    }
}
