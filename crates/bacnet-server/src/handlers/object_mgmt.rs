use super::*;

/// Handle a WhoHas request and return an IHave response if we have the object.
///
/// Returns `Some(IHaveRequest)` if we have the requested object, `None` otherwise.
pub fn handle_who_has(
    db: &ObjectDatabase,
    service_data: &[u8],
    device_oid: ObjectIdentifier,
) -> Result<Option<IHaveRequest>, Error> {
    let request = WhoHasRequest::decode(service_data)?;

    let instance = device_oid.instance_number();
    if let (Some(low), Some(high)) = (request.low_limit, request.high_limit) {
        if instance < low || instance > high {
            return Ok(None);
        }
    }

    match &request.object {
        WhoHasObject::Identifier(oid) => {
            if let Some(obj) = db.get(oid) {
                return Ok(Some(IHaveRequest {
                    device_identifier: device_oid,
                    object_identifier: *oid,
                    object_name: obj.object_name().to_string(),
                }));
            }
        }
        WhoHasObject::Name(name) => {
            for (oid, obj) in db.iter_objects() {
                if obj.object_name() == name {
                    return Ok(Some(IHaveRequest {
                        device_identifier: device_oid,
                        object_identifier: oid,
                        object_name: name.clone(),
                    }));
                }
            }
        }
    }

    Ok(None)
}

/// Handle a CreateObject request.
///
/// Supports creating objects by type (server picks instance) or by identifier.
/// Returns the encoded ObjectIdentifier of the created object (ComplexAck payload).
pub fn handle_create_object(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
) -> Result<(), Error> {
    let request = CreateObjectRequest::decode(service_data)?;

    const MAX_OBJECTS: usize = 10_000;
    if db.len() >= MAX_OBJECTS {
        return Err(Error::Protocol {
            class: ErrorClass::RESOURCES.to_raw() as u32,
            code: ErrorCode::NO_SPACE_FOR_OBJECT.to_raw() as u32,
        });
    }

    let (object_type, instance) = match &request.object_specifier {
        ObjectSpecifier::Type(obj_type) => {
            let existing: HashSet<u32> = db
                .find_by_type(*obj_type)
                .iter()
                .map(|oid| oid.instance_number())
                .collect();
            let next = (1u32..=4_194_303)
                .find(|i| !existing.contains(i))
                .ok_or_else(|| Error::Protocol {
                    class: ErrorClass::RESOURCES.to_raw() as u32,
                    code: ErrorCode::NO_SPACE_FOR_OBJECT.to_raw() as u32,
                })?;
            (*obj_type, next)
        }
        ObjectSpecifier::Identifier(oid) => {
            if db.get(oid).is_some() {
                return Err(Error::Protocol {
                    class: ErrorClass::OBJECT.to_raw() as u32,
                    code: ErrorCode::OBJECT_IDENTIFIER_ALREADY_EXISTS.to_raw() as u32,
                });
            }
            (oid.object_type(), oid.instance_number())
        }
    };

    let name = format!("{:?}-{}", object_type, instance);

    let object: Box<dyn bacnet_objects::traits::BACnetObject> =
        if object_type == ObjectType::ANALOG_INPUT {
            Box::new(bacnet_objects::analog::AnalogInputObject::new(
                instance, &name, 95,
            )?)
        } else if object_type == ObjectType::ANALOG_OUTPUT {
            Box::new(bacnet_objects::analog::AnalogOutputObject::new(
                instance, &name, 95,
            )?)
        } else if object_type == ObjectType::BINARY_INPUT {
            Box::new(bacnet_objects::binary::BinaryInputObject::new(
                instance, &name,
            )?)
        } else if object_type == ObjectType::BINARY_OUTPUT {
            Box::new(bacnet_objects::binary::BinaryOutputObject::new(
                instance, &name,
            )?)
        } else if object_type == ObjectType::BINARY_VALUE {
            Box::new(bacnet_objects::binary::BinaryValueObject::new(
                instance, &name,
            )?)
        } else if object_type == ObjectType::MULTI_STATE_INPUT {
            Box::new(bacnet_objects::multistate::MultiStateInputObject::new(
                instance, &name, 2,
            )?)
        } else if object_type == ObjectType::MULTI_STATE_OUTPUT {
            Box::new(bacnet_objects::multistate::MultiStateOutputObject::new(
                instance, &name, 2,
            )?)
        } else if object_type == ObjectType::MULTI_STATE_VALUE {
            Box::new(bacnet_objects::multistate::MultiStateValueObject::new(
                instance, &name, 2,
            )?)
        } else {
            return Err(Error::Protocol {
                class: ErrorClass::OBJECT.to_raw() as u32,
                code: ErrorCode::UNSUPPORTED_OBJECT_TYPE.to_raw() as u32,
            });
        };

    let created_oid = object.object_identifier();
    db.add(object)?;

    // Apply initial values; on failure, remove the created object.
    for pv in &request.list_of_initial_values {
        let (value, _) = match bacnet_encoding::primitives::decode_application_value(&pv.value, 0) {
            Ok(v) => v,
            Err(e) => {
                db.remove(&created_oid);
                return Err(e);
            }
        };
        if let Some(obj) = db.get_mut(&created_oid) {
            if let Err(e) = obj.write_property(
                pv.property_identifier,
                pv.property_array_index,
                value,
                pv.priority,
            ) {
                db.remove(&created_oid);
                return Err(e);
            }
        }
    }

    bacnet_encoding::primitives::encode_app_object_id(buf, &created_oid);
    Ok(())
}

/// Handle a DeleteObject request.
///
/// Removes the object from the database. Returns an error if the object
/// doesn't exist or is the Device object (which cannot be deleted).
pub fn handle_delete_object(db: &mut ObjectDatabase, service_data: &[u8]) -> Result<(), Error> {
    let request = DeleteObjectRequest::decode(service_data)?;

    if request.object_identifier.object_type() == ObjectType::DEVICE {
        return Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OBJECT_DELETION_NOT_PERMITTED.to_raw() as u32,
        });
    }

    db.remove(&request.object_identifier)
        .ok_or(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
        })?;

    Ok(())
}
