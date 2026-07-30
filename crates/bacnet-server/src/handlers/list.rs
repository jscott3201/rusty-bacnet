use super::*;

/// Handle an AddListElement request.
///
/// Reads the target property, appends the new elements, and writes back.
pub fn handle_add_list_element(db: &mut ObjectDatabase, service_data: &[u8]) -> Result<(), Error> {
    use bacnet_encoding::primitives::decode_application_value;
    use bacnet_services::list_manipulation::ListElementRequest;

    let request = ListElementRequest::decode(service_data)?;

    let object = db
        .get_mut(&request.object_identifier)
        .ok_or(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
        })?;

    let current =
        object.read_property(request.property_identifier, request.property_array_index)?;
    let mut items = match current {
        PropertyValue::List(items) => items,
        _ => Vec::new(),
    };

    let mut offset = 0;
    let data = &request.list_of_elements;
    while offset < data.len() {
        match decode_application_value(data, offset) {
            Ok((val, new_offset)) => {
                items.push(val);
                offset = new_offset;
            }
            Err(_) => break,
        }
    }

    object.write_property(
        request.property_identifier,
        request.property_array_index,
        PropertyValue::List(items),
        None,
    )?;

    Ok(())
}

/// Handle a RemoveListElement request.
///
/// Reads the target property, removes matching elements, and writes back.
pub fn handle_remove_list_element(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<(), Error> {
    use bacnet_encoding::primitives::decode_application_value;
    use bacnet_services::list_manipulation::ListElementRequest;

    let request = ListElementRequest::decode(service_data)?;

    let object = db
        .get_mut(&request.object_identifier)
        .ok_or(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
        })?;

    let current =
        object.read_property(request.property_identifier, request.property_array_index)?;
    let mut items = match current {
        PropertyValue::List(items) => items,
        _ => Vec::new(),
    };

    let mut to_remove = Vec::new();
    let mut offset = 0;
    let data = &request.list_of_elements;
    while offset < data.len() {
        match decode_application_value(data, offset) {
            Ok((val, new_offset)) => {
                to_remove.push(val);
                offset = new_offset;
            }
            Err(_) => break,
        }
    }

    // Remove matching elements
    items.retain(|item| !to_remove.contains(item));

    object.write_property(
        request.property_identifier,
        request.property_array_index,
        PropertyValue::List(items),
        None,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_objects::multistate::MultiStateInputObject;
    use bacnet_services::list_manipulation::ListElementRequest;
    use bacnet_types::enums::ObjectType;
    use bytes::BytesMut;

    fn request(oid: ObjectIdentifier, element: u8) -> BytesMut {
        let mut encoded = BytesMut::new();
        ListElementRequest {
            object_identifier: oid,
            property_identifier: PropertyIdentifier::ALARM_VALUES,
            property_array_index: None,
            list_of_elements: vec![0x21, element],
        }
        .encode(&mut encoded);
        encoded
    }

    #[test]
    fn add_and_remove_list_element_mutate_msi_alarm_values() {
        let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 1).unwrap();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(MultiStateInputObject::new(1, "MSI-1", 3).unwrap()))
            .unwrap();

        handle_add_list_element(&mut db, &request(oid, 2)).unwrap();
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::ALARM_VALUES, None)
                .unwrap(),
            PropertyValue::List(vec![PropertyValue::Unsigned(2)])
        );

        handle_remove_list_element(&mut db, &request(oid, 2)).unwrap();
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::ALARM_VALUES, None)
                .unwrap(),
            PropertyValue::List(vec![])
        );
    }
}
