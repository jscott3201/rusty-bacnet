use super::*;

/// Handle a ReadProperty request.
///
/// Looks up the object and property in the database, encodes the value,
/// and returns the ReadPropertyACK service bytes.
pub fn handle_read_property(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let request = ReadPropertyRequest::decode(service_data)?;

    let lookup_oid = resolve_device_wildcard(db, &request.object_identifier);

    let object = db.get(&lookup_oid).ok_or(Error::Protocol {
        class: ErrorClass::OBJECT.to_raw() as u32,
        code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
    })?;

    if request.property_array_index.is_some() {
        let is_array_property = matches!(
            request.property_identifier,
            p if p == PropertyIdentifier::PRIORITY_ARRAY
                || p == PropertyIdentifier::OBJECT_LIST
                || p == PropertyIdentifier::PROPERTY_LIST
                || p == PropertyIdentifier::WEEKLY_SCHEDULE
                || p == PropertyIdentifier::EXCEPTION_SCHEDULE
                || p == PropertyIdentifier::DATE_LIST
                || p == PropertyIdentifier::LIST_OF_GROUP_MEMBERS
                || p == PropertyIdentifier::RECIPIENT_LIST
                || p == PropertyIdentifier::LOG_BUFFER
                || p == PropertyIdentifier::STATE_TEXT
                || p == PropertyIdentifier::ALARM_VALUES
                || p == PropertyIdentifier::FAULT_VALUES
                || p == PropertyIdentifier::EVENT_TIME_STAMPS
                || p == PropertyIdentifier::EVENT_MESSAGE_TEXTS
                || p == PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES
                || p == PropertyIdentifier::DEVICE_ADDRESS_BINDING
                || p == PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS
                || p == PropertyIdentifier::TAGS
        );
        if !is_array_property {
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::PROPERTY_IS_NOT_AN_ARRAY.to_raw() as u32,
            });
        }
    }

    let value = object.read_property(request.property_identifier, request.property_array_index)?;

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

/// Resolve Device wildcard instance 4194303 to the actual Device object.
fn resolve_device_wildcard(db: &ObjectDatabase, oid: &ObjectIdentifier) -> ObjectIdentifier {
    if oid.object_type() == ObjectType::DEVICE && oid.instance_number() == 4194303 {
        for candidate in db.list_objects() {
            if candidate.object_type() == ObjectType::DEVICE {
                return candidate;
            }
        }
    }
    *oid
}

/// Handle a ReadPropertyMultiple request.
///
/// Per-property errors are returned inline rather than failing the entire request.
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
                    let prop_ids: Vec<PropertyIdentifier> = match prop_ref.property_identifier {
                        PropertyIdentifier::ALL => object.property_list().to_vec(),
                        PropertyIdentifier::REQUIRED => object.required_properties().to_vec(),
                        PropertyIdentifier::OPTIONAL => {
                            let required: std::collections::HashSet<PropertyIdentifier> =
                                object.required_properties().iter().copied().collect();
                            object
                                .property_list()
                                .iter()
                                .copied()
                                .filter(|p| !required.contains(p))
                                .collect()
                        }
                        other => vec![other],
                    };

                    for prop_id in prop_ids {
                        let array_index = if prop_ref.property_identifier == prop_id {
                            prop_ref.property_array_index
                        } else {
                            None
                        };
                        match object.read_property(prop_id, array_index) {
                            Ok(value) => {
                                let mut value_buf = BytesMut::new();
                                match encode_property_value(&mut value_buf, &value) {
                                    Ok(()) => {
                                        elements.push(ReadResultElement {
                                            property_identifier: prop_id,
                                            property_array_index: array_index,
                                            property_value: Some(value_buf.to_vec()),
                                            error: None,
                                        });
                                    }
                                    Err(_) => {
                                        elements.push(ReadResultElement {
                                            property_identifier: prop_id,
                                            property_array_index: array_index,
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
                                    property_array_index: array_index,
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
                        property_array_index: prop_ref.property_array_index,
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
