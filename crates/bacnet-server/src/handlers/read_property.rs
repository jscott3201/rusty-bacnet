use super::group_present_value::GroupMembers;
use super::rpm_budget::{self, PlannedRow};
use super::*;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::PropertyReference;
use bacnet_types::error::RemoteError;

/// One single-property read (ReadProperty, ReadRange or the local read),
/// planned before any value is read: its target and row, with the member rows
/// of a Group's whole Present_Value charged to the view's work limit (#1172).
/// Planning reads no value, so the request chooses its COV snapshot from the
/// plan and reads afterwards (#1213).
pub(crate) struct PropertyPlan {
    lookup_oid: ObjectIdentifier,
    row: PlannedRow,
}

impl PropertyPlan {
    /// Plan the read of `property` on `stored`, the object the service's own
    /// lookup found at `lookup_oid`. A missing object plans nothing more; the
    /// read reports it.
    pub(super) fn new(
        db: &ObjectDatabase,
        view: Option<&DeviceReadContext<'_>>,
        lookup_oid: ObjectIdentifier,
        stored: Option<&dyn BACnetObject>,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<Self, ReadFailure> {
        let reference = PropertyReference {
            property_identifier: property,
            property_array_index: array_index,
        };
        let members = match stored {
            Some(stored) => {
                let served = view.map(|view| view.object(stored));
                let object: &dyn BACnetObject = served.as_ref().map_or(stored, |served| served);
                group_present_value::plan_property(db, view, object, &reference)?
            }
            None => None,
        };
        Ok(Self {
            lookup_oid,
            row: PlannedRow { reference, members },
        })
    }

    /// The selected Device's COV lists this read takes, by name or through
    /// the planned members of a Group.
    pub(crate) fn live_cov(&self, db: &ObjectDatabase) -> Option<LiveCovSelection> {
        rpm_budget::live_cov_selection(db, [(self.lookup_oid, &self.row)])
    }

    pub(super) fn into_members(self) -> Option<GroupMembers> {
        self.row.members
    }
}

/// Plan a ReadProperty, or a local read, of the resolved `lookup_oid`. Past
/// the view's work limit it fails with [`ReadFailure::Work`], before any
/// value is read.
pub(crate) fn plan_read_property(
    db: &ObjectDatabase,
    view: Option<&DeviceReadContext<'_>>,
    lookup_oid: ObjectIdentifier,
    property: PropertyIdentifier,
    array_index: Option<u32>,
) -> Result<PropertyPlan, ReadFailure> {
    PropertyPlan::new(
        db,
        view,
        lookup_oid,
        read_target_object(db, &lookup_oid),
        property,
        array_index,
    )
}

/// Handle a ReadProperty request against standalone object data.
///
/// Looks up the object and property in the database, encodes the value,
/// and returns the ReadPropertyACK service bytes. This low-level helper has
/// no executor context: a built-in Device follows its declared service profile,
/// including absent or empty COV lists. Running server reads use the executor's
/// property definitions and live subscription table instead.
pub fn handle_read_property(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let request = ReadPropertyRequest::decode(service_data).map_err(Error::into_request_reject)?;
    let lookup_oid = resolve_read_target(db, &request.object_identifier, None, None);
    let plan = plan_read_property(
        db,
        None,
        lookup_oid,
        request.property_identifier,
        request.property_array_index,
    )
    .map_err(ReadFailure::unlimited)?;
    read_property_request_observed(db, None, &request, plan, buf, |_, _, _| {})
}

/// ReadProperty evaluator over one decoded request and its plan. `view`
/// carries executor-owned Device definitions and the request-local COV lists
/// the plan selected; observations carry only execution outcomes, never the
/// read value. A read past the work limit failed in planning and is not
/// observed, as a ReadPropertyMultiple over its work budget is not.
pub(crate) fn read_property_request_observed(
    db: &ObjectDatabase,
    view: Option<&DeviceReadContext<'_>>,
    request: &ReadPropertyRequest,
    plan: PropertyPlan,
    buf: &mut BytesMut,
    mut completed: impl FnMut(ObjectIdentifier, &ReadPropertyRequest, &Result<(), Error>),
) -> Result<(), Error> {
    let lookup_oid = plan.lookup_oid;
    let result = read_property_decoded(db, view, request, plan, buf);
    completed(lookup_oid, request, &result);
    result
}

/// Evaluate one planned property read with ReadProperty error precedence:
/// unknown object, then non-array index using the effective property
/// definition, then the executor view or raw object's reader, except that a
/// Group's Present_Value is rebuilt from the members its plan holds.
pub(crate) fn read_property_value(
    db: &ObjectDatabase,
    view: Option<&DeviceReadContext<'_>>,
    plan: PropertyPlan,
) -> Result<PropertyValue, Error> {
    let PropertyPlan {
        lookup_oid,
        row: PlannedRow { reference, members },
    } = plan;
    let property = reference.property_identifier;
    let array_index = reference.property_array_index;
    let object = read_target_object(db, &lookup_oid).ok_or(Error::protocol(
        ErrorClass::OBJECT,
        ErrorCode::UNKNOWN_OBJECT,
    ))?;

    let served = view.map(|view| view.object(object));
    let object: &dyn BACnetObject = served.as_ref().map_or(object, |served| served);

    // Clause 15.5.1.3: an array index on a non-array property is rejected
    // with PROPERTY / PROPERTY_IS_NOT_AN_ARRAY. The array/list decision
    // belongs to the object (identifier-static whitelists cannot express the
    // type-dependent identifiers, e.g. ALARM_VALUES), so the handler defers
    // to the trait query.
    if array_index.is_some() && !object.is_array_property(property) {
        return Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ));
    }

    group_present_value::read_served_property(db, view, object, property, array_index, members)
}

fn read_property_decoded(
    db: &ObjectDatabase,
    view: Option<&DeviceReadContext<'_>>,
    request: &ReadPropertyRequest,
    plan: PropertyPlan,
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let lookup_oid = plan.lookup_oid;
    let value = read_property_value(db, view, plan)?;

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

pub(crate) fn read_target_object<'a>(
    db: &'a ObjectDatabase,
    oid: &ObjectIdentifier,
) -> Option<&'a dyn bacnet_objects::traits::BACnetObject> {
    // An unregistered wildcard is never an ordinary stored object, even if a
    // custom database supplied that reserved identifier.
    if oid.object_type() == ObjectType::NETWORK_PORT && oid.instance_number() == 4194303 {
        return None;
    }
    db.get(oid)
}

fn is_device_wildcard(oid: &ObjectIdentifier) -> bool {
    oid.object_type() == ObjectType::DEVICE && oid.instance_number() == 4194303
}

/// Resolve Device or explicitly registered receiving-port wildcard identity.
pub(crate) fn resolve_read_target(
    db: &ObjectDatabase,
    oid: &ObjectIdentifier,
    registered_port: Option<ObjectIdentifier>,
    frozen_device: Option<ObjectIdentifier>,
) -> ObjectIdentifier {
    if oid.object_type() == ObjectType::NETWORK_PORT && oid.instance_number() == 4194303 {
        return registered_port
            .filter(|selected| Some(*selected) == db.registered_port_internal())
            .unwrap_or(*oid);
    }
    if is_device_wildcard(oid) {
        // A configured executor never rebinds to another Device after mutation.
        // Returning a missing frozen OID preserves UNKNOWN_OBJECT downstream.
        if let Some(device) = frozen_device.or_else(|| db.selected_device()) {
            return device;
        }
    }
    *oid
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
    let request =
        ReadPropertyMultipleRequest::decode(service_data).map_err(Error::into_request_reject)?;

    let mut results = Vec::new();
    for spec in &request.list_of_read_access_specs {
        let mut elements = Vec::new();

        let lookup_oid = resolve_read_target(db, &spec.object_identifier, None, None);
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
                        match group_present_value::plan_property(db, None, object, &reference)
                            .map_err(ReadFailure::unlimited)
                            .and_then(|members| {
                                group_present_value::read_served_property(
                                    db,
                                    None,
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
