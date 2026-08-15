use super::*;

/// Reject an `OBJECT_NAME` write whose target name is already owned by a
/// different object, and refresh the database name index after a successful
/// rename.
///
/// `write_property(OBJECT_NAME, …)` mutates the object's name field in place
/// but does not touch [`ObjectDatabase`]'s secondary name index, so the index
/// would keep pointing at the stale name and uniqueness would go unenforced
/// unless the write boundary performs both checks. This helper centralizes
/// that database-owned validation/refresh for all write routes.
///
/// On `Ok` the name is available (or already belongs to `oid`) and the write
/// may proceed; the caller must invoke [`ObjectDatabase::update_name_index`]
/// once the underlying `write_property` succeeds. On `Err` the duplicate name
/// is reported via `DUPLICATE_NAME`.
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

struct PropertyRollback {
    property: PropertyIdentifier,
    array_index: Option<u32>,
    value: PropertyValue,
}

struct RollbackRecord {
    oid: ObjectIdentifier,
    written_property: PropertyIdentifier,
    properties: Vec<PropertyRollback>,
    object_state: Option<WritePropertyRollback>,
}

struct RollbackFailure {
    error: Error,
    residual_oids: Vec<ObjectIdentifier>,
}

fn record_rollback_failure(
    first_error: &mut Option<Error>,
    residual_oids: &mut Vec<ObjectIdentifier>,
    oid: ObjectIdentifier,
    error: Error,
) {
    if first_error.is_none() {
        *first_error = Some(error);
    }
    if !residual_oids.contains(&oid) {
        residual_oids.push(oid);
    }
}

fn rollback_writes(
    db: &mut ObjectDatabase,
    applied: Vec<RollbackRecord>,
) -> Result<(), RollbackFailure> {
    let mut first_error = None;
    let mut residual_oids = Vec::new();
    for rollback in applied.into_iter().rev() {
        let RollbackRecord {
            oid,
            written_property,
            properties,
            object_state,
        } = rollback;
        if properties.is_empty() && object_state.is_none() {
            let error = Error::Encoding(format!(
                "no rollback snapshot available for property {}",
                written_property.to_raw()
            ));
            tracing::error!(
                object = ?oid,
                property = written_property.to_raw(),
                "WPM rollback snapshot unavailable"
            );
            record_rollback_failure(&mut first_error, &mut residual_oids, oid, error);
            if written_property == PropertyIdentifier::OBJECT_NAME {
                db.update_name_index(&oid);
            }
            continue;
        }
        let mut rejected_properties = Vec::new();
        if let Some(object) = db.get_mut(&oid) {
            for property_rollback in properties.into_iter().rev() {
                let PropertyRollback {
                    property,
                    array_index,
                    value,
                } = property_rollback;
                // PRIORITY_ARRAY ignores the priority argument; other
                // properties replay their saved value.
                if let Err(error) =
                    object.write_property(property, array_index, value.clone(), None)
                {
                    // The object-state token may restore this value as a side
                    // effect. Check rejected replays after restoring the token.
                    rejected_properties.push((property, array_index, value, error));
                }
            }
            if let Some(state) = object_state {
                if let Err(error) = object.restore_write_property_rollback(state) {
                    tracing::error!(
                        object = ?oid,
                        %error,
                        "WPM object-state rollback failed"
                    );
                    record_rollback_failure(&mut first_error, &mut residual_oids, oid, error);
                }
            }
            for (property, array_index, value, error) in rejected_properties {
                // A rollback in the same write can restore this value as a
                // side effect (notably OOS restoring Reliability, or an
                // object-owned token restoring fallback-backed storage).
                let restored = object
                    .read_property(property, array_index)
                    .is_ok_and(|current| current == value);
                if !restored {
                    tracing::error!(
                        object = ?oid,
                        property = property.to_raw(),
                        %error,
                        "WPM property rollback failed"
                    );
                    record_rollback_failure(&mut first_error, &mut residual_oids, oid, error);
                }
            }
        }
        // A token may own state that also affects Object_Name. Refresh after
        // the complete write rollback rather than only after property replay.
        if written_property == PropertyIdentifier::OBJECT_NAME {
            db.update_name_index(&oid);
        }
    }

    match first_error {
        Some(error) => Err(RollbackFailure {
            error,
            residual_oids,
        }),
        None => Ok(()),
    }
}

fn error_after_rollback(
    db: &mut ObjectDatabase,
    applied: Vec<RollbackRecord>,
    write_error: Error,
    residual_oids: &mut Vec<ObjectIdentifier>,
) -> Error {
    match rollback_writes(db, applied) {
        Ok(()) => write_error,
        Err(rollback_failure) => {
            for oid in rollback_failure.residual_oids {
                if !residual_oids.contains(&oid) {
                    residual_oids.push(oid);
                }
            }
            Error::Encoding(format!(
                "WritePropertyMultiple failed ({write_error}); rollback failed ({})",
                rollback_failure.error
            ))
        }
    }
}

/// Handle a WritePropertyMultiple request.
///
/// Validates all properties first, then applies the repository's atomic-write
/// policy. If any write fails, every snapshotted write is rolled back; a
/// restoration failure is returned instead of being hidden in tracing. Returns
/// the written object identifiers.
///
/// `OBJECT_NAME` writes are routed through the database name index: a
/// duplicate name is rejected up front, and a successful rename refreshes the
/// index. Rollback restores the index for any rolled-back `OBJECT_NAME` write
/// so the pre-transaction name mappings are preserved.
///
/// `PRESENT_VALUE` writes on a commandable object are snapshotted at the
/// **priority-array slot** the write targets, not as the effective present
/// value. Reading `PRESENT_VALUE` returns the resolved (highest-priority)
/// value, so a generic "read it, write it back" rollback would instead write
/// that resolved value to priority 16 — leaving the originally-changed slot
/// un-restored and adding a spurious priority-16 command. The snapshot reads
/// `PRIORITY_ARRAY[priority]` (the exact slot about to change, or `Null` if it
/// was relinquished) and the rollback restores that slot directly; a `Null`
/// snapshot relinquishes the slot again. Non-commandable objects (where
/// `PRIORITY_ARRAY` is not readable) fall back to the generic value snapshot.
///
/// `OUT_OF_SERVICE` writes on objects with `Reliability` snapshot both
/// properties. Returning to service restores the pre-simulation Reliability,
/// so rolling back only `OUT_OF_SERVICE` would destroy the client's simulated
/// value and the object's saved evaluated value. Rollback replays
/// `OUT_OF_SERVICE` first and then `RELIABILITY`, reconstructing both visible
/// state and the object's private saved-value slot.
///
/// Objects may also supply an opaque rollback token when property readback is
/// not state-equivalent. Tokens cover reset side effects, derived properties,
/// destructive log writes, command slots that cannot be written directly, and
/// fallback-backed storage. Replaying the readable value alone would not
/// restore the pre-request state. This rollback is repository policy; Clause
/// 15.10 itself permits preceding writes to remain applied when a later write
/// fails.
pub fn handle_write_property_multiple(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<Vec<ObjectIdentifier>, Error> {
    handle_write_property_multiple_with_residuals(db, service_data).0
}

pub(crate) fn handle_write_property_multiple_with_residuals(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> (Result<Vec<ObjectIdentifier>, Error>, Vec<ObjectIdentifier>) {
    let mut residual_oids = Vec::new();
    let result = handle_write_property_multiple_inner(db, service_data, &mut residual_oids);
    (result, residual_oids)
}

fn handle_write_property_multiple_inner(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    residual_oids: &mut Vec<ObjectIdentifier>,
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
            // Clause 15.9.1.3: an array index on a non-array property fails
            // the request. Gating in the validation phase keeps the request
            // atomic — the commit loop never starts for a rejected index.
            let object = db.get(&oid).expect("existence checked above");
            if prop.property_array_index.is_some()
                && !object.is_array_property(prop.property_identifier)
            {
                return Err(Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::PROPERTY_IS_NOT_AN_ARRAY.to_raw() as u32,
                });
            }
            let value = decode_write_property_value(prop.property_identifier, &prop.value)?;
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
    let mut applied: Vec<RollbackRecord> = Vec::new();

    for (oid, prop_id, array_index, value, priority) in &decoded_writes {
        // Enforce Object_Name uniqueness against the database index before
        // mutating the object, so a rejected duplicate leaves no trace.
        if *prop_id == PropertyIdentifier::OBJECT_NAME {
            if let Err(error) = check_and_prepare_name_write(db, oid, value) {
                return Err(error_after_rollback(db, applied, error, residual_oids));
            }
        }
        let object = db.get_mut(oid).unwrap();
        // Capture a state-equivalent snapshot for rollback. For a commandable
        // PRESENT_VALUE the readable value is the resolved priority-array
        // output, not the slot being written, so snapshot the priority-array
        // slot itself (see the function doc). `rollback_records` holds the
        // (property, index, value) entries to restore; for commandable
        // PRESENT_VALUE that is PRIORITY_ARRAY[priority], for OUT_OF_SERVICE
        // it can also include RELIABILITY, and otherwise it is the property as
        // written. `read_property` is best-effort: a write-only property has no
        // readable value and yields no rollback record (matches prior behavior).
        //
        // A non-commandable object (e.g. an AnalogInput placed out-of-service,
        // or a Command/Timer/Color object whose PRESENT_VALUE is always
        // writable) has no PRIORITY_ARRAY, so reading that slot returns `Err`.
        // In that case fall back to snapshotting PRESENT_VALUE directly — the
        // same value the old code snapshotted — so the write still rolls back.
        // Without this fallback a failed multi-write would leave the
        // non-commandable PRESENT_VALUE changed despite this implementation's
        // all-or-nothing WPM policy.
        let rollback_records = if *prop_id == PropertyIdentifier::PRESENT_VALUE {
            // The write targets priority `priority.unwrap_or(16)` (matching
            // `write_priority_array!`). Snapshot that exact slot so rollback
            // restores it rather than writing the resolved value to priority 16.
            // An out-of-range priority (e.g. 0) makes a *commandable* write fail,
            // so its snapshot is discarded anyway — but only snapshot a valid
            // slot to avoid reading the array-size element (index 0) by mistake.
            let slot = priority.unwrap_or(16);
            if (1..=16).contains(&slot) {
                object
                    .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(slot as u32))
                    .ok()
                    .map(|v| (PropertyIdentifier::PRIORITY_ARRAY, Some(slot as u32), v))
                    .or_else(|| {
                        object
                            .read_property(*prop_id, *array_index)
                            .ok()
                            .map(|v| (*prop_id, *array_index, v))
                    })
                    .into_iter()
                    .collect()
            } else {
                // Out-of-range priority: a commandable write will fail (so the
                // snapshot is discarded), but a non-commandable write ignores the
                // priority and succeeds — snapshot its PRESENT_VALUE so it still
                // rolls back.
                object
                    .read_property(*prop_id, *array_index)
                    .ok()
                    .map(|v| (*prop_id, *array_index, v))
                    .into_iter()
                    .collect()
            }
        } else if *prop_id == PropertyIdentifier::OUT_OF_SERVICE {
            let mut records = Vec::new();
            if let Ok(out_of_service) = object.read_property(*prop_id, *array_index) {
                // Leaving OOS may replace the client's simulated Reliability
                // with a saved evaluated value. Preserve both only in that
                // direction. Entering OOS needs no separate Reliability replay:
                // restoring OOS to FALSE restores the saved value itself, and a
                // subsequent network Reliability write would be rejected.
                if out_of_service == PropertyValue::Boolean(true) {
                    if let Ok(reliability) =
                        object.read_property(PropertyIdentifier::RELIABILITY, None)
                    {
                        // Pushed before OUT_OF_SERVICE so reverse rollback
                        // restores OOS first, then the client simulation.
                        records.push((PropertyIdentifier::RELIABILITY, None, reliability));
                    }
                }
                records.push((*prop_id, *array_index, out_of_service));
            }
            records
        } else {
            object
                .read_property(*prop_id, *array_index)
                .ok()
                .map(|v| (*prop_id, *array_index, v))
                .into_iter()
                .collect()
        };
        let properties = rollback_records
            .into_iter()
            .map(|(property, array_index, value)| PropertyRollback {
                property,
                array_index,
                value,
            })
            .collect::<Vec<_>>();
        // Readable snapshots must be captured before this hook: destructive
        // writes may move private state into their rollback token.
        let object_rollback = object.capture_write_property_rollback(*prop_id, value);
        let has_rollback = !properties.is_empty() || object_rollback.is_some();
        match object.write_property(*prop_id, *array_index, value.clone(), *priority) {
            Ok(()) => {
                // A successful Object_Name write changed the object's name field;
                // resync the database name index to the new name.
                if *prop_id == PropertyIdentifier::OBJECT_NAME {
                    db.update_name_index(oid);
                }
                applied.push(RollbackRecord {
                    oid: *oid,
                    written_property: *prop_id,
                    properties,
                    object_state: object_rollback,
                });
            }
            Err(e) => {
                // `BACnetObject::write_property` requires errors to be
                // side-effect-free. A snapshot still lets rollback defend
                // against a custom implementation that violates the contract.
                if has_rollback {
                    applied.push(RollbackRecord {
                        oid: *oid,
                        written_property: *prop_id,
                        properties,
                        object_state: object_rollback,
                    });
                }
                return Err(error_after_rollback(db, applied, e, residual_oids));
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

/// PROPERTY / INVALID_DATA_ENCODING for a `propertyValue` payload that does
/// not decode (Clause 15.9.1.3: "The encoding is not valid for the datatype
/// of the property").
fn invalid_data_encoding_error() -> Error {
    Error::Protocol {
        class: ErrorClass::PROPERTY.to_raw() as u32,
        code: ErrorCode::INVALID_DATA_ENCODING.to_raw() as u32,
    }
}

/// Decode the `propertyValue` bytes of a WriteProperty-family request into
/// the `PropertyValue` handed to the object's write arm.
///
/// The whole payload is consumed (#182): elements are decoded with
/// `decode_application_value` until the input is exhausted, mirroring
/// `encode_property_value`'s `List` flattening. Exactly one element yields a
/// scalar `PropertyValue`; more than one yields `PropertyValue::List`. A
/// partial element at the tail — and any other decode failure — is
/// PROPERTY / INVALID_DATA_ENCODING; trailing bytes are never silently
/// dropped on the floor between the single-element decoder and the object's
/// write arm.
///
/// Context-tagged content keeps the generic decoder's `ApplicationData`
/// widening: framed ASN.1 properties (e.g. `Event_Parameters`, one
/// opening/closing CHOICE element) arrive as one `ApplicationData` scalar,
/// and context-tagged member productions (the Loop/Accumulator reference
/// properties, primitive context tags [0]/[1]/[2]) arrive as one
/// `ApplicationData` element per member — the object arm owns their
/// reassembly and framed decode (Clause 12.17).
///
/// `Recipient_List` stays verbatim: its framed form is a `BACnetLIST` — a
/// concatenation of destinations whose members mix application tags with the
/// recipient CHOICE's context tags — so the object layer keeps owning its
/// framed codec, and the property's Clause 12 datatype is fixed by its
/// identifier wherever it appears (Notification Class, Notification
/// Forwarder).
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
        0 => Err(invalid_data_encoding_error()),
        1 => Ok(values.pop().expect("one element present")),
        _ => Ok(PropertyValue::List(values)),
    }
}

/// Handle a WriteProperty request.
///
/// Returns the written object identifier for COV/event notifications.
///
/// `OBJECT_NAME` writes are routed through the database name index: a
/// duplicate name is rejected up front, and a successful rename refreshes
/// the index so lookups resolve to the new name and the old name is freed.
pub fn handle_write_property(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<ObjectIdentifier, Error> {
    let request = WritePropertyRequest::decode(service_data)?;
    let oid = request.object_identifier;

    // Reject unknown objects before decoding the value, preserving the
    // historical error precedence (UNKNOWN_OBJECT for a bad OID regardless of
    // whether the property_value bytes are also malformed).
    if db.get(&oid).is_none() {
        return Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
        });
    }

    // Clause 15.9.1.3: an array index on a non-array property is rejected
    // with PROPERTY / PROPERTY_IS_NOT_AN_ARRAY before any decoding or
    // mutation. The array/list decision belongs to the object.
    if request.property_array_index.is_some()
        && !db
            .get(&oid)
            .expect("existence checked above")
            .is_array_property(request.property_identifier)
    {
        return Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::PROPERTY_IS_NOT_AN_ARRAY.to_raw() as u32,
        });
    }

    let value = decode_write_property_value(request.property_identifier, &request.property_value)?;

    // Enforce Object_Name uniqueness against the database index before mutating.
    if request.property_identifier == PropertyIdentifier::OBJECT_NAME {
        check_and_prepare_name_write(db, &oid, &value)?;
    }

    let object = db.get_mut(&oid).expect("existence checked above");

    object.write_property(
        request.property_identifier,
        request.property_array_index,
        value,
        request.priority,
    )?;

    // Resync the database name index to the object's new name.
    if request.property_identifier == PropertyIdentifier::OBJECT_NAME {
        db.update_name_index(&oid);
    }

    Ok(oid)
}
