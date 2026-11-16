//! What an audited request names, and what its answer makes of each target
//! (#2617): the Clause 19.6.5 and Table 19-5 side of source Audit on a
//! seated client. A record's Reporter filters and delivery are the node's.

use bacnet_services::audit::{AuditLogQueryAck, AuditLogQueryRequest};
use bacnet_services::file::{AtomicReadFileRequest, AtomicWriteFileRequest};
use bacnet_services::list_manipulation::ListElementRequest;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_services::read_range::{ReadRangeAck, ReadRangeRequest};
use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
use bacnet_services::wpm::WritePropertyMultipleRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::AuditPropertyReference;
use bacnet_types::enums::{
    AbortReason, AuditOperation, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::{ErrorDetail, ReadRangeViolation};
use bacnet_types::primitives::ObjectIdentifier;

use super::*;

/// One target of an audited operation: each is reported on its own
/// (Clause 19.6.5).
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditTarget {
    /// The object operated on.
    pub object: ObjectIdentifier,
    /// The property and its array index; `None` for a file and for an
    /// Audit Log query, which name no property (Table 19-5).
    pub property: Option<AuditPropertyReference>,
    /// The priority a write was sent with, which makes it a command at
    /// that priority; `None` for a read and for a write sent without one.
    pub priority: Option<u8>,
    /// The value a write sends, as encoded; `None` for a read and for a
    /// file write (Table 19-5).
    pub value: Option<Vec<u8>>,
}

impl AuditTarget {
    fn read(object: ObjectIdentifier, property: PropertyIdentifier, index: Option<u32>) -> Self {
        Self {
            object,
            property: Some(AuditPropertyReference {
                property_identifier: property,
                property_array_index: index.map(u64::from),
            }),
            priority: None,
            value: None,
        }
    }

    fn object(object: ObjectIdentifier) -> Self {
        Self {
            object,
            property: None,
            priority: None,
            value: None,
        }
    }

    fn write(
        object: ObjectIdentifier,
        property: PropertyIdentifier,
        index: Option<u32>,
        value: &[u8],
        priority: Option<u8>,
    ) -> Self {
        Self {
            priority,
            value: Some(value.to_vec()),
            ..Self::read(object, property, index)
        }
    }

    /// Whether this target names `property` with `index`.
    fn names(&self, property: PropertyIdentifier, index: Option<u32>) -> bool {
        self.property.as_ref().is_some_and(|reference| {
            reference.property_identifier == property
                && reference.property_array_index == index.map(u64::from)
        })
    }
}

/// What an audited request came back with (#2617): its caller's result,
/// and one entry per target to report.
#[doc(hidden)]
pub struct AuditedOutcome {
    /// The request's own result, for its caller: the answer's service data,
    /// or the error.
    pub result: Result<Bytes, Error>,
    /// The invoke ID it went with, once a transaction was registered.
    pub invoke_id: Option<u8>,
    /// Whether the request reached its target: it was sent, or the device
    /// answered. A request refused before anything went, or cut off by a
    /// stop of the client or its node, reports nothing.
    pub attempted: bool,
    /// One entry per target to report, in the request's order, with its
    /// Result: `None` for a success. A ReadPropertyMultiple answer that
    /// doesn't follow the request reference by reference, as one for
    /// ALL, REQUIRED or OPTIONAL doesn't, gives one entry per value it
    /// reports.
    pub records: Vec<(AuditTarget, Option<(ErrorClass, ErrorCode)>)>,
    /// The Device a read's answer names, when its successful results name
    /// exactly one: the record's Target Device then.
    pub target_device: Option<ObjectIdentifier>,
}

/// A record's Result: `None` for a success, else the error's class and
/// code.
type Outcome = Option<(ErrorClass, ErrorCode)>;

/// Each target to report, with its Result.
type Records = Vec<(AuditTarget, Outcome)>;

/// An audited request decoded: its operation, its targets and the request.
pub(super) type DecodedRequest = (AuditOperation, Vec<AuditTarget>, Decoded);

/// A request of a service this release audits, decoded.
pub(super) enum Decoded {
    Property(ReadPropertyRequest),
    Multiple(ReadPropertyMultipleRequest),
    Range(ReadRangeRequest),
    ReadFile(AtomicReadFileRequest),
    LogQuery(AuditLogQueryRequest),
    /// WritePropertyMultiple, whose error names the first write that failed.
    WriteMultiple,
    /// Every other write: one target, answered by an acknowledgement.
    Write,
}

/// The operation and targets of a request of `service`, and the request
/// decoded: `None` for a service Table 19-5 doesn't map to READ or WRITE,
/// or that this release leaves unaudited (#2617: WriteGroup is unconfirmed,
/// and CREATE, DELETE and the others follow).
///
/// # Errors
///
/// A request of an audited service that doesn't decode: it is not sent,
/// since what it names can't be reported.
pub(super) fn decode(
    service: ConfirmedServiceChoice,
    data: &[u8],
) -> Option<Result<DecodedRequest, Error>> {
    use ConfirmedServiceChoice as S;
    let read = |targets, decoded| Ok((AuditOperation::READ, targets, decoded));
    let write = |targets, decoded| Ok((AuditOperation::WRITE, targets, decoded));
    let decoded = match service {
        S::READ_PROPERTY => ReadPropertyRequest::decode(data).and_then(|r| {
            let target = AuditTarget::read(
                r.object_identifier,
                r.property_identifier,
                r.property_array_index,
            );
            read(vec![target], Decoded::Property(r))
        }),
        S::READ_PROPERTY_MULTIPLE => ReadPropertyMultipleRequest::decode(data).and_then(|r| {
            let targets = r
                .list_of_read_access_specs
                .iter()
                .flat_map(|spec| {
                    spec.list_of_property_references.iter().map(|reference| {
                        AuditTarget::read(
                            spec.object_identifier,
                            reference.property_identifier,
                            reference.property_array_index,
                        )
                    })
                })
                .collect();
            read(targets, Decoded::Multiple(r))
        }),
        S::READ_RANGE => ReadRangeRequest::decode(data).and_then(|r| {
            let target = AuditTarget::read(
                r.object_identifier,
                r.property_identifier,
                r.property_array_index,
            );
            read(vec![target], Decoded::Range(r))
        }),
        S::ATOMIC_READ_FILE => AtomicReadFileRequest::decode(data).and_then(|r| {
            read(
                vec![AuditTarget::object(r.file_identifier)],
                Decoded::ReadFile(r),
            )
        }),
        S::AUDIT_LOG_QUERY => AuditLogQueryRequest::decode(data)
            .and_then(|r| read(vec![AuditTarget::object(r.audit_log)], Decoded::LogQuery(r))),
        S::WRITE_PROPERTY => WritePropertyRequest::decode(data).and_then(|r| {
            let target = AuditTarget::write(
                r.object_identifier,
                r.property_identifier,
                r.property_array_index,
                &r.property_value,
                r.priority,
            );
            write(vec![target], Decoded::Write)
        }),
        S::WRITE_PROPERTY_MULTIPLE => WritePropertyMultipleRequest::decode(data).and_then(|r| {
            let targets = r
                .list_of_write_access_specs
                .iter()
                .flat_map(|spec| {
                    spec.list_of_properties.iter().map(|value| {
                        AuditTarget::write(
                            spec.object_identifier,
                            value.property_identifier,
                            value.property_array_index,
                            &value.value,
                            value.priority,
                        )
                    })
                })
                .collect();
            write(targets, Decoded::WriteMultiple)
        }),
        S::ATOMIC_WRITE_FILE => AtomicWriteFileRequest::decode(data)
            .and_then(|r| write(vec![AuditTarget::object(r.file_identifier)], Decoded::Write)),
        S::ADD_LIST_ELEMENT | S::REMOVE_LIST_ELEMENT => {
            ListElementRequest::decode(data).and_then(|r| {
                let target = AuditTarget::write(
                    r.object_identifier,
                    r.property_identifier,
                    r.property_array_index,
                    &r.list_of_elements,
                    None,
                );
                write(vec![target], Decoded::Write)
            })
        }
        _ => return None,
    };
    Some(decoded.map_err(|error| {
        Error::Encoding(format!(
            "an audited request must decode, to report what it names: {error}"
        ))
    }))
}

/// Whether a request that ended with `result` reached its target, given
/// that it registered a transaction. A peer's answer, a timeout, a
/// transport failure while sending, and a request sent before the node's
/// DeviceCommunicationControl withheld its retry did; a refusal before
/// anything was sent, and a request whose transaction its client's or
/// node's stop ended, did not.
pub(super) fn attempted(result: &Result<Bytes, Error>) -> bool {
    match result {
        Ok(_) => true,
        Err(error) => matches!(
            error,
            Error::Remote(_)
                | Error::Structured { .. }
                | Error::RoutedPathTooLong { .. }
                | Error::Timeout(_)
                | Error::Transport(_)
                | Error::Segmentation(_)
                | Error::InitiationDisabled { sent: true }
        ),
    }
}

/// The Result an audit record carries for a request that failed with
/// `error` (Table 19-4): a peer's Error as it came, an Abort or Reject as
/// its COMMUNICATION code, and the client's own wait for an answer running
/// out as TIMEOUT. The client reports that wait as an Abort for a TSM
/// timeout, which a peer's Abort for that reason can't be told from; both
/// say the device didn't answer in time. A request whose retry the node's
/// DeviceCommunicationControl withheld is SERVICES, COMMUNICATION_DISABLED,
/// the pair the standard uses for a device whose communication that
/// control turned off.
pub(crate) fn audit_result(error: &Error) -> (ErrorClass, ErrorCode) {
    use bacnet_types::error::RemoteError;
    let code = match error {
        Error::Remote(RemoteError::Error { class, code })
        | Error::Structured { class, code, .. } => {
            return (*class, *code);
        }
        Error::InitiationDisabled { .. } => {
            return (ErrorClass::SERVICES, ErrorCode::COMMUNICATION_DISABLED);
        }
        Error::Timeout(_)
        | Error::Remote(RemoteError::Abort {
            reason: AbortReason::TSM_TIMEOUT,
        }) => ErrorCode::TIMEOUT,
        Error::Remote(RemoteError::Abort { reason }) => abort_code(*reason),
        Error::Remote(RemoteError::Reject { reason }) => reject_code(*reason),
        Error::RoutedPathTooLong { .. } => ErrorCode::MESSAGE_TOO_LONG,
        _ => ErrorCode::OTHER,
    };
    (ErrorClass::COMMUNICATION, code)
}

fn abort_code(reason: AbortReason) -> ErrorCode {
    match reason {
        AbortReason::OTHER => ErrorCode::ABORT_OTHER,
        AbortReason::BUFFER_OVERFLOW => ErrorCode::ABORT_BUFFER_OVERFLOW,
        AbortReason::INVALID_APDU_IN_THIS_STATE => ErrorCode::ABORT_INVALID_APDU_IN_THIS_STATE,
        AbortReason::PREEMPTED_BY_HIGHER_PRIORITY_TASK => {
            ErrorCode::ABORT_PREEMPTED_BY_HIGHER_PRIORITY_TASK
        }
        AbortReason::SEGMENTATION_NOT_SUPPORTED => ErrorCode::ABORT_SEGMENTATION_NOT_SUPPORTED,
        AbortReason::SECURITY_ERROR => ErrorCode::ABORT_SECURITY_ERROR,
        AbortReason::INSUFFICIENT_SECURITY => ErrorCode::ABORT_INSUFFICIENT_SECURITY,
        AbortReason::WINDOW_SIZE_OUT_OF_RANGE => ErrorCode::ABORT_WINDOW_SIZE_OUT_OF_RANGE,
        AbortReason::APPLICATION_EXCEEDED_REPLY_TIME => {
            ErrorCode::ABORT_APPLICATION_EXCEEDED_REPLY_TIME
        }
        AbortReason::OUT_OF_RESOURCES => ErrorCode::ABORT_OUT_OF_RESOURCES,
        AbortReason::APDU_TOO_LONG => ErrorCode::ABORT_APDU_TOO_LONG,
        reason if reason.to_raw() >= 64 => ErrorCode::ABORT_PROPRIETARY,
        _ => ErrorCode::OTHER,
    }
}

fn reject_code(reason: RejectReason) -> ErrorCode {
    match reason {
        RejectReason::OTHER => ErrorCode::REJECT_OTHER,
        RejectReason::BUFFER_OVERFLOW => ErrorCode::REJECT_BUFFER_OVERFLOW,
        RejectReason::INCONSISTENT_PARAMETERS => ErrorCode::REJECT_INCONSISTENT_PARAMETERS,
        RejectReason::INVALID_PARAMETER_DATA_TYPE => ErrorCode::REJECT_INVALID_PARAMETER_DATA_TYPE,
        RejectReason::INVALID_TAG => ErrorCode::REJECT_INVALID_TAG,
        RejectReason::MISSING_REQUIRED_PARAMETER => ErrorCode::REJECT_MISSING_REQUIRED_PARAMETER,
        RejectReason::PARAMETER_OUT_OF_RANGE => ErrorCode::REJECT_PARAMETER_OUT_OF_RANGE,
        RejectReason::TOO_MANY_ARGUMENTS => ErrorCode::REJECT_TOO_MANY_ARGUMENTS,
        RejectReason::UNDEFINED_ENUMERATION => ErrorCode::REJECT_UNDEFINED_ENUMERATION,
        RejectReason::UNRECOGNIZED_SERVICE => ErrorCode::REJECT_UNRECOGNIZED_SERVICE,
        reason if reason.to_raw() >= 64 => ErrorCode::REJECT_PROPRIETARY,
        _ => ErrorCode::OTHER,
    }
}

/// An answer that doesn't decode, or doesn't answer what was asked.
const MALFORMED: Outcome = Some((ErrorClass::COMMUNICATION, ErrorCode::OTHER));

impl Decoded {
    /// One entry per target to report for a request whose `targets` ended
    /// with `result`, and the Device a read's answer names.
    pub(super) fn records(
        &self,
        targets: Vec<AuditTarget>,
        result: &Result<Bytes, Error>,
    ) -> (Records, Option<ObjectIdentifier>) {
        let data = match result {
            Ok(data) => data,
            Err(error) => return (self.failed(targets, error), None),
        };
        let records = match self {
            Self::Property(request) => {
                let [mut target] = <[AuditTarget; 1]>::try_from(targets).expect("one target");
                match crate::acks::read_property(request, data) {
                    Ok(ack) => {
                        target.object = ack.object_identifier;
                        vec![(target, None)]
                    }
                    Err(_) => vec![(target, MALFORMED)],
                }
            }
            Self::Multiple(request) => multiple(request, targets, data),
            // A ReadRange answer that names what was read is the device's
            // answer, whichever rule of its page it breaks: the caller's
            // strict or lenient check, which the record can't see, decides
            // whether the caller keeps it.
            Self::Range(request) => answered(
                targets,
                ReadRangeAck::decode(data).is_ok_and(|ack| {
                    !ack.violations(request).iter().any(|violation| {
                        matches!(
                            violation,
                            ReadRangeViolation::ObjectMismatch
                                | ReadRangeViolation::PropertyMismatch
                                | ReadRangeViolation::ArrayIndexMismatch
                        )
                    })
                }),
            ),
            Self::ReadFile(request) => answered(
                targets,
                crate::acks::atomic_read_file(&request.access, data).is_ok(),
            ),
            Self::LogQuery(request) => answered(
                targets,
                AuditLogQueryAck::decode(data).is_ok_and(|ack| ack.audit_log == request.audit_log),
            ),
            Self::WriteMultiple | Self::Write => return (with_result(targets, None), None),
        };
        let device = one_device(&records);
        (records, device)
    }

    /// Every target of a request that failed with `error`. A
    /// WritePropertyMultiple carried out its writes in order up to the
    /// first that failed, which its error names: those before it
    /// succeeded, and the rest were not made. The error names a property,
    /// not a position, so when a request writes that property more than
    /// once it is taken to name the last write of it, and the earlier ones
    /// are recorded as made: a device stops at its first failure, so a
    /// failed earlier write would have left the later one unmade, which the
    /// error can't tell apart.
    fn failed(&self, targets: Vec<AuditTarget>, error: &Error) -> Records {
        let result = audit_result(error);
        let first_failed = match (self, error) {
            (Self::WriteMultiple, Error::Structured { detail, .. }) => match detail.as_ref() {
                ErrorDetail::FirstFailedWriteAttempt(failed) => {
                    targets.iter().rposition(|target| {
                        target.object == failed.object_identifier
                            && target.names(
                                PropertyIdentifier::from_raw(failed.property_identifier),
                                failed.property_array_index,
                            )
                    })
                }
                _ => None,
            },
            _ => None,
        };
        let first_failed = first_failed.unwrap_or(0);
        targets
            .into_iter()
            .enumerate()
            .map(|(position, target)| (target, (position >= first_failed).then_some(result)))
            .collect()
    }
}

fn with_result(targets: Vec<AuditTarget>, result: Outcome) -> Records {
    targets.into_iter().map(|target| (target, result)).collect()
}

/// Every target, a success when the device's answer decodes as an answer
/// to the request (`answered`).
fn answered(targets: Vec<AuditTarget>, answered: bool) -> Records {
    with_result(targets, if answered { None } else { MALFORMED })
}

/// A ReadPropertyMultiple answer's entries. One that follows the request
/// reference by reference keeps the requested targets, with the object the
/// device reports (a wildcard read names the device's own) and each
/// result's error. Any other answer that decodes, such as one for ALL,
/// REQUIRED or OPTIONAL, gives one entry per value it reports, as it
/// reports it.
fn multiple(
    request: &ReadPropertyMultipleRequest,
    targets: Vec<AuditTarget>,
    data: &[u8],
) -> Records {
    let Ok(ack) = ReadPropertyMultipleACK::decode(data) else {
        return with_result(targets, MALFORMED);
    };
    let follows = request.list_of_read_access_specs.len() == ack.list_of_read_access_results.len()
        && request
            .list_of_read_access_specs
            .iter()
            .zip(&ack.list_of_read_access_results)
            .all(|(spec, result)| {
                let requested = spec.object_identifier;
                let reported = result.object_identifier;
                let object = reported == requested
                    || (requested.instance_number() == ObjectIdentifier::WILDCARD_INSTANCE
                        && reported.object_type() == requested.object_type()
                        && reported.instance_number() != ObjectIdentifier::WILDCARD_INSTANCE);
                object
                    && spec.list_of_property_references.len() == result.list_of_results.len()
                    && spec
                        .list_of_property_references
                        .iter()
                        .zip(&result.list_of_results)
                        .all(|(asked, got)| {
                            asked.property_identifier == got.property_identifier
                                && (asked.property_array_index == got.property_array_index
                                    || (got.property_array_index.is_none() && got.error.is_some()))
                        })
            });
    let reported = ack.list_of_read_access_results.iter().flat_map(|result| {
        result
            .list_of_results
            .iter()
            .map(move |element| (result.object_identifier, element))
    });
    if follows {
        targets
            .into_iter()
            .zip(reported)
            .map(|(mut target, (object, element))| {
                target.object = object;
                (target, element.error)
            })
            .collect()
    } else {
        reported
            .map(|(object, element)| {
                let target = AuditTarget::read(
                    object,
                    element.property_identifier,
                    element.property_array_index,
                );
                (target, element.error)
            })
            .collect()
    }
}

/// The one Device a read's successful entries name, if they name exactly
/// one and no wildcard.
fn one_device(records: &[(AuditTarget, Outcome)]) -> Option<ObjectIdentifier> {
    let mut devices = records
        .iter()
        .filter(|(target, result)| {
            result.is_none()
                && target.object.object_type() == ObjectType::DEVICE
                && target.object.instance_number() != ObjectIdentifier::WILDCARD_INSTANCE
        })
        .map(|(target, _)| target.object);
    let first = devices.next()?;
    devices.all(|object| object == first).then_some(first)
}
