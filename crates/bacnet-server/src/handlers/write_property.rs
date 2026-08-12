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

/// Handle a WritePropertyMultiple request.
///
/// Validates all properties first, then commits atomically. If any write fails,
/// all previously applied writes are rolled back. Returns the written object identifiers.
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
    let mut applied: Vec<(
        ObjectIdentifier,
        PropertyIdentifier,
        Option<u32>,
        PropertyValue,
    )> = Vec::new();

    for (oid, prop_id, array_index, value, priority) in &decoded_writes {
        // Enforce Object_Name uniqueness against the database index before
        // mutating the object, so a rejected duplicate leaves no trace.
        if *prop_id == PropertyIdentifier::OBJECT_NAME {
            check_and_prepare_name_write(db, oid, value)?;
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
        // non-commandable PRESENT_VALUE changed despite reporting failure
        // (ASHRAE 135-2020 §19.1.2).
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
            if let Ok(reliability) = object.read_property(PropertyIdentifier::RELIABILITY, None) {
                // Pushed before OUT_OF_SERVICE so reverse-order rollback
                // restores OOS first, then the client simulation.
                records.push((PropertyIdentifier::RELIABILITY, None, reliability));
            }
            if let Ok(out_of_service) = object.read_property(*prop_id, *array_index) {
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
        match object.write_property(*prop_id, *array_index, value.clone(), *priority) {
            Ok(()) => {
                // A successful Object_Name write changed the object's name field;
                // resync the database name index to the new name.
                if *prop_id == PropertyIdentifier::OBJECT_NAME {
                    db.update_name_index(oid);
                }
                for (rb_prop, rb_idx, rb_val) in rollback_records {
                    applied.push((*oid, rb_prop, rb_idx, rb_val));
                }
            }
            Err(e) => {
                for (rb_oid, rb_prop, rb_idx, rb_val) in applied.into_iter().rev() {
                    if let Some(obj) = db.get_mut(&rb_oid) {
                        // Restore the snapshotted state. For a PRIORITY_ARRAY slot
                        // the priority argument is ignored (write_priority_array_direct
                        // writes the indexed slot; a `Null` value relinquishes it);
                        // for any other property this re-writes the saved value.
                        let _ = obj.write_property(rb_prop, rb_idx, rb_val, None);
                        // The rollback restored the object's name field; resync
                        // the database index so it no longer maps the rolled-back
                        // name and once again maps the restored name.
                        if rb_prop == PropertyIdentifier::OBJECT_NAME {
                            db.update_name_index(&rb_oid);
                        }
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

/// Decode the `propertyValue` bytes of a WriteProperty-family request into
/// the `PropertyValue` handed to the object's write arm.
///
/// Framed ASN.1 properties with a leading context tag (e.g.
/// `Event_Parameters`) decode to one [`PropertyValue::ApplicationData`]
/// element via the generic decoder. `Recipient_List` is different: its
/// framed form is a `BACnetLIST` — a concatenation of destinations whose
/// members are application-tagged — so the generic single-element decode
/// would consume only the first member (the `valid_days` bit string) and
/// discard the rest (the whole-list decode gap tracked as #182). The object
/// layer owns the framed codec, so hand the complete content over verbatim;
/// the property's Clause 12 datatype is fixed by its identifier wherever it
/// appears (Notification Class, Notification Forwarder).
pub(crate) fn decode_write_property_value(
    property: PropertyIdentifier,
    bytes: &[u8],
) -> Result<PropertyValue, Error> {
    if property == PropertyIdentifier::RECIPIENT_LIST {
        return Ok(PropertyValue::ApplicationData(bytes.to_vec()));
    }
    Ok(bacnet_encoding::primitives::decode_application_value(bytes, 0)?.0)
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
