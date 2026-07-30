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

    object
        .write_property(
            request.property_identifier,
            request.property_array_index,
            PropertyValue::List(items),
            None,
        )
        .map_err(|err| match err {
            // Clause 15.1 gives AddListElement its own resource error; the
            // object arm only knows WriteProperty's.
            Error::Protocol { class, code }
                if class == ErrorClass::RESOURCES.to_raw() as u32
                    && code == ErrorCode::NO_SPACE_TO_WRITE_PROPERTY.to_raw() as u32 =>
            {
                Error::Protocol {
                    class,
                    code: ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32,
                }
            }
            other => other,
        })?;

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

    fn request(oid: ObjectIdentifier, element: u8, array_index: Option<u32>) -> BytesMut {
        let mut encoded = BytesMut::new();
        ListElementRequest {
            object_identifier: oid,
            property_identifier: PropertyIdentifier::ALARM_VALUES,
            property_array_index: array_index,
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

        handle_add_list_element(&mut db, &request(oid, 2, None)).unwrap();
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::ALARM_VALUES, None)
                .unwrap(),
            PropertyValue::List(vec![PropertyValue::Unsigned(2)])
        );

        handle_remove_list_element(&mut db, &request(oid, 2, None)).unwrap();
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::ALARM_VALUES, None)
                .unwrap(),
            PropertyValue::List(vec![])
        );
    }

    #[test]
    fn add_list_element_over_cap_returns_the_clause_15_1_error() {
        let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 1).unwrap();
        let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
        // Fill to MAX_ALARM_VALUES (1024) so the appended element trips the cap.
        msi.set_alarm_values((0..1024).collect());
        let mut db = ObjectDatabase::new();
        db.add(Box::new(msi)).unwrap();

        let err = handle_add_list_element(&mut db, &request(oid, 7, None)).unwrap_err();
        match err {
            Error::Protocol { class, code } => {
                assert_eq!(class, ErrorClass::RESOURCES.to_raw() as u32);
                // Clause 15.1 names AddListElement's own error, not
                // WriteProperty's NO_SPACE_TO_WRITE_PROPERTY.
                assert_eq!(
                    code,
                    ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32
                );
            }
            other => panic!("expected Protocol error, got {other:?}"),
        }
        // The list must be unchanged.
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::ALARM_VALUES, None)
                .unwrap(),
            PropertyValue::List((0..1024).map(PropertyValue::Unsigned).collect())
        );
    }

    #[test]
    fn add_list_element_rejects_array_index_on_alarm_values() {
        let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 1).unwrap();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(MultiStateInputObject::new(1, "MSI-1", 3).unwrap()))
            .unwrap();

        match handle_add_list_element(&mut db, &request(oid, 2, Some(1))).unwrap_err() {
            Error::Protocol { class, code } => {
                assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
                assert_eq!(code, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY.to_raw() as u32);
            }
            other => panic!("expected PROPERTY_IS_NOT_AN_ARRAY, got {other:?}"),
        }
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::ALARM_VALUES, None)
                .unwrap(),
            PropertyValue::List(vec![])
        );
    }

    #[test]
    fn whole_list_write_property_pins_decoder_gap() {
        let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 1).unwrap();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(MultiStateInputObject::new(1, "MSI-1", 3).unwrap()))
            .unwrap();
        let request = WritePropertyRequest {
            object_identifier: oid,
            property_identifier: PropertyIdentifier::ALARM_VALUES,
            property_array_index: None,
            // A BACnetLIST is consecutive application-tagged elements.
            property_value: vec![0x21, 2, 0x21, 3],
            priority: None,
        };
        let mut encoded = BytesMut::new();
        request.encode(&mut encoded);

        // #182: WriteProperty currently decodes exactly one primitive, so this
        // is INVALID_DATA_TYPE. Flip this expectation when LIST decoding lands.
        match handle_write_property(&mut db, &encoded).unwrap_err() {
            Error::Protocol { class, code } => {
                assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
                assert_eq!(code, ErrorCode::INVALID_DATA_TYPE.to_raw() as u32);
            }
            other => panic!("expected INVALID_DATA_TYPE, got {other:?}"),
        }
    }
}
