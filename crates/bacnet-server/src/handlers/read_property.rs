use super::*;

/// Handle a ReadProperty request against standalone object data.
///
/// Looks up the object and property in the database, encodes the value,
/// and returns the ReadPropertyACK service bytes. This low-level helper has
/// no server context: the Device's `Active_COV_Subscriptions` reads as the
/// object's standalone empty list. A running `BACnetServer` projects that
/// property live from its COV subscription table instead.
pub fn handle_read_property(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let request = ReadPropertyRequest::decode(service_data)?;
    read_property_request_observed(db, None, &request, buf, |_, _, _| {})
}

/// Server ReadProperty evaluator over one decoded request. `live` carries the
/// request-local Device `Active_COV_Subscriptions`; observations carry only
/// execution outcomes, never the read value.
pub(crate) fn read_property_request_observed(
    db: &ObjectDatabase,
    live: Option<&ActiveCovSubscriptions>,
    request: &ReadPropertyRequest,
    buf: &mut BytesMut,
    mut completed: impl FnMut(ObjectIdentifier, &ReadPropertyRequest, &Result<(), Error>),
) -> Result<(), Error> {
    let lookup_oid = resolve_device_wildcard(db, &request.object_identifier);
    let result = read_property_decoded(db, live, request, lookup_oid, buf);
    completed(lookup_oid, request, &result);
    result
}

/// Evaluate one property read with ReadProperty error precedence: unknown
/// object, then non-array index, then the live Device projection or the
/// object's own reader.
pub(crate) fn read_property_value(
    db: &ObjectDatabase,
    live: Option<&ActiveCovSubscriptions>,
    lookup_oid: ObjectIdentifier,
    property: PropertyIdentifier,
    array_index: Option<u32>,
) -> Result<PropertyValue, Error> {
    let object = db.get(&lookup_oid).ok_or(Error::Protocol {
        class: ErrorClass::OBJECT.to_raw() as u32,
        code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
    })?;

    // Clause 15.5.1.3: an array index on a non-array property is rejected
    // with PROPERTY / PROPERTY_IS_NOT_AN_ARRAY. The array/list decision
    // belongs to the object (identifier-static whitelists cannot express the
    // type-dependent identifiers, e.g. ALARM_VALUES), so the handler defers
    // to the trait query.
    if array_index.is_some() && !object.is_array_property(property) {
        return Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::PROPERTY_IS_NOT_AN_ARRAY.to_raw() as u32,
        });
    }

    match live.and_then(|live| live.resolve(lookup_oid, property)) {
        Some(value) => Ok(value),
        None => object.read_property(property, array_index),
    }
}

fn read_property_decoded(
    db: &ObjectDatabase,
    live: Option<&ActiveCovSubscriptions>,
    request: &ReadPropertyRequest,
    lookup_oid: ObjectIdentifier,
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let value = read_property_value(
        db,
        live,
        lookup_oid,
        request.property_identifier,
        request.property_array_index,
    )?;

    let mut value_buf = BytesMut::new();
    encode_property_value(&mut value_buf, &value)?;

    let ack = ReadPropertyACK {
        object_identifier: lookup_oid,
        property_identifier: request.property_identifier,
        property_array_index: request.property_array_index,
        property_value: value_buf.to_vec(),
    };

    ack.encode(buf);
    Ok(())
}

/// The local Device that wildcard instance 4194303 names. Ordinary and live
/// Device reads share this one selection.
fn selected_device(db: &ObjectDatabase) -> Option<ObjectIdentifier> {
    db.list_objects()
        .into_iter()
        .find(|candidate| candidate.object_type() == ObjectType::DEVICE)
}

fn is_device_wildcard(oid: &ObjectIdentifier) -> bool {
    oid.object_type() == ObjectType::DEVICE && oid.instance_number() == 4194303
}

/// Resolve Device wildcard instance 4194303 to the actual Device object.
pub(crate) fn resolve_device_wildcard(
    db: &ObjectDatabase,
    oid: &ObjectIdentifier,
) -> ObjectIdentifier {
    if is_device_wildcard(oid) {
        if let Some(device) = selected_device(db) {
            return device;
        }
    }
    *oid
}

/// The selected Device when `(lookup_oid, property)` is its server-owned
/// `Active_COV_Subscriptions`; any other read needs no COV table snapshot.
pub(crate) fn active_cov_device(
    db: &ObjectDatabase,
    lookup_oid: ObjectIdentifier,
    property: PropertyIdentifier,
) -> Option<ObjectIdentifier> {
    if property != PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS {
        return None;
    }
    selected_device(db).filter(|device| *device == lookup_oid)
}

/// The selected Device when any ReadPropertyMultiple reference to it may
/// select `Active_COV_Subscriptions`, explicitly or through ALL, REQUIRED or
/// OPTIONAL expansion. One snapshot then serves every such row.
pub(crate) fn active_cov_device_for_rpm(
    db: &ObjectDatabase,
    request: &ReadPropertyMultipleRequest,
) -> Option<ObjectIdentifier> {
    let may_select = |reference: &bacnet_services::common::PropertyReference| {
        matches!(
            reference.property_identifier,
            PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS
                | PropertyIdentifier::ALL
                | PropertyIdentifier::REQUIRED
                | PropertyIdentifier::OPTIONAL
        )
    };
    // Only a request that may select the property pays the Device scan.
    let mut specs = request
        .list_of_read_access_specs
        .iter()
        .filter(|spec| spec.list_of_property_references.iter().any(may_select))
        .peekable();
    specs.peek()?;
    let device = selected_device(db)?;
    specs
        .any(|spec| spec.object_identifier == device || is_device_wildcard(&spec.object_identifier))
        .then_some(device)
}

fn expand_property_reference(
    object: &dyn bacnet_objects::traits::BACnetObject,
    property_identifier: PropertyIdentifier,
) -> Vec<PropertyIdentifier> {
    let metadata = object.property_metadata();
    if !metadata.is_empty() {
        return match property_identifier {
            PropertyIdentifier::ALL => metadata
                .iter()
                .filter_map(|row| {
                    (row.property_identifier != PropertyIdentifier::PROPERTY_LIST)
                        .then_some(row.property_identifier)
                })
                .collect(),
            PropertyIdentifier::REQUIRED => metadata
                .iter()
                .filter_map(|row| {
                    (row.property_identifier != PropertyIdentifier::PROPERTY_LIST
                        && row.is_required())
                    .then_some(row.property_identifier)
                })
                .collect(),
            PropertyIdentifier::OPTIONAL => metadata
                .iter()
                .filter_map(|row| (!row.is_required()).then_some(row.property_identifier))
                .collect(),
            other => vec![other],
        };
    }

    match property_identifier {
        PropertyIdentifier::ALL => object.property_list().to_vec(),
        PropertyIdentifier::REQUIRED => object.required_properties().to_vec(),
        PropertyIdentifier::OPTIONAL => {
            let required: std::collections::HashSet<PropertyIdentifier> =
                object.required_properties().iter().copied().collect();
            object
                .property_list()
                .iter()
                .copied()
                .filter(|property| !required.contains(property))
                .collect()
        }
        other => vec![other],
    }
}

/// Clause 15.7.3.2.2.2 includes an index only for a declared array property.
/// An unknown object/property or unavailable declaration conservatively omits
/// it. Classification must not probe `read_property` or change error precedence.
pub(super) fn rpm_response_index(
    object: Option<&dyn bacnet_objects::traits::BACnetObject>,
    property: PropertyIdentifier,
    requested: Option<u32>,
) -> Option<u32> {
    let index = requested?;
    let object = object?;
    let metadata = object.property_metadata();
    let present = if metadata.is_empty() {
        object.property_list().contains(&property)
    } else {
        metadata
            .iter()
            .any(|row| row.property_identifier == property)
    };
    (present && object.is_array_property(property)).then_some(index)
}

/// Handle a ReadPropertyMultiple request.
///
/// Per-property errors are returned inline rather than failing the entire request.
/// This legacy low-level helper has no configured service budget. Configured
/// `BACnetServer` dispatch uses a separate bounded implementation.
pub fn handle_read_property_multiple(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let request = ReadPropertyMultipleRequest::decode(service_data)?;

    let mut results = Vec::new();
    for spec in &request.list_of_read_access_specs {
        let mut elements = Vec::new();

        let lookup_oid = resolve_device_wildcard(db, &spec.object_identifier);
        match db.get(&lookup_oid) {
            Some(object) => {
                for prop_ref in &spec.list_of_property_references {
                    let prop_ids = expand_property_reference(object, prop_ref.property_identifier);

                    for prop_id in prop_ids {
                        let array_index = if prop_ref.property_identifier == prop_id {
                            prop_ref.property_array_index
                        } else {
                            None
                        };
                        let response_index = rpm_response_index(Some(object), prop_id, array_index);
                        // Same gate as ReadProperty (Clause 15.5.1.3): an
                        // array index on a non-array property fails this
                        // reference inline; sibling references still run.
                        // ALL/REQUIRED/OPTIONAL expansions attach no index,
                        // so they pass through untouched.
                        if array_index.is_some() && !object.is_array_property(prop_id) {
                            elements.push(ReadResultElement {
                                property_identifier: prop_id,
                                property_array_index: response_index,
                                property_value: None,
                                error: Some((
                                    ErrorClass::PROPERTY,
                                    ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
                                )),
                            });
                            continue;
                        }
                        match object.read_property(prop_id, array_index) {
                            Ok(value) => {
                                let mut value_buf = BytesMut::new();
                                match encode_property_value(&mut value_buf, &value) {
                                    Ok(()) => {
                                        elements.push(ReadResultElement {
                                            property_identifier: prop_id,
                                            property_array_index: response_index,
                                            property_value: Some(value_buf.to_vec()),
                                            error: None,
                                        });
                                    }
                                    Err(_) => {
                                        elements.push(ReadResultElement {
                                            property_identifier: prop_id,
                                            property_array_index: response_index,
                                            property_value: None,
                                            error: Some((ErrorClass::PROPERTY, ErrorCode::OTHER)),
                                        });
                                    }
                                }
                            }
                            Err(e) => {
                                let (err_class, err_code) = match &e {
                                    Error::Protocol { class, code } => (
                                        ErrorClass::from_raw(*class as u16),
                                        ErrorCode::from_raw(*code as u16),
                                    ),
                                    _ => (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
                                };
                                elements.push(ReadResultElement {
                                    property_identifier: prop_id,
                                    property_array_index: response_index,
                                    property_value: None,
                                    error: Some((err_class, err_code)),
                                });
                            }
                        }
                    }
                }
            }
            None => {
                for prop_ref in &spec.list_of_property_references {
                    elements.push(ReadResultElement {
                        property_identifier: prop_ref.property_identifier,
                        property_array_index: rpm_response_index(
                            None,
                            prop_ref.property_identifier,
                            prop_ref.property_array_index,
                        ),
                        property_value: None,
                        error: Some((ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT)),
                    });
                }
            }
        }

        results.push(ReadAccessResult {
            object_identifier: spec.object_identifier,
            list_of_results: elements,
        });
    }

    let ack = ReadPropertyMultipleACK {
        list_of_read_access_results: results,
    };
    ack.encode(buf);
    Ok(())
}

#[cfg(test)]
#[path = "tests/rpm_result_index.rs"]
mod rpm_result_index_tests;
