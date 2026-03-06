use bacnet_types::error::Error;

/// BACnet error types exposed to Java/Kotlin via UniFFI.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum BacnetError {
    #[error("Protocol error: class={error_class}, code={error_code}")]
    Protocol { error_class: u32, error_code: u32 },

    #[error("Timeout: {msg}")]
    Timeout { msg: String },

    #[error("Reject: reason={reason}")]
    Reject { reason: u32 },

    #[error("Abort: reason={reason}")]
    Abort { reason: u32 },

    #[error("Transport error: {msg}")]
    Transport { msg: String },

    #[error("Encoding error: {msg}")]
    Encoding { msg: String },

    #[error("Invalid argument: {msg}")]
    InvalidArgument { msg: String },

    #[error("Not started")]
    NotStarted,
}

impl From<Error> for BacnetError {
    fn from(e: Error) -> Self {
        match e {
            Error::Timeout(_) => BacnetError::Timeout { msg: e.to_string() },
            Error::Protocol { class, code } => BacnetError::Protocol {
                error_class: class,
                error_code: code,
            },
            Error::Reject { reason } => BacnetError::Reject {
                reason: reason as u32,
            },
            Error::Abort { reason } => BacnetError::Abort {
                reason: reason as u32,
            },
            Error::Transport(_) => BacnetError::Transport { msg: e.to_string() },
            Error::Encoding(msg) => BacnetError::Encoding { msg },
            Error::Segmentation(msg) => BacnetError::Transport { msg },
            Error::BufferTooShort { need, have } => BacnetError::Encoding {
                msg: format!("buffer too short: need {need}, have {have}"),
            },
            Error::Decoding { message, .. } => BacnetError::Encoding { msg: message },
            Error::InvalidTag(msg) => BacnetError::Encoding { msg },
            Error::OutOfRange(msg) => BacnetError::InvalidArgument { msg },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_timeout_conversion() {
        let e: BacnetError = Error::Timeout(Duration::from_secs(6)).into();
        assert!(matches!(e, BacnetError::Timeout { .. }));
        assert!(e.to_string().contains("Timeout"));
    }

    #[test]
    fn test_protocol_conversion() {
        let e: BacnetError = Error::Protocol { class: 2, code: 31 }.into();
        match e {
            BacnetError::Protocol {
                error_class,
                error_code,
            } => {
                assert_eq!(error_class, 2);
                assert_eq!(error_code, 31);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_reject_conversion() {
        let e: BacnetError = Error::Reject { reason: 4 }.into();
        match e {
            BacnetError::Reject { reason } => assert_eq!(reason, 4),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_abort_conversion() {
        let e: BacnetError = Error::Abort { reason: 2 }.into();
        match e {
            BacnetError::Abort { reason } => assert_eq!(reason, 2),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_encoding_conversion() {
        let e: BacnetError = Error::Encoding("bad tag".into()).into();
        assert!(matches!(e, BacnetError::Encoding { .. }));
    }

    #[test]
    fn test_buffer_too_short_conversion() {
        let e: BacnetError = Error::BufferTooShort { need: 10, have: 5 }.into();
        match e {
            BacnetError::Encoding { msg } => {
                assert!(msg.contains("10"));
                assert!(msg.contains("5"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_out_of_range_conversion() {
        let e: BacnetError = Error::OutOfRange("value too big".into()).into();
        assert!(matches!(e, BacnetError::InvalidArgument { .. }));
    }
}
