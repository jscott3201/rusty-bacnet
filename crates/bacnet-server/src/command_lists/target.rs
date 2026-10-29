//! Making one write of a run, in this device or another, and saying how it
//! failed (#1264).
//!
//! A Channel reports the first failure of a distribution in its Reliability
//! (Clause 12.53.9), which names three kinds: the member's configuration, a
//! communication failure, and any other error. The runner sorts each failed
//! write into one of them:
//!
//! - configuration: the target's answer says the reference names nothing it
//!   has (UNKNOWN_OBJECT, UNKNOWN_PROPERTY, an array index or array access
//!   that doesn't fit) or that the property takes no value of this datatype
//!   (INVALID_DATA_TYPE, DATATYPE_NOT_SUPPORTED, or a Reject for an invalid
//!   parameter datatype). The Schedule reads both kinds of refusal the same
//!   way, as a configuration fault (#1433).
//!   A value the Channel itself can't coerce is a configuration failure too
//!   (in `channel`).
//! - communication: a member in another device that couldn't be reached: no
//!   fresh binding and no I-Am to the Who-Is sent for one (or none sent, as
//!   one drew nothing within the last minute), initiation disabled by
//!   DeviceCommunicationControl, or no answer to the first attempt or any
//!   retry. An attempt the transport failed to send waits like a silent one,
//!   so a send failure on every attempt ends here too.
//! - process: every other refusal (an Error with another code, a Reject for
//!   another reason, an Abort) and every write this server couldn't start:
//!   a value it can't encode, a request longer than one APDU, no free invoke
//!   ID, a server stopping, or no network at all.
//!
//! A Command keeps only whether each write succeeded, so the kind matters to
//! Channels alone.

use std::fmt;

use bacnet_objects::command::{CommandRun, RunPlan, WriteFailure};
use bacnet_types::constructed::BACnetActionCommand;
use bacnet_types::enums::{ErrorCode, RejectReason};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::ObjectIdentifier;
use tracing::debug;

use super::RunHost;
use crate::server::RemoteRequestError;

/// A write that failed.
#[derive(Debug)]
pub(super) struct Failed {
    /// How it failed.
    pub(super) failure: WriteFailure,
    /// The target's answer, when it refused the write.
    pub(super) answer: Option<Error>,
    /// Whether the target's device answered none of the attempts, or none of
    /// the Who-Is sent to find it.
    pub(super) unanswered: bool,
}

/// Make `command` on behalf of `run`: in this device, or in `device` when it
/// names another one.
pub(super) async fn write<H: RunHost>(
    host: &H,
    run: &CommandRun,
    device: Option<ObjectIdentifier>,
    command: &BACnetActionCommand,
) -> Result<(), Failed> {
    let failed = match device {
        None => {
            let Err(error) = host.write(run, command).await else {
                return Ok(());
            };
            log_failure(run, None, command, &error);
            Failed {
                failure: refused(&error),
                answer: Some(error),
                unanswered: false,
            }
        }
        Some(device) => {
            let Err(error) = host.write_remote(device, command).await else {
                return Ok(());
            };
            log_failure(run, Some(device), command, &error);
            match error {
                RemoteRequestError::Refused(refusal) => {
                    let answer = Error::from(refusal);
                    Failed {
                        failure: refused(&answer),
                        answer: Some(answer),
                        unanswered: false,
                    }
                }
                other => Failed {
                    failure: unmade(other),
                    answer: None,
                    unanswered: silent(other),
                },
            }
        }
    };
    Err(failed)
}

/// Log a failed write under the field a Command's (`command`) or a
/// Channel's (`channel`) run is known by.
fn log_failure(
    run: &CommandRun,
    device: Option<ObjectIdentifier>,
    command: &BACnetActionCommand,
    error: &dyn fmt::Display,
) {
    let target = command.object_identifier;
    let property = command.property_identifier;
    match (&run.plan, device) {
        (RunPlan::Actions(_), None) => debug!(
            command = %run.source, %target, ?property, %error, "Command write failed"
        ),
        (RunPlan::Actions(_), Some(device)) => debug!(
            command = %run.source, %device, %target, ?property, %error,
            "Command write to another device failed"
        ),
        (RunPlan::Channel(_), None) => debug!(
            channel = %run.source, %target, ?property, %error, "Channel member write failed"
        ),
        (RunPlan::Channel(_), Some(device)) => debug!(
            channel = %run.source, %device, %target, ?property, %error,
            "Channel member write to another device failed"
        ),
    }
}

/// The error code `error` carries, if it's an Error answer.
fn code(error: &Error) -> Option<ErrorCode> {
    match error {
        Error::Remote(RemoteError::Error { code, .. }) | Error::Structured { code, .. } => {
            Some(*code)
        }
        _ => None,
    }
}

/// Whether the target refused the write for the value's datatype: an Error
/// of INVALID_DATA_TYPE or a Reject for an invalid parameter datatype, the
/// two answers Clause 12.53.7 names for a NULL.
pub(super) fn refuses_datatype(error: &Error) -> bool {
    let invalid = ErrorCode::INVALID_DATA_TYPE;
    code(error) == Some(invalid)
        || matches!(error, Error::Remote(RemoteError::Reject { reason })
            if *reason == RejectReason::INVALID_PARAMETER_DATA_TYPE)
}

/// How a write the target refused with `error` failed.
fn refused(error: &Error) -> WriteFailure {
    const CONFIGURATION: [ErrorCode; 5] = [
        ErrorCode::UNKNOWN_OBJECT,
        ErrorCode::UNKNOWN_PROPERTY,
        ErrorCode::INVALID_ARRAY_INDEX,
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ErrorCode::DATATYPE_NOT_SUPPORTED,
    ];
    let configuration =
        refuses_datatype(error) || code(error).is_some_and(|code| CONFIGURATION.contains(&code));
    if configuration {
        WriteFailure::Configuration
    } else {
        WriteFailure::Process
    }
}

/// Whether a write in another device that ended with `error` found the
/// device silent: no answer to any attempt, or no I-Am to the Who-Is sent
/// to find it.
fn silent(error: RemoteRequestError) -> bool {
    matches!(
        error,
        RemoteRequestError::Unanswered | RemoteRequestError::Undiscovered
    )
}

/// How a write in another device that got no answer, or was never sent,
/// failed.
fn unmade(error: RemoteRequestError) -> WriteFailure {
    match error {
        RemoteRequestError::Disabled
        | RemoteRequestError::Unbound
        | RemoteRequestError::Undiscovered
        | RemoteRequestError::Unanswered => WriteFailure::Communication,
        RemoteRequestError::Refused(refusal) => refused(&Error::from(refusal)),
        RemoteRequestError::Unencodable
        | RemoteRequestError::TooLong
        | RemoteRequestError::NoInvokeId
        | RemoteRequestError::Stopping
        | RemoteRequestError::NoNetwork
        // Only a read's answer is ever malformed.
        | RemoteRequestError::Malformed => WriteFailure::Process,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::{AbortReason, ErrorClass};
    use bacnet_types::error::RemoteError;

    fn error(code: ErrorCode) -> Error {
        RemoteError::Error {
            class: ErrorClass::PROPERTY,
            code,
        }
        .into()
    }

    #[test]
    fn failures_sort_into_configuration_process_and_communication() {
        for code in [
            ErrorCode::UNKNOWN_OBJECT,
            ErrorCode::UNKNOWN_PROPERTY,
            ErrorCode::INVALID_ARRAY_INDEX,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
            ErrorCode::INVALID_DATA_TYPE,
            ErrorCode::DATATYPE_NOT_SUPPORTED,
        ] {
            assert_eq!(
                refused(&error(code)),
                WriteFailure::Configuration,
                "{code:?}"
            );
        }
        let datatype = RemoteError::Reject {
            reason: RejectReason::INVALID_PARAMETER_DATA_TYPE,
        };
        assert_eq!(refused(&datatype.into()), WriteFailure::Configuration);
        for answer in [
            error(ErrorCode::WRITE_ACCESS_DENIED),
            error(ErrorCode::VALUE_OUT_OF_RANGE),
            RemoteError::Reject {
                reason: RejectReason::OTHER,
            }
            .into(),
            RemoteError::Abort {
                reason: AbortReason::OTHER,
            }
            .into(),
        ] {
            assert_eq!(refused(&answer), WriteFailure::Process, "{answer:?}");
        }

        for silence in [
            RemoteRequestError::Disabled,
            RemoteRequestError::Unbound,
            RemoteRequestError::Undiscovered,
            RemoteRequestError::Unanswered,
        ] {
            assert_eq!(unmade(silence), WriteFailure::Communication);
        }
        for local in [
            RemoteRequestError::Unencodable,
            RemoteRequestError::TooLong,
            RemoteRequestError::NoInvokeId,
            RemoteRequestError::Stopping,
            RemoteRequestError::NoNetwork,
        ] {
            assert_eq!(unmade(local), WriteFailure::Process);
        }
    }

    #[test]
    fn only_silence_to_the_write_or_its_who_is_marks_the_device_silent() {
        assert!(silent(RemoteRequestError::Unanswered));
        assert!(silent(RemoteRequestError::Undiscovered));
        // Nothing was asked of the device for these.
        for unasked in [
            RemoteRequestError::Disabled,
            RemoteRequestError::Unbound,
            RemoteRequestError::NoInvokeId,
            RemoteRequestError::Stopping,
        ] {
            assert!(!silent(unasked), "{unasked:?}");
        }
        let busy = RemoteError::Error {
            class: ErrorClass::OBJECT,
            code: ErrorCode::BUSY,
        };
        assert!(!silent(RemoteRequestError::Refused(busy)));
    }

    #[test]
    fn only_an_invalid_datatype_error_or_reject_refuses_the_datatype() {
        assert!(refuses_datatype(&error(ErrorCode::INVALID_DATA_TYPE)));
        assert!(refuses_datatype(
            &RemoteError::Reject {
                reason: RejectReason::INVALID_PARAMETER_DATA_TYPE
            }
            .into()
        ));
        assert!(!refuses_datatype(&error(ErrorCode::DATATYPE_NOT_SUPPORTED)));
        assert!(!refuses_datatype(
            &RemoteError::Reject {
                reason: RejectReason::OTHER
            }
            .into()
        ));
    }
}
