use super::*;
use bacnet_device_core::write::{
    self, Applied, WriteCommitObserver, WriteTarget, check_and_prepare_name_write, commit_attempt,
    gate_and_decode_write,
};

/// Rich WPM result retained inside the server boundary.
pub(crate) enum WritePropertyMultipleOutcome {
    Success {
        committed_oids: Vec<ObjectIdentifier>,
    },
    Error {
        error: Error,
        first_failed_write_attempt: BACnetObjectPropertyReference,
        committed_oids: Vec<ObjectIdentifier>,
    },
    Reject {
        reason: RejectReason,
    },
}

/// Handle WPM while preserving the historical direct handler projection.
///
/// The complete successful prefix remains committed if a later attempt fails.
/// A Command or Channel object's Present_Value write in that prefix runs
/// nothing: as in [`handle_write_property`], its list or distribution ends at
/// once as unsuccessful.
pub fn handle_write_property_multiple(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<Vec<ObjectIdentifier>, Error> {
    let mut snapshots = crate::life_safety_cov::LifeSafetyCovSnapshots::default();
    match handle_write_property_multiple_detailed(db, service_data, &mut snapshots) {
        WritePropertyMultipleOutcome::Success { committed_oids } => {
            write::end_unmade(db, &committed_oids);
            Ok(committed_oids)
        }
        WritePropertyMultipleOutcome::Error {
            error,
            committed_oids,
            ..
        } => {
            write::end_unmade(db, &committed_oids);
            Err(error)
        }
        WritePropertyMultipleOutcome::Reject { reason } => Err(Error::reject(reason)),
    }
}

/// Execute WPM incrementally in wire order for server dispatch.
pub(crate) fn handle_write_property_multiple_detailed(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    snapshots: &mut crate::life_safety_cov::LifeSafetyCovSnapshots,
) -> WritePropertyMultipleOutcome {
    handle_write_property_multiple_authorized(db, service_data, snapshots, None)
}

type WritePropertyMultipleGate<'a> =
    &'a dyn Fn(&bacnet_services::wpm::WritePropertyAttempt) -> Result<(), Error>;

/// Optional per-element gate; `None` preserves the existing incremental path.
pub(crate) fn handle_write_property_multiple_authorized(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    snapshots: &mut crate::life_safety_cov::LifeSafetyCovSnapshots,
    authorize: Option<WritePropertyMultipleGate<'_>>,
) -> WritePropertyMultipleOutcome {
    handle_write_property_multiple_observed(
        db,
        service_data,
        snapshots,
        authorize,
        None,
        None,
        None,
    )
}

pub(crate) fn handle_write_property_multiple_observed(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    snapshots: &mut crate::life_safety_cov::LifeSafetyCovSnapshots,
    authorize: Option<WritePropertyMultipleGate<'_>>,
    mut observer: Option<&mut dyn WriteCommitObserver>,
    source: Option<&bacnet_objects::device::AuditWriteSource>,
    command_origin: Option<&bacnet_objects::command_source::CommandOrigin>,
) -> WritePropertyMultipleOutcome {
    let mut cursor = WritePropertyMultipleCursor::new(service_data);
    let mut committed_oids = Vec::new();
    // Whether any attempt has succeeded. A NULL the property left as it was
    // commits no object but is a successful write all the same, so a later
    // syntax error is an Error with INVALID_TAG, not a Reject (Clause
    // 15.10.2).
    let mut wrote = false;

    loop {
        let event = match cursor.next_event() {
            Ok(Some(event)) => event,
            Ok(None) => {
                return WritePropertyMultipleOutcome::Success { committed_oids };
            }
            Err(cursor_error) => {
                use bacnet_services::wpm::WritePropertyMultipleFailureKind;
                let code = match cursor_error.kind {
                    WritePropertyMultipleFailureKind::PriorityOutOfRange => {
                        ErrorCode::PARAMETER_OUT_OF_RANGE
                    }
                    WritePropertyMultipleFailureKind::Syntax(reason) if !wrote => {
                        return WritePropertyMultipleOutcome::Reject { reason };
                    }
                    WritePropertyMultipleFailureKind::Syntax(_) => ErrorCode::INVALID_TAG,
                };
                return WritePropertyMultipleOutcome::Error {
                    error: protocol_error(ErrorClass::SERVICES, code),
                    first_failed_write_attempt: cursor_error
                        .first_failed_write_attempt
                        .unwrap_or_else(wpm_undecodable_coordinate),
                    committed_oids,
                };
            }
        };
        let WritePropertyMultipleEvent::WriteAttempt(attempt) = event else {
            continue;
        };
        let reference = attempt.reference.clone();
        let oid = reference.object_identifier;
        let property = PropertyIdentifier::from_raw(reference.property_identifier);

        let Some(object) = db.get(&oid) else {
            return semantic_failure(
                protocol_error(ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
                reference,
                committed_oids,
            );
        };
        let value = match gate_and_decode_write(
            object,
            property,
            reference.property_array_index,
            &attempt.value,
        ) {
            Ok(value) => write::localize(db, oid, property, value),
            Err(error) => return semantic_failure(error, reference, committed_oids),
        };
        if property == PropertyIdentifier::OBJECT_NAME
            && let Err(error) = check_and_prepare_name_write(db, &oid, &value)
        {
            return semantic_failure(error, reference, committed_oids);
        }

        if let Some(authorize) = authorize
            && let Err(error) = authorize(&attempt)
        {
            return semantic_failure(error, reference, committed_oids);
        }
        if property == PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT
            && let Some(mut device) = db
                .get_mut(&oid)
                .and_then(|object| object.device_authority_internal())
            && device.object_identifier() == oid
        {
            if let Err(error) = device.write_audit_recipient(
                reference.property_array_index,
                value,
                attempt.priority,
                source,
            ) {
                return semantic_failure(error, reference, committed_oids);
            }
            if let Some(observer) = observer.as_deref_mut() {
                observer.applied(db, oid);
            }
            wrote = true;
            if !committed_oids.contains(&oid) {
                committed_oids.push(oid);
            }
            continue;
        }
        snapshots.capture_before_write(db, oid);
        let target = WriteTarget {
            oid,
            property,
            array_index: reference.property_array_index,
            priority: attempt.priority,
            value: &attempt.value,
        };
        match commit_attempt(
            db,
            observer.as_deref_mut(),
            target,
            value,
            source,
            command_origin,
        ) {
            // A NULL the property left as it was commits nothing.
            Ok(Applied::Unchanged) => wrote = true,
            Ok(Applied::Written) => {
                wrote = true;
                if !committed_oids.contains(&oid) {
                    committed_oids.push(oid);
                }
            }
            Err(error) => return semantic_failure(error, reference, committed_oids),
        }
    }
}

fn semantic_failure(
    error: Error,
    first_failed_write_attempt: BACnetObjectPropertyReference,
    committed_oids: Vec<ObjectIdentifier>,
) -> WritePropertyMultipleOutcome {
    WritePropertyMultipleOutcome::Error {
        error,
        first_failed_write_attempt,
        committed_oids,
    }
}

fn protocol_error(class: ErrorClass, code: ErrorCode) -> Error {
    Error::protocol(class, code)
}

fn wpm_undecodable_coordinate() -> BACnetObjectPropertyReference {
    BACnetObjectPropertyReference {
        // Clause 15.10 fixes instance 4194303. DEVICE / ALL / no index is the
        // repository's local policy for the remaining undecodable coordinates.
        object_identifier: ObjectIdentifier::new(
            ObjectType::DEVICE,
            ObjectIdentifier::MAX_INSTANCE,
        )
        .expect("the wildcard instance is valid service vocabulary"),
        property_identifier: PropertyIdentifier::ALL.to_raw(),
        property_array_index: None,
    }
}

/// Handle a WriteProperty request.
///
/// This synchronous handler makes no writes on a Command or Channel object's
/// behalf: it has no task to wait out a delay in. A Present_Value write that
/// selects a list with commands is accepted, and the run it starts ends at
/// once with every command unsuccessful, so In_Process is FALSE again on
/// return and All_Writes_Successful is FALSE (#1178). A Channel's
/// Present_Value write is accepted too, and the distribution it starts ends at
/// once without writing the members, so Write_Status reads FAILED on return
/// (#1151). The bundled [`BACnetServer`](crate::server::BACnetServer) runs
/// both.
pub fn handle_write_property(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<ObjectIdentifier, Error> {
    let (oid, _) = write::write_property(db, service_data, None, None, None)?;
    write::end_unmade(db, std::slice::from_ref(&oid));
    Ok(oid)
}
