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
