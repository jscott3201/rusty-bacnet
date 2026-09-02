use super::*;

/// Validate database-owned Object_Name uniqueness before mutation.
fn check_and_prepare_name_write(
    db: &ObjectDatabase,
    oid: &ObjectIdentifier,
    value: &PropertyValue,
) -> Result<(), Error> {
    if let PropertyValue::CharacterString(new_name) = value {
        db.check_name_available(oid, new_name)?;
    }
    Ok(())
}

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
pub fn handle_write_property_multiple(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<Vec<ObjectIdentifier>, Error> {
    let mut snapshots = crate::life_safety_cov::LifeSafetyCovSnapshots::default();
    match handle_write_property_multiple_detailed(db, service_data, &mut snapshots) {
        WritePropertyMultipleOutcome::Success { committed_oids } => Ok(committed_oids),
        WritePropertyMultipleOutcome::Error { error, .. } => Err(error),
        WritePropertyMultipleOutcome::Reject { reason } => Err(Error::Reject {
            reason: reason.to_raw(),
        }),
    }
}

/// Execute WPM incrementally in wire order for server dispatch.
pub(crate) fn handle_write_property_multiple_detailed(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    snapshots: &mut crate::life_safety_cov::LifeSafetyCovSnapshots,
) -> WritePropertyMultipleOutcome {
    let mut cursor = WritePropertyMultipleCursor::new(service_data);
    let mut committed_oids = Vec::new();

    loop {
        let event = match cursor.next_event() {
            Ok(Some(event)) => event,
            Ok(None) => {
                return WritePropertyMultipleOutcome::Success { committed_oids };
            }
            Err(cursor_error) if committed_oids.is_empty() => {
                return WritePropertyMultipleOutcome::Reject {
                    reason: cursor_error.reject_reason,
                };
            }
            Err(cursor_error) => {
                return WritePropertyMultipleOutcome::Error {
                    error: protocol_error(ErrorClass::SERVICES, ErrorCode::INVALID_TAG),
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
        let reference = attempt.reference;
        let oid = reference.object_identifier;
        let property = PropertyIdentifier::from_raw(reference.property_identifier);

        let Some(object) = db.get(&oid) else {
            return semantic_failure(
                protocol_error(ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
                reference,
                committed_oids,
            );
        };
        if reference.property_array_index.is_some() && !object.is_array_property(property) {
            return semantic_failure(
                protocol_error(ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
                reference,
                committed_oids,
            );
        }
        let value = match decode_write_property_value(property, &attempt.value) {
            Ok(value) => value,
            Err(error) => return semantic_failure(error, reference, committed_oids),
        };
        if property == PropertyIdentifier::OBJECT_NAME {
            if let Err(error) = check_and_prepare_name_write(db, &oid, &value) {
                return semantic_failure(error, reference, committed_oids);
            }
        }

        snapshots.capture_before_write(db, oid);
        let write = db
            .get_mut(&oid)
            .expect("existence checked above")
            .write_property(
                property,
                reference.property_array_index,
                value,
                attempt.priority,
            );
        if let Err(error) = write {
            return semantic_failure(error, reference, committed_oids);
        }
        if property == PropertyIdentifier::OBJECT_NAME {
            db.update_name_index(&oid);
        }
        if !committed_oids.contains(&oid) {
            committed_oids.push(oid);
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
    Error::Protocol {
        class: class.to_raw() as u32,
        code: code.to_raw() as u32,
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

/// PROPERTY / INVALID_DATA_ENCODING for an undecodable propertyValue payload.
fn invalid_data_encoding_error() -> Error {
    protocol_error(ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_ENCODING)
}

/// Decode the complete propertyValue payload handed to an object write arm.
pub(crate) fn decode_write_property_value(
    property: PropertyIdentifier,
    bytes: &[u8],
) -> Result<PropertyValue, Error> {
    if property == PropertyIdentifier::EVENT_PARAMETERS && bytes.starts_with(&[0xfe, 0xff]) {
        use bacnet_types::constructed::BACnetEventParameter;

        return match bacnet_encoding::constructed::decode_event_parameter(bytes, 0) {
            Ok((BACnetEventParameter::Opaque { tag, data }, consumed))
                if tag == u8::MAX && consumed == bytes.len() =>
            {
                Ok(PropertyValue::OctetString(data))
            }
            _ => Err(invalid_data_encoding_error()),
        };
    }
    if property == PropertyIdentifier::RECIPIENT_LIST {
        return Ok(PropertyValue::ApplicationData(bytes.to_vec()));
    }
    let mut values = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let (value, new_offset) =
            bacnet_encoding::primitives::decode_application_value(bytes, offset)
                .map_err(|_| invalid_data_encoding_error())?;
        values.push(value);
        offset = new_offset;
    }
    match values.len() {
        0 if property == PropertyIdentifier::FAULT_SIGNALS => Ok(PropertyValue::List(values)),
        0 => Err(invalid_data_encoding_error()),
        1 => Ok(values.pop().expect("one element present")),
        _ => Ok(PropertyValue::List(values)),
    }
}

/// Handle a WriteProperty request.
pub fn handle_write_property(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<ObjectIdentifier, Error> {
    let request = WritePropertyRequest::decode(service_data)?;
    let oid = request.object_identifier;

    if db.get(&oid).is_none() {
        return Err(protocol_error(
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        ));
    }
    if request.property_array_index.is_some()
        && !db
            .get(&oid)
            .expect("existence checked above")
            .is_array_property(request.property_identifier)
    {
        return Err(protocol_error(
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ));
    }
    let value = decode_write_property_value(request.property_identifier, &request.property_value)?;
    if request.property_identifier == PropertyIdentifier::OBJECT_NAME {
        check_and_prepare_name_write(db, &oid, &value)?;
    }
    db.get_mut(&oid)
        .expect("existence checked above")
        .write_property(
            request.property_identifier,
            request.property_array_index,
            value,
            request.priority,
        )?;
    if request.property_identifier == PropertyIdentifier::OBJECT_NAME {
        db.update_name_index(&oid);
    }
    Ok(oid)
}
