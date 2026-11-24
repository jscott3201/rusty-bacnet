//! Tool errors as structured results (design §5.2).
//!
//! A tool that ran and failed returns `isError: true` with one of these
//! bodies; an unknown tool or arguments that don't fit the input schema are
//! JSON-RPC errors instead. "Refused, nothing was sent" (`refused`,
//! `invalid-argument`, `unknown-device`) and "sent, then failed" (`bacnet`,
//! `network-reject`, `timeout`, `failed`) are always distinct, because an
//! agent's retry logic depends on the difference.

use bacnet_ops::label::kebab;
use bacnet_ops::{DeviceAddress, OpsError, PropertyError};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::{Error, NetworkReject, RemoteError};
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
    /// budget after it had sent requests (no `device` then). `busy` is the
    /// busy reject (reason 2) a router on the way sent first, if one did
    /// (#2724).
    Timeout {
        device: Option<u32>,
        after_ms: u64,
        busy: Option<NetworkReject>,
    },
    /// A router on the way refused the request to `device` at the network
    /// layer (#2724). The device never received it when the request sent
    /// one frame; after a retry or a segment, an earlier attempt may have
    /// reached it.
    NetworkReject { device: u32, reject: NetworkReject },
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
    /// `error`, met while reading `object` (`analog-input:1`), with the way
    /// past it in `hint`: what a tool that reads object by object says when
    /// one object alone outlasts the call (#2779). Its body is `error`'s,
    /// with `object` and `hint` beside.
    At {
        error: Box<ToolError>,
        object: String,
        hint: String,
    },
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

    /// The device `target` names, as written (`7`, `2:7`), is claimed by
    /// every address of `candidates` (#2560): refused as ambiguous, naming
    /// them. Nothing was sent.
    pub(crate) fn ambiguous(target: &str, candidates: &[DeviceAddress]) -> Self {
        let claims: Vec<String> = candidates.iter().map(ToString::to_string).collect();
        Self::refused(
            "ambiguous-device",
            format!(
                "device {target} is claimed by {}; name one by its address",
                claims.join(" and ")
            ),
        )
    }

    /// A request to `device` that failed with `error`: a timeout (the
    /// client's own, with how long it waited, or the state machine's
    /// abort, with 0), a router's network reject, the device's answer, or
    /// anything else as text.
    pub(crate) fn device(device: u32, error: &OpsError) -> Self {
        match error {
            OpsError::Ambiguous { candidates, .. } => {
                Self::ambiguous(&device.to_string(), candidates)
            }
            OpsError::Unreachable {
                source: Error::NetworkRejected(reject),
                ..
            }
            | OpsError::Client(Error::NetworkRejected(reject)) => Self::NetworkReject {
                device,
                reject: **reject,
            },
            error if error.timed_out() => {
                let (after, busy) = match error {
                    OpsError::Unreachable {
                        source: Error::Timeout(after),
                        ..
                    }
                    | OpsError::Client(Error::Timeout(after)) => (after.as_millis(), None),
                    OpsError::Unreachable {
                        source: Error::RouterBusyTimeout(busy),
                        ..
                    }
                    | OpsError::Client(Error::RouterBusyTimeout(busy)) => (0, Some(**busy)),
                    _ => (0, None),
                };
                Self::Timeout {
                    device: Some(device),
                    after_ms: u64::try_from(after).unwrap_or(u64::MAX),
                    busy,
                }
            }
            error => match error.remote() {
                Some(remote) => Self::Bacnet(remote),
                None => Self::Failed(error.to_string()),
            },
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
            Self::Timeout {
                device,
                after_ms,
                busy,
            } => {
                let mut body = json!({ "error": "timeout", "after_ms": after_ms });
                if let Some(device) = device {
                    body["device"] = json!(device);
                }
                if let Some(busy) = busy {
                    body["router_busy"] = json!({
                        "router": busy.router().to_string(),
                        "network": busy.network,
                        "refused_by": busy.refused_by().map(|by| by.to_string()),
                    });
                }
                body
            }
            Self::NetworkReject { device, reject } => json!({
                "error": "network-reject",
                "device": device,
                "router": reject.router().to_string(),
                "refused_by": reject.refused_by().map(|by| by.to_string()),
                "network": reject.network,
                "reason": kebab(reject.reason),
                "reason_number": reject.reason.to_raw(),
                "reached_device": if reject.single_frame { "no" } else { "unknown" },
                "message": reject.to_string(),
            }),
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
            Self::At {
                error,
                object,
                hint,
            } => {
                let mut body = error.json();
                body["object"] = json!(object);
                body["hint"] = json!(hint);
                body
            }
        }
    }

    /// One line for the text rendering.
    pub(crate) fn summary(&self) -> String {
        match self {
            Self::Invalid { field, message } => format!("invalid {field}: {message}"),
            Self::Bacnet(remote) => format!("the device answered: {remote}"),
            Self::Timeout {
                device: Some(device),
                busy: None,
                ..
            } => format!("device {device} did not answer in time"),
            Self::Timeout {
                device: Some(device),
                busy: Some(busy),
                ..
            } => format!(
                "device {device} did not answer in time; on the way, {}",
                busy.what()
            ),
            Self::Timeout { device: None, .. } => {
                "the call ran out of time after sending requests".into()
            }
            Self::NetworkReject { device, reject } => {
                format!("a router refused the request to device {device}: {reject}")
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
            Self::At {
                error,
                object,
                hint,
            } => format!("{}, reading {object}; {hint}", error.summary()),
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
pub(crate) fn row_error_json(error: &PropertyError) -> Value {
    match error {
        PropertyError::Remote(remote) => remote_json(remote),
        PropertyError::NoValue => json!({ "result": "no-value" }),
        other => json!({ "result": "failed", "message": other.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::{AbortReason, RejectReason};

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
            row_error_json(&PropertyError::NoValue),
            json!({ "result": "no-value" })
        );
    }

    #[test]
    fn a_timeout_is_not_a_device_answer() {
        let at = DeviceAddress::local(&[7]).unwrap();
        let error = OpsError::Unreachable {
            device: at.clone(),
            source: Error::Timeout(std::time::Duration::from_millis(6000)),
        };
        assert_eq!(
            ToolError::device(7, &error),
            ToolError::Timeout {
                device: Some(7),
                after_ms: 6000,
                busy: None
            }
        );
        let abort = OpsError::Unreachable {
            device: at.clone(),
            source: Error::Remote(RemoteError::Abort {
                reason: AbortReason::TSM_TIMEOUT,
            }),
        };
        assert_eq!(
            ToolError::device(7, &abort),
            ToolError::Timeout {
                device: Some(7),
                after_ms: 0,
                busy: None
            }
        );
        let refused = OpsError::Refused {
            device: at,
            error: RemoteError::Reject {
                reason: RejectReason::UNRECOGNIZED_SERVICE,
            },
            detail: None,
        };
        assert!(matches!(
            ToolError::device(7, &refused),
            ToolError::Bacnet(RemoteError::Reject { .. })
        ));
    }

    /// A router's network reject is its own kind (#2724): sent, never a
    /// timeout and never the device's answer, with the router in its
    /// link's form and the reason by name and number.
    #[test]
    fn a_network_reject_names_router_network_and_reason() {
        use bacnet_encoding::npdu::view::{Mac, RejectMessageReason};
        use bacnet_types::data_link::DataLink;
        let reject = NetworkReject::new(
            Mac::from_array([10, 0, 0, 1, 0xBA, 0xC0]),
            Some(DataLink::Bip),
            7,
            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
        );
        let sorted = |reject| OpsError::Unreachable {
            device: DeviceAddress::new(7, &[5]).unwrap(),
            source: Error::NetworkRejected(Box::new(reject)),
        };
        let tool = ToolError::device(9, &sorted(reject));
        assert_eq!(tool, ToolError::NetworkReject { device: 9, reject });
        // After a retry, an earlier attempt may have reached the device.
        assert_eq!(
            tool.json(),
            json!({ "error": "network-reject", "device": 9, "router": "10.0.0.1:47808",
                    "refused_by": null, "network": 7, "reason": "not-directly-connected",
                    "reason_number": 1, "reached_device": "unknown",
                    "message": "Reject-Message-To-Network from router 10.0.0.1:47808: no \
                                route to network 7 (not-directly-connected, reason 1); an \
                                earlier attempt may have reached the device" })
        );
        assert!(
            tool.summary()
                .starts_with("a router refused the request to device 9")
        );
        // For the one frame sent, it never did; relayed, the far router is named.
        let once = reject
            .after_one_frame()
            .relayed_from(5, Mac::from_array([0x0A]));
        let body = ToolError::device(9, &sorted(once)).json();
        assert_eq!(body["reached_device"], json!("no"));
        assert_eq!(body["refused_by"], json!("5:0x0A"));
    }

    /// A timeout after a router said it was busy is still a timeout, and
    /// names that router and the network (#2724).
    #[test]
    fn a_busy_timeout_is_a_timeout_naming_the_router() {
        use bacnet_encoding::npdu::view::{Mac, RejectMessageReason};
        use bacnet_types::data_link::DataLink;
        let busy = NetworkReject::new(
            Mac::from_array([10, 0, 0, 1, 0xBA, 0xC0]),
            Some(DataLink::Bip),
            7,
            RejectMessageReason::ROUTER_BUSY,
        );
        let error = OpsError::Unreachable {
            device: DeviceAddress::new(7, &[5]).unwrap(),
            source: Error::RouterBusyTimeout(Box::new(busy)),
        };
        assert!(error.timed_out());
        let tool = ToolError::device(9, &error);
        assert_eq!(
            tool,
            ToolError::Timeout {
                device: Some(9),
                after_ms: 0,
                busy: Some(busy)
            }
        );
        assert_eq!(
            tool.json(),
            json!({ "error": "timeout", "after_ms": 0, "device": 9,
                    "router_busy": { "router": "10.0.0.1:47808", "network": 7,
                                     "refused_by": null } })
        );
        assert!(tool.summary().contains("busy for network 7"));
    }
}
