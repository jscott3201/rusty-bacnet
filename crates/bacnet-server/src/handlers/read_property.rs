use super::*;
use bacnet_device_core::read::{
    self, NoOverlay, ReadFailure, ReadScope, group, read_target_object, rpm_response_index,
};
use bacnet_types::constructed::PropertyReference;
use bacnet_types::error::RemoteError;

/// The handler tests' ReadProperty: bacnet-device-core's staged read with no
/// overlay and no work limit, writing the ReadPropertyACK service bytes. A
/// built-in Device follows its declared service profile, including absent or
/// empty COV lists; running server reads use the executor's view instead.
#[cfg(test)]
pub(crate) fn handle_read_property(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let request = ReadPropertyRequest::decode(service_data).map_err(Error::into_request_reject)?;
    let scope = ReadScope::UNLIMITED;
    let lookup_oid = read::resolve_target(db, &scope, &request.object_identifier);
    let plan = read::plan_property(
        db,
        &scope,
        None::<&NoOverlay>,
        lookup_oid,
        request.property_identifier,
        request.property_array_index,
    )
    .map_err(ReadFailure::unlimited)?;
    read::read_property_ack(db, None::<&NoOverlay>, &request, plan, buf, |_, _, _| {})
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
    let request =
        ReadPropertyMultipleRequest::decode(service_data).map_err(Error::into_request_reject)?;

    let mut results = Vec::new();
    for spec in &request.list_of_read_access_specs {
        let mut elements = Vec::new();

        let lookup_oid = read::resolve_target(db, &ReadScope::UNLIMITED, &spec.object_identifier);
        match read_target_object(db, &lookup_oid) {
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
                        let reference = PropertyReference {
                            property_identifier: prop_id,
                            property_array_index: array_index,
                        };
                        match group::plan_property(
                            db,
                            &ReadScope::UNLIMITED,
                            None::<&NoOverlay>,
                            object,
                            &reference,
                        )
                        .map_err(ReadFailure::unlimited)
                        .and_then(|members| {
                            group::read_served_property(
                                db,
                                None::<&NoOverlay>,
                                object,
                                prop_id,
                                array_index,
                                members,
                            )
                        }) {
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
                                    Error::Remote(RemoteError::Error { class, code }) => {
                                        (*class, *code)
                                    }
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
            object_identifier: lookup_oid,
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
