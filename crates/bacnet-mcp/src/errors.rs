//! Tool errors as structured results (design §5.2).
//!
//! A tool that ran and failed returns `isError: true` with one of these
//! bodies; an unknown tool or arguments that don't fit the input schema are
//! JSON-RPC errors instead. "Refused, nothing was sent" (`refused`,
//! `invalid-argument`, `unknown-device`) and "sent, then failed" (`bacnet`,
//! `timeout`, `failed`) are always distinct, because an agent's retry logic
//! depends on the difference.

use bacnet_ops::format::RowError;
use bacnet_ops::label::kebab;
use bacnet_types::enums::{AbortReason, ErrorClass, ErrorCode};
use bacnet_types::error::{Error, RemoteError};
use serde_json::{Value, json};

/// Why a tool call failed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ToolError {
    /// A target, object, property or other argument that doesn't parse.
    /// Nothing was sent.
    Invalid {
        field: &'static str,
        message: String,
    },
    /// The device answered with an Error, Reject or Abort PDU.
    Bacnet(RemoteError),
    /// The device didn't answer in time, or the call ran out of its
    /// budget after it had sent requests (no `device` then).
    Timeout { device: Option<u32>, after_ms: u64 },
    /// The policy refused the call. Nothing was sent.
    Refused {
        reason: &'static str,
        message: String,
        retry_after_s: Option<u64>,
    },
    /// The device isn't in the discovery table. Nothing was sent.
    UnknownDevice { device: String, hint: String },
    /// Any other failure, such as the transport's.
    Failed(String),
    /// Writes that went out, or were meant to, and didn't all take: the
    /// body names each write's outcome, and `error` says what went wrong
    /// (`timeout` when one may or may not have been applied).
    Write(Value),
}

impl ToolError {
    /// An argument error for `field`.
    pub(crate) fn invalid(field: &'static str, message: impl Into<String>) -> Self {
        Self::Invalid {
            field,
            message: message.into(),
        }
    }

    /// A policy refusal.
    pub(crate) fn refused(reason: &'static str, message: impl Into<String>) -> Self {
        Self::Refused {
            reason,
            message: message.into(),
            retry_after_s: None,
        }
    }

    /// A request to `device` that failed with `error`.
    pub(crate) fn device(device: u32, error: &Error) -> Self {
        match (error, error.remote()) {
            (_, Some(RemoteError::Abort { reason })) if reason == AbortReason::TSM_TIMEOUT => {
                Self::Timeout {
                    device: Some(device),
                    after_ms: 0,
                }
            }
            (_, Some(remote)) => Self::Bacnet(remote),
            (Error::Timeout(after), None) => Self::Timeout {
                device: Some(device),
                after_ms: u64::try_from(after.as_millis()).unwrap_or(u64::MAX),
            },
            (other, None) => Self::Failed(other.to_string()),
        }
    }

    /// The structured body.
    pub(crate) fn json(&self) -> Value {
        match self {
            Self::Invalid { field, message } => {
                json!({ "error": "invalid-argument", "field": field, "message": message })
            }
            Self::Bacnet(remote) => {
                let mut body = remote_json(remote);
                body["error"] = json!("bacnet");
                body
            }
            Self::Timeout { device, after_ms } => {
                let mut body = json!({ "error": "timeout", "after_ms": after_ms });
                if let Some(device) = device {
                    body["device"] = json!(device);
                }
                body
            }
            Self::Refused {
                reason,
                message,
                retry_after_s,
            } => {
                let mut body = json!({ "error": "refused", "reason": reason, "message": message });
                if let Some(after) = retry_after_s {
                    body["retry_after_s"] = json!(after);
                }
                body
            }
            Self::UnknownDevice { device, hint } => {
                json!({ "error": "unknown-device", "target": device, "hint": hint })
            }
            Self::Failed(message) => json!({ "error": "failed", "message": message }),
            Self::Write(body) => body.clone(),
        }
    }

    /// One line for the text rendering.
    pub(crate) fn summary(&self) -> String {
        match self {
            Self::Invalid { field, message } => format!("invalid {field}: {message}"),
            Self::Bacnet(remote) => format!("the device answered: {remote}"),
            Self::Timeout {
                device: Some(device),
                ..
            } => format!("device {device} did not answer in time"),
            Self::Timeout { device: None, .. } => {
                "the call ran out of time after sending requests".into()
            }
            Self::Refused {
                reason, message, ..
            } => format!("refused ({reason}), nothing was sent: {message}"),
            Self::UnknownDevice { device, hint } => {
                format!("device {device} is not in the discovery table; {hint}")
            }
            Self::Failed(message) => format!("failed: {message}"),
            Self::Write(body) => format!(
                "not every write took ({}); see each write's outcome",
                body["error"].as_str().unwrap_or("failed")
            ),
        }
    }
}

/// A device's Error, Reject or Abort as data: each name in kebab case
/// beside its number. A refusal by a device that doesn't grant the service
/// (services / service-request-denied) is marked `denied`, as the TUI
/// keeps it apart.
pub(crate) fn remote_json(remote: &RemoteError) -> Value {
    match *remote {
        RemoteError::Error { class, code } => {
            let mut body = json!({
                "result": "error",
                "class": kebab(class),
                "class_number": class.to_raw(),
                "code": kebab(code),
                "code_number": code.to_raw(),
            });
            if class == ErrorClass::SERVICES && code == ErrorCode::SERVICE_REQUEST_DENIED {
                body["denied"] = json!(true);
            }
            body
        }
        RemoteError::Reject { reason } => json!({
            "result": "reject",
            "reason": kebab(reason),
            "reason_number": reason.to_raw(),
        }),
        RemoteError::Abort { reason } => json!({
            "result": "abort",
            "reason": kebab(reason),
            "reason_number": reason.to_raw(),
        }),
    }
}

/// Why one property of a read has no value, as data.
pub(crate) fn row_error_json(error: &RowError) -> Value {
    match error {
        RowError::Remote(remote) => remote_json(remote),
        RowError::NoValue => json!({ "result": "no-value" }),
        other => json!({ "result": "failed", "message": other.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::RejectReason;

    #[test]
    fn device_errors_keep_their_names_and_numbers() {
        let missing = RemoteError::Error {
            class: ErrorClass::PROPERTY,
            code: ErrorCode::UNKNOWN_PROPERTY,
        };
        assert_eq!(
            ToolError::Bacnet(missing).json(),
            json!({ "error": "bacnet", "result": "error", "class": "property",
                    "class_number": 2, "code": "unknown-property", "code_number": 32 })
        );
        let denied = RemoteError::Error {
            class: ErrorClass::SERVICES,
            code: ErrorCode::SERVICE_REQUEST_DENIED,
        };
        assert_eq!(remote_json(&denied)["denied"], json!(true));
        let reject = RemoteError::Reject {
            reason: RejectReason::UNRECOGNIZED_SERVICE,
        };
        assert_eq!(
            remote_json(&reject),
            json!({ "result": "reject", "reason": "unrecognized-service", "reason_number": 9 })
        );
        assert_eq!(
            row_error_json(&RowError::NoValue),
            json!({ "result": "no-value" })
        );
    }

    #[test]
    fn a_timeout_is_not_a_device_answer() {
        let error = Error::Timeout(std::time::Duration::from_millis(6000));
        assert_eq!(
            ToolError::device(7, &error),
            ToolError::Timeout {
                device: Some(7),
                after_ms: 6000
            }
        );
        let abort = Error::Remote(RemoteError::Abort {
            reason: AbortReason::TSM_TIMEOUT,
        });
        assert!(matches!(
            ToolError::device(7, &abort),
            ToolError::Timeout { .. }
        ));
    }
}
