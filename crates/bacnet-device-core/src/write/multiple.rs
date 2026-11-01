//! WritePropertyMultiple: each attempt checked and committed in wire order,
//! the successful prefix kept when a later attempt fails.
use super::{
    Applied, WriteCommitObserver, WriteTarget, check_and_prepare_name_write, commit_attempt,
    gate_and_decode_write, localize, protocol_error,
};
use alloc::vec::Vec;
use bacnet_objects::database::ObjectDatabase;
use bacnet_services::wpm::{WritePropertyMultipleCursor, WritePropertyMultipleEvent};
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier, RejectReason};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

/// What a WritePropertyMultiple did: the answer it draws, and the objects
/// its successful attempts changed.
pub enum WpmOutcome {
    /// Every attempt succeeded: a SimpleACK.
    Success {
        /// The objects the attempts changed, each once, in the order of
        /// their first change.
        committed_oids: Vec<ObjectIdentifier>,
    },
    /// An attempt failed: the WritePropertyMultiple Error naming it. The
    /// attempts before it stay committed.
    Error {
        /// Why the attempt failed.
        error: Error,
        /// The attempt that failed.
        first_failed_write_attempt: BACnetObjectPropertyReference,
        /// The objects the earlier attempts changed.
        committed_oids: Vec<ObjectIdentifier>,
    },
    /// The request didn't decode before any attempt succeeded: a Reject.
    Reject {
        /// The Reject's reason.
        reason: RejectReason,
    },
}

type WritePropertyMultipleGate<'a> =
    &'a dyn Fn(&bacnet_services::wpm::WritePropertyAttempt) -> Result<(), Error>;

/// Execute WPM incrementally in wire order.
///
/// `authorize`, when given, decides each attempt after its checks and before
/// it commits. The complete successful prefix remains committed if a later
/// attempt fails.
pub fn write_property_multiple(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    authorize: Option<WritePropertyMultipleGate<'_>>,
    mut observer: Option<&mut dyn WriteCommitObserver>,
    source: Option<&bacnet_objects::device::AuditWriteSource>,
    command_origin: Option<&bacnet_objects::command_source::CommandOrigin>,
) -> WpmOutcome {
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
                return WpmOutcome::Success { committed_oids };
            }
            Err(cursor_error) => {
                use bacnet_services::wpm::WritePropertyMultipleFailureKind;
                let code = match cursor_error.kind {
                    WritePropertyMultipleFailureKind::PriorityOutOfRange => {
                        ErrorCode::PARAMETER_OUT_OF_RANGE
                    }
                    WritePropertyMultipleFailureKind::Syntax(reason) if !wrote => {
                        return WpmOutcome::Reject { reason };
                    }
                    WritePropertyMultipleFailureKind::Syntax(_) => ErrorCode::INVALID_TAG,
                };
                return WpmOutcome::Error {
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
            Ok(value) => localize(db, oid, property, value),
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
) -> WpmOutcome {
    WpmOutcome::Error {
        error,
        first_failed_write_attempt,
        committed_oids,
    }
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
