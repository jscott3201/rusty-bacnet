use super::*;

/// Handle a WritePropertyMultiple request.
///
/// Validates all properties first, then commits atomically. If any write fails,
/// all previously applied writes are rolled back. Returns the written object identifiers.
pub fn handle_write_property_multiple(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<Vec<ObjectIdentifier>, Error> {
    let request = WritePropertyMultipleRequest::decode(service_data)?;

    // Validate: decode all values and verify objects exist.
    #[allow(clippy::type_complexity)]
    let mut decoded_writes: Vec<(
        ObjectIdentifier,
        PropertyIdentifier,
        Option<u32>,
        PropertyValue,
        Option<u8>,
    )> = Vec::new();

    for spec in &request.list_of_write_access_specs {
        let oid = spec.object_identifier;
        if db.get(&oid).is_none() {
            return Err(Error::Protocol {
                class: ErrorClass::OBJECT.to_raw() as u32,
                code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
            });
        }
        for prop in &spec.list_of_properties {
            let (value, _) = bacnet_encoding::primitives::decode_application_value(&prop.value, 0)?;
            decoded_writes.push((
                oid,
                prop.property_identifier,
                prop.property_array_index,
                value,
                prop.priority,
            ));
        }
    }

    // Commit: apply all writes, rolling back on failure.
    let mut applied: Vec<(
        ObjectIdentifier,
        PropertyIdentifier,
        Option<u32>,
        PropertyValue,
    )> = Vec::new();

    for (oid, prop_id, array_index, value, priority) in &decoded_writes {
        let object = db.get_mut(oid).unwrap();
        // Save old value for rollback (best-effort; read may fail for write-only props).
        let old_value = object.read_property(*prop_id, *array_index).ok();
        match object.write_property(*prop_id, *array_index, value.clone(), *priority) {
            Ok(()) => {
                if let Some(old) = old_value {
                    applied.push((*oid, *prop_id, *array_index, old));
                }
            }
            Err(e) => {
                for (rb_oid, rb_prop, rb_idx, rb_val) in applied.into_iter().rev() {
                    if let Some(obj) = db.get_mut(&rb_oid) {
                        let _ = obj.write_property(rb_prop, rb_idx, rb_val, None);
                    }
                }
                return Err(e);
            }
        }
    }

    let mut written_oids = Vec::new();
    for (oid, _, _, _, _) in &decoded_writes {
        if !written_oids.contains(oid) {
            written_oids.push(*oid);
        }
    }

    Ok(written_oids)
}

/// Handle a WriteProperty request.
///
/// Returns the written object identifier for COV/event notifications.
pub fn handle_write_property(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<ObjectIdentifier, Error> {
    let request = WritePropertyRequest::decode(service_data)?;
    let oid = request.object_identifier;

    let object = db.get_mut(&oid).ok_or(Error::Protocol {
        class: ErrorClass::OBJECT.to_raw() as u32,
        code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
    })?;

    let (value, _) =
        bacnet_encoding::primitives::decode_application_value(&request.property_value, 0)?;

    object.write_property(
        request.property_identifier,
        request.property_array_index,
        value,
        request.priority,
    )?;

    Ok(oid)
}
