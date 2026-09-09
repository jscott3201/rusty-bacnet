use super::*;
use bacnet_objects::file::FileWriteStart;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::FileAccessMethod as ObjectFileAccessMethod;

/// Refuse a request whose access method does not match the File object's
/// declared File_Access_Method, or whose declared method cannot be read
/// back as the Clause 21 `BACnetFileAccessMethod` production.
///
/// Clauses 14.1 and 14.2 require SERVICES / INVALID_FILE_ACCESS_METHOD for
/// an "Incorrect File access method"; Clause 18 defines the code as the
/// error generated when AtomicReadFile or AtomicWriteFile specifies a File
/// Access Method that is not valid for the specified file. Reading fails
/// closed: a missing, undecodable, or out-of-production property value is
/// treated as a mismatch rather than defaulting to stream or record access.
fn invalid_file_access_method() -> Error {
    Error::Protocol {
        class: ErrorClass::SERVICES.to_raw() as u32,
        code: ErrorCode::INVALID_FILE_ACCESS_METHOD.to_raw() as u32,
    }
}

/// Refuse a request whose 'File Identifier' names a non-File object type.
///
/// The Clause 14.1.4.1 and 14.2.4.1 error tables pair "A non-File Object
/// Identifier was provided" with SERVICES / INCONSISTENT_OBJECT_TYPE, and
/// Clause 18 gives an AtomicReadFile request for a non-File object as the
/// code's example. The type is a property of the parameter alone, so it is
/// classified before the object lookup; the standard does not sequence this
/// check against "The File object does not exist", so an absent non-File
/// identifier gets this error rather than OBJECT / UNKNOWN_OBJECT.
fn inconsistent_object_type() -> Error {
    Error::Protocol {
        class: ErrorClass::SERVICES.to_raw() as u32,
        code: ErrorCode::INCONSISTENT_OBJECT_TYPE.to_raw() as u32,
    }
}

fn validate_file_access_method(
    object: &dyn BACnetObject,
    expected: ObjectFileAccessMethod,
) -> Result<(), Error> {
    let actual = match object.read_property(PropertyIdentifier::FILE_ACCESS_METHOD, None) {
        Ok(PropertyValue::Enumerated(raw)) => raw,
        _ => return Err(invalid_file_access_method()),
    };
    if actual != expected.to_raw() {
        return Err(invalid_file_access_method());
    }
    Ok(())
}

/// Refuse a request whose start position or record is not valid for the
/// file.
///
/// The Clause 14.1 Service Procedure returns an error when 'File Start
/// Position' or 'File Start Record' "is either less than 0 or exceeds the
/// actual file size"; Clause 18 pairs both parameters with
/// INVALID_FILE_START_POSITION. Clause 14.2 defines only -1 as a negative
/// write start (append), so any other negative value gets the same error.
fn invalid_file_start_position() -> Error {
    Error::Protocol {
        class: ErrorClass::SERVICES.to_raw() as u32,
        code: ErrorCode::INVALID_FILE_START_POSITION.to_raw() as u32,
    }
}

/// Refuse access to a File object the handler cannot safely use.
///
/// Clause 18 scopes FILE_ACCESS_DENIED to a file "that is currently locked
/// or otherwise not accessible", and Clause 14.2.4.1 pairs it with "Write
/// to a read-only File". Both handlers report a File-typed object whose
/// `file_storage_internal` hook returns `None` this way rather than reading
/// it as empty, and the write handler also reports a `Read_Only` that is
/// TRUE, unreadable, or not a BOOLEAN: like the access-method gate, it
/// fails closed instead of assuming the file is writable.
fn file_access_denied() -> Error {
    Error::Protocol {
        class: ErrorClass::SERVICES.to_raw() as u32,
        code: ErrorCode::FILE_ACCESS_DENIED.to_raw() as u32,
    }
}

fn unknown_object() -> Error {
    Error::Protocol {
        class: ErrorClass::OBJECT.to_raw() as u32,
        code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
    }
}

/// Resolved write positions travel in the ACK as an INTEGER; the
/// `FileStorage` contract keeps them within that range, so a value that
/// does not fit is an implementation fault, not a client error.
fn ack_position(actual: u64) -> Result<i32, Error> {
    i32::try_from(actual).map_err(|_| Error::Protocol {
        class: ErrorClass::DEVICE.to_raw() as u32,
        code: ErrorCode::INTERNAL_ERROR.to_raw() as u32,
    })
}

/// Handle an AtomicReadFile request.
///
/// The dispatcher holds the object database's read guard for the whole
/// handler, so no AtomicWriteFile can interleave the read (Clause 14
/// atomicity); concurrent AtomicReadFile handlers are admitted and see the
/// same unmodified contents.
pub fn handle_atomic_read_file(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    read_file(db, service_data, buf, None).map_err(|failure| match failure {
        AtomicReadFileFailure::Service(error) => error,
        AtomicReadFileFailure::Budget(_) => unreachable!("unconfigured read has no budget"),
    })
}

/// Keep local budget refusals distinct from pre-existing service/storage errors.
#[derive(Debug)]
pub(crate) enum AtomicReadFileFailure {
    Service(Error),
    Budget(bacnet_types::enums::AbortReason),
}

impl From<Error> for AtomicReadFileFailure {
    fn from(error: Error) -> Self {
        Self::Service(error)
    }
}

/// Apply local request-count and complete service-ACK budgets, preserving the
/// legacy handler's validation and storage-error precedence for admitted reads.
pub(crate) fn handle_atomic_read_file_budgeted(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
    budget: crate::server::AtomicReadFileBudget,
) -> Result<(), AtomicReadFileFailure> {
    read_file(db, service_data, buf, Some(budget))
}

fn read_file(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
    budget: Option<crate::server::AtomicReadFileBudget>,
) -> Result<(), AtomicReadFileFailure> {
    use bacnet_services::common::MAX_DECODED_ITEMS;
    use bacnet_services::file::{
        AtomicReadFileAck, AtomicReadFileRequest, FileAccessMethod, FileReadAckMethod,
    };

    let request = AtomicReadFileRequest::decode(service_data)?;

    if request.file_identifier.object_type() != ObjectType::FILE {
        return Err(inconsistent_object_type().into());
    }

    let object = db
        .get(&request.file_identifier)
        .ok_or_else(unknown_object)?;

    // Clause 14.1 Service Procedure, first step: a File object "currently
    // inaccessible for another reason" is refused before its properties are
    // consulted; an object without storage is that case.
    if object.file_storage_internal().is_none() {
        return Err(file_access_denied().into());
    }

    // Clause 14.1: refuse a mismatched access method before any file read
    // or ACK encoding. The request CHOICE maps semantically — Stream means
    // STREAM_ACCESS, Record means RECORD_ACCESS — never by CHOICE tag
    // number (stream is CHOICE [0] but enumeration value 1).
    let expected = match &request.access {
        FileAccessMethod::Stream { .. } => ObjectFileAccessMethod::STREAM_ACCESS,
        FileAccessMethod::Record { .. } => ObjectFileAccessMethod::RECORD_ACCESS,
    };
    validate_file_access_method(object, expected)?;

    let storage = object
        .file_storage_internal()
        .ok_or_else(file_access_denied)?;

    match request.access {
        FileAccessMethod::Stream {
            file_start_position,
            requested_octet_count,
        } => {
            let start =
                u64::try_from(file_start_position).map_err(|_| invalid_file_start_position())?;
            if budget.is_some_and(|b| {
                u64::from(requested_octet_count) > b.max_requested_stream_octets as u64
            }) {
                return Err(AtomicReadFileFailure::Budget(
                    bacnet_types::enums::AbortReason::OUT_OF_RESOURCES,
                ));
            }
            let read = storage.read_stream(start, u64::from(requested_octet_count))?;
            let ack = AtomicReadFileAck {
                end_of_file: read.end_of_file,
                access: FileReadAckMethod::Stream {
                    file_start_position,
                    file_data: read.data,
                },
            };
            encode_read_ack(&ack, buf, budget)?;
            Ok(())
        }
        FileAccessMethod::Record {
            file_start_record,
            requested_record_count,
        } => {
            let start =
                u64::try_from(file_start_record).map_err(|_| invalid_file_start_position())?;
            if budget
                .is_some_and(|b| u64::from(requested_record_count) > b.max_requested_records as u64)
            {
                return Err(AtomicReadFileFailure::Budget(
                    bacnet_types::enums::AbortReason::OUT_OF_RESOURCES,
                ));
            }
            // The workspace's AtomicReadFile-ACK decoder accepts at most
            // MAX_DECODED_ITEMS records in one SEQUENCE OF, so one ACK never
            // carries more. A client sees 'Returned Record Count' below its
            // request and, while records remain, End Of File FALSE, so it
            // continues from start + returned. Clause 14.1's Service
            // Procedure short-reads only when fewer records remain; this
            // window is a second, local reason.
            let count = u64::from(requested_record_count).min(MAX_DECODED_ITEMS as u64);
            let read = storage.read_records(start, count)?;
            let ack = AtomicReadFileAck {
                end_of_file: read.end_of_file,
                access: FileReadAckMethod::Record {
                    file_start_record,
                    returned_record_count: read.records.len() as u32,
                    file_record_data: read.records,
                },
            };
            encode_read_ack(&ack, buf, budget)?;
            Ok(())
        }
    }
}

fn encode_read_ack(
    ack: &bacnet_services::file::AtomicReadFileAck,
    buf: &mut BytesMut,
    budget: Option<crate::server::AtomicReadFileBudget>,
) -> Result<(), AtomicReadFileFailure> {
    if budget.is_some_and(|b| ack.encoded_len_bounded(b.max_service_ack_bytes).is_none()) {
        return Err(AtomicReadFileFailure::Budget(
            bacnet_types::enums::AbortReason::BUFFER_OVERFLOW,
        ));
    }
    // All budget failures precede encoding: no payload copy or caller mutation
    // is needed to size the owned storage result.
    ack.encode(buf);
    Ok(())
}

/// Handle an AtomicWriteFile request.
///
/// The dispatcher holds the object database's write guard for the whole
/// handler, so the gates, the write, and the ACK are one atomic operation
/// per Clause 14. Pre-write refusals do not invoke a storage write or modify
/// caller output (metadata/hook side effects are excluded); a storage that breaks the
/// [`FileStorage`](bacnet_objects::file::FileStorage) position contract
/// can leave the object mutated and still draw DEVICE / INTERNAL_ERROR
/// from the ACK conversion.
pub fn handle_atomic_write_file(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    write_file(db, service_data, buf, None).map_err(|failure| match failure {
        AtomicWriteFileFailure::Service(error) => error,
        AtomicWriteFileFailure::Budget => unreachable!("unconfigured write has no budget"),
    })
}

/// A local admission refusal is not an opaque storage `Error::Abort`.
#[derive(Debug)]
pub(crate) enum AtomicWriteFileFailure {
    Service(Error),
    Budget,
}

impl From<Error> for AtomicWriteFileFailure {
    fn from(error: Error) -> Self {
        Self::Service(error)
    }
}

pub(crate) fn handle_atomic_write_file_budgeted(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
    budget: crate::server::AtomicWriteFileBudget,
) -> Result<(), AtomicWriteFileFailure> {
    write_file(db, service_data, buf, Some(budget))
}

fn write_file(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
    budget: Option<crate::server::AtomicWriteFileBudget>,
) -> Result<(), AtomicWriteFileFailure> {
    use bacnet_services::file::{
        AtomicWriteFileAck, AtomicWriteFileRequest, FileWriteAccessMethod, FileWriteAckMethod,
    };

    let request = AtomicWriteFileRequest::decode(service_data)?;

    // Wire decoding enforces record cardinality. Keep this cross-check so a
    // future typed or internal request path cannot bypass the same pre-mutation
    // refusal.
    if let FileWriteAccessMethod::Record {
        record_count,
        file_record_data,
        ..
    } = &request.access
    {
        if *record_count as usize != file_record_data.len() {
            return Err(Error::Reject {
                reason: RejectReason::MISSING_REQUIRED_PARAMETER.to_raw(),
            }
            .into());
        }
    }

    if request.file_identifier.object_type() != ObjectType::FILE {
        return Err(inconsistent_object_type().into());
    }

    let object = db
        .get_mut(&request.file_identifier)
        .ok_or_else(unknown_object)?;

    // Clause 14.2 Service Procedure, first step: a File object "currently
    // inaccessible for another reason" is refused before its properties are
    // consulted; an object without storage is that case.
    if object.file_storage_internal().is_none() {
        return Err(file_access_denied().into());
    }

    // Clause 14.2.4.1 "Write to a read-only File". Reading the property
    // fails closed, as the access-method gate below does: a missing,
    // undecodable, or non-BOOLEAN Read_Only is treated as read-only rather
    // than as permission to write.
    match object.read_property(PropertyIdentifier::READ_ONLY, None) {
        Ok(PropertyValue::Boolean(false)) => {}
        _ => return Err(file_access_denied().into()),
    }

    // Clause 14.2: refuse a mismatched access method before any mutation or
    // ACK encoding, preserving the existing READ_ONLY precedence. The
    // request CHOICE maps semantically — Stream means STREAM_ACCESS, Record
    // means RECORD_ACCESS — never by CHOICE tag number.
    let expected = match &request.access {
        FileWriteAccessMethod::Stream { .. } => ObjectFileAccessMethod::STREAM_ACCESS,
        FileWriteAccessMethod::Record { .. } => ObjectFileAccessMethod::RECORD_ACCESS,
    };
    validate_file_access_method(&**object, expected)?;

    let storage = object
        .file_storage_internal_mut()
        .ok_or_else(file_access_denied)?;

    match request.access {
        FileWriteAccessMethod::Stream {
            file_start_position,
            file_data,
        } => {
            let start = write_start(file_start_position)?;
            if budget.is_some_and(|b| file_data.len() > b.max_stream_payload_octets) {
                return Err(AtomicWriteFileFailure::Budget);
            }
            let actual = storage.write_stream(start, &file_data)?;
            let ack = AtomicWriteFileAck {
                access: FileWriteAckMethod::Stream {
                    file_start_position: ack_position(actual)?,
                },
            };
            ack.encode(buf);
            Ok(())
        }
        FileWriteAccessMethod::Record {
            file_start_record,
            file_record_data,
            ..
        } => {
            let start = write_start(file_start_record)?;
            if budget.is_some_and(|b| {
                file_record_data.len() > b.max_records
                    || !record_payload_fits(
                        file_record_data.iter().map(Vec::len),
                        b.max_record_payload_bytes,
                    )
            }) {
                return Err(AtomicWriteFileFailure::Budget);
            }
            let actual = storage.write_records(start, &file_record_data)?;
            let ack = AtomicWriteFileAck {
                access: FileWriteAckMethod::Record {
                    file_start_record: ack_position(actual)?,
                },
            };
            ack.encode(buf);
            Ok(())
        }
    }
}

/// Map a request's 'File Start Position' / 'File Start Record' to a write
/// start: -1 is the Clauses 14.2.2.2 / 14.2.2.3 append sentinel, other
/// negatives are invalid, and the ACK later carries the position actually
/// written.
fn write_start(requested: i32) -> Result<FileWriteStart, Error> {
    match requested {
        -1 => Ok(FileWriteStart::Append),
        position => u64::try_from(position)
            .map(FileWriteStart::At)
            .map_err(|_| invalid_file_start_position()),
    }
}

/// Checked sum, stopping at the first excess without examining later lengths.
fn record_payload_fits(lengths: impl IntoIterator<Item = usize>, cap: usize) -> bool {
    let mut total = 0usize;
    for len in lengths {
        let Some(next) = total.checked_add(len).filter(|&next| next <= cap) else {
            return false;
        };
        total = next;
    }
    true
}

#[cfg(test)]
mod write_budget_arithmetic {
    use super::record_payload_fits;

    #[test]
    fn atomic_write_file_checked_sum_and_early_stop() {
        assert!(record_payload_fits([0, usize::MAX], usize::MAX));
        assert!(!record_payload_fits([usize::MAX, 1], usize::MAX));
        assert!(record_payload_fits([2, 3, 0], 5));
        assert!(!record_payload_fits([2, 3], 4));
        assert!(!record_payload_fits(
            std::iter::once(6).chain(std::iter::from_fn(|| panic!("past excess"))),
            5
        ));
    }
}
