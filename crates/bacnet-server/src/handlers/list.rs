use super::*;
use bacnet_encoding::{constructed::decode_destination_list, primitives::decode_application_value};
use bacnet_services::list_manipulation::ListElementRequest;
use bacnet_types::constructed::BACnetDestination;

fn invalid_data_type() -> Error {
    Error::Protocol {
        class: ErrorClass::PROPERTY.to_raw() as u32,
        code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
    }
}

/// Handle an AddListElement request.
///
/// Reads the target property, appends the new elements, and writes back.
pub fn handle_add_list_element(db: &mut ObjectDatabase, service_data: &[u8]) -> Result<(), Error> {
    handle_list_element_observed(db, service_data, false, |_, _, _| {})
}

/// Handle a RemoveListElement request.
///
/// Reads the target property, removes matching elements, and writes back.
pub fn handle_remove_list_element(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<(), Error> {
    handle_list_element_observed(db, service_data, true, |_, _, _| {})
}

enum ListEdits {
    Values(Vec<PropertyValue>),
    Destinations(Vec<BACnetDestination>),
}

/// Decode once and observe only executable requests. The callback borrows the
/// pre-image already needed by execution; it never causes a second property read.
/// Lookup/read errors keep their original precedence over element decode errors.
pub(crate) fn handle_list_element_observed(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    remove: bool,
    mut before: impl FnMut(&ObjectDatabase, &ListElementRequest, Option<&PropertyValue>),
) -> Result<(), Error> {
    let request = ListElementRequest::decode(service_data)?;

    let current = db
        .get(&request.object_identifier)
        .ok_or(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
        })
        .and_then(|object| {
            object.read_property(request.property_identifier, request.property_array_index)
        });
    // A missing/unreadable NotificationClass Recipient_List still has a known
    // framed datatype. For readable properties, retain the existing shape-based
    // execution choice, including the strict handling of ApplicationData.
    let framed = matches!(&current, Ok(PropertyValue::ApplicationData(_)))
        || (current.is_err()
            && request.object_identifier.object_type() == ObjectType::NOTIFICATION_CLASS
            && request.property_identifier == PropertyIdentifier::RECIPIENT_LIST);
    let edits = if framed {
        decode_destination_list(&request.list_of_elements).map(ListEdits::Destinations)
    } else {
        let mut values = Vec::new();
        let mut offset = 0;
        let decode = || -> Result<Vec<PropertyValue>, Error> {
            while offset < request.list_of_elements.len() {
                let (value, next) = decode_application_value(&request.list_of_elements, offset)?;
                if current.is_ok() {
                    values.push(value);
                }
                offset = next;
            }
            Ok(values)
        };
        decode().map(ListEdits::Values)
    };
    let current = match current {
        Ok(current) => current,
        Err(error) => {
            if edits.is_ok() {
                before(db, &request, None);
            }
            return Err(error);
        }
    };
    let edits = edits.map_err(|_| invalid_data_type())?;
    let value = match edits {
        ListEdits::Destinations(edits) => {
            let PropertyValue::ApplicationData(bytes) = &current else {
                unreachable!()
            };
            // Decode BOTH lists before observation or mutation. A malformed
            // stored frame must not be treated as an empty list on removal.
            let mut destinations =
                decode_destination_list(bytes).map_err(|_| invalid_data_type())?;
            before(db, &request, Some(&current));
            if remove {
                destinations.retain(|d| !edits.contains(d));
            } else {
                destinations.extend(edits);
            }
            let mut bytes = BytesMut::new();
            bacnet_encoding::constructed::encode_destination_list(&mut bytes, &destinations);
            PropertyValue::ApplicationData(bytes.to_vec())
        }
        ListEdits::Values(edits) => {
            before(db, &request, Some(&current));
            let mut items = match current {
                PropertyValue::List(items) => items,
                _ => Vec::new(),
            };
            if remove {
                items.retain(|item| !edits.contains(item));
            } else {
                items.extend(edits);
            }
            PropertyValue::List(items)
        }
    };
    db.get_mut(&request.object_identifier)
        .expect("readable object")
        .write_property(
            request.property_identifier,
            request.property_array_index,
            value,
            None,
        )
        .map_err(|error| match error {
            // Preserve AddListElement's existing Clause 15.1 resource mapping.
            Error::Protocol { class, code }
                if !remove
                    && class == ErrorClass::RESOURCES.to_raw() as u32
                    && code == ErrorCode::NO_SPACE_TO_WRITE_PROPERTY.to_raw() as u32 =>
            {
                Error::Protocol {
                    class,
                    code: ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32,
                }
            }
            other => other,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_objects::multistate::MultiStateInputObject;
    use bacnet_objects::traits::BACnetObject;
    use bacnet_services::list_manipulation::ListElementRequest;
    use bacnet_types::enums::ObjectType;
    use bytes::BytesMut;

    fn request(oid: ObjectIdentifier, element: u8, array_index: Option<u32>) -> BytesMut {
        request_with_elements(oid, vec![0x21, element], array_index)
    }

    fn request_with_elements(
        oid: ObjectIdentifier,
        list_of_elements: Vec<u8>,
        array_index: Option<u32>,
    ) -> BytesMut {
        let mut encoded = BytesMut::new();
        ListElementRequest {
            object_identifier: oid,
            property_identifier: PropertyIdentifier::ALARM_VALUES,
            property_array_index: array_index,
            list_of_elements,
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
    fn add_list_element_malformed_tail_errors_without_partial_commit() {
        let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 1).unwrap();
        let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
        msi.set_alarm_values(vec![1]);
        let mut db = ObjectDatabase::new();
        db.add(Box::new(msi)).unwrap();

        let encoded = request_with_elements(oid, vec![0x21, 2, 0xD1, 0], None);
        match handle_add_list_element(&mut db, &encoded).unwrap_err() {
            Error::Protocol { class, code } => {
                assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
                assert_eq!(code, ErrorCode::INVALID_DATA_TYPE.to_raw() as u32);
            }
            other => panic!("expected PROPERTY/INVALID_DATA_TYPE, got {other:?}"),
        }
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::ALARM_VALUES, None)
                .unwrap(),
            PropertyValue::List(vec![PropertyValue::Unsigned(1)])
        );
    }

    #[test]
    fn remove_list_element_malformed_tail_errors_without_partial_commit() {
        let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 1).unwrap();
        let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
        msi.set_alarm_values(vec![2, 3]);
        let mut db = ObjectDatabase::new();
        db.add(Box::new(msi)).unwrap();

        let encoded = request_with_elements(oid, vec![0x21, 2, 0xD1, 0], None);
        match handle_remove_list_element(&mut db, &encoded).unwrap_err() {
            Error::Protocol { class, code } => {
                assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
                assert_eq!(code, ErrorCode::INVALID_DATA_TYPE.to_raw() as u32);
            }
            other => panic!("expected PROPERTY/INVALID_DATA_TYPE, got {other:?}"),
        }
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::ALARM_VALUES, None)
                .unwrap(),
            PropertyValue::List(vec![PropertyValue::Unsigned(2), PropertyValue::Unsigned(3)])
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

    // ---- Framed Recipient_List element editing (#152 review) ----

    use bacnet_objects::notification_class::NotificationClass;
    use bacnet_types::constructed::{BACnetDestination, BACnetRecipient};
    use bacnet_types::primitives::Time;

    fn framed_dest(device_instance: u32) -> BACnetDestination {
        let t = |h, m| Time {
            hour: h,
            minute: m,
            second: 0,
            hundredths: 0,
        };
        BACnetDestination {
            valid_days: 0b0111_1111,
            from_time: t(0, 0),
            to_time: t(23, 59),
            recipient: BACnetRecipient::Device(
                ObjectIdentifier::new(ObjectType::DEVICE, device_instance).unwrap(),
            ),
            process_identifier: device_instance,
            issue_confirmed_notifications: false,
            transitions: 0b0000_0111,
        }
    }

    fn framed_bytes(destinations: &[BACnetDestination]) -> Vec<u8> {
        let mut buf = BytesMut::new();
        bacnet_encoding::constructed::encode_destination_list(&mut buf, destinations);
        buf.to_vec()
    }

    fn nc_db(entries: &[BACnetDestination]) -> (ObjectDatabase, ObjectIdentifier) {
        let mut db = ObjectDatabase::new();
        let mut nc = NotificationClass::new(1, "NC-1").unwrap();
        for d in entries {
            nc.add_destination(d.clone());
        }
        let oid = nc.object_identifier();
        db.add(Box::new(nc)).unwrap();
        (db, oid)
    }

    fn recipient_list_wire_bytes(db: &ObjectDatabase, oid: ObjectIdentifier) -> Vec<u8> {
        let v = db
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
            .unwrap();
        let PropertyValue::ApplicationData(bytes) = v else {
            panic!("expected ApplicationData");
        };
        bytes
    }

    fn device_instances(db: &ObjectDatabase, oid: ObjectIdentifier) -> Vec<u32> {
        let bytes = recipient_list_wire_bytes(db, oid);
        bacnet_encoding::constructed::decode_destination_list(&bytes)
            .unwrap()
            .iter()
            .map(|d| match &d.recipient {
                BACnetRecipient::Device(o) => o.instance_number(),
                other => panic!("expected Device recipient, got {other:?}"),
            })
            .collect()
    }

    #[test]
    fn remove_list_element_from_framed_recipient_list_leaves_rest() {
        let (mut db, oid) = nc_db(&[framed_dest(10), framed_dest(20), framed_dest(30)]);
        let request = ListElementRequest {
            object_identifier: oid,
            property_identifier: PropertyIdentifier::RECIPIENT_LIST,
            property_array_index: None,
            list_of_elements: framed_bytes(&[framed_dest(20)]),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);
        handle_remove_list_element(&mut db, &buf).unwrap();
        assert_eq!(device_instances(&db, oid), vec![10, 30]);
        // The wire form re-encodes as exactly the two remaining destinations.
        assert_eq!(
            recipient_list_wire_bytes(&db, oid),
            framed_bytes(&[framed_dest(10), framed_dest(30)])
        );
    }

    #[test]
    fn remove_list_element_non_matching_entry_is_noop() {
        let (mut db, oid) = nc_db(&[framed_dest(10), framed_dest(20)]);
        let before = recipient_list_wire_bytes(&db, oid);
        let request = ListElementRequest {
            object_identifier: oid,
            property_identifier: PropertyIdentifier::RECIPIENT_LIST,
            property_array_index: None,
            list_of_elements: framed_bytes(&[framed_dest(99)]),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);
        handle_remove_list_element(&mut db, &buf).unwrap();
        assert_eq!(device_instances(&db, oid), vec![10, 20]);
        assert_eq!(
            recipient_list_wire_bytes(&db, oid),
            before,
            "bytes unchanged"
        );
    }

    #[test]
    fn add_list_element_to_framed_recipient_list_appends() {
        let (mut db, oid) = nc_db(&[framed_dest(10)]);
        let request = ListElementRequest {
            object_identifier: oid,
            property_identifier: PropertyIdentifier::RECIPIENT_LIST,
            property_array_index: None,
            list_of_elements: framed_bytes(&[framed_dest(20), framed_dest(30)]),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);
        handle_add_list_element(&mut db, &buf).unwrap();
        assert_eq!(device_instances(&db, oid), vec![10, 20, 30]);
        assert_eq!(
            recipient_list_wire_bytes(&db, oid),
            framed_bytes(&[framed_dest(10), framed_dest(20), framed_dest(30)])
        );
    }

    #[test]
    fn remove_list_element_malformed_framed_payload_errors_and_preserves() {
        let (mut db, oid) = nc_db(&[framed_dest(10), framed_dest(20)]);
        let before = recipient_list_wire_bytes(&db, oid);
        // Well-formed TLV, but NOT a BACnetDestination (a bare application
        // Unsigned where the destination's valid-days bit string belongs).
        let request = ListElementRequest {
            object_identifier: oid,
            property_identifier: PropertyIdentifier::RECIPIENT_LIST,
            property_array_index: None,
            list_of_elements: vec![0x21, 0x2A],
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);
        let err = handle_remove_list_element(&mut db, &buf).unwrap_err();
        match err {
            Error::Protocol { class, code } => {
                assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
                assert_eq!(code, ErrorCode::INVALID_DATA_TYPE.to_raw() as u32);
            }
            other => panic!("expected PROPERTY/INVALID_DATA_TYPE, got {other:?}"),
        }
        assert_eq!(
            recipient_list_wire_bytes(&db, oid),
            before,
            "no silent wipe"
        );
    }

    #[test]
    fn whole_list_write_property_decodes_all_elements() {
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
        request.encode(&mut encoded).unwrap();

        // #182: WriteProperty loop-decodes the whole payload, so the
        // whole-list write lands with per-element validation in the arm.
        handle_write_property(&mut db, &encoded).unwrap();
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::ALARM_VALUES, None)
                .unwrap(),
            PropertyValue::List(vec![PropertyValue::Unsigned(2), PropertyValue::Unsigned(3)])
        );

        // Per-element validation still applies: one non-Unsigned member
        // refuses the whole write and leaves the list untouched.
        let request = WritePropertyRequest {
            object_identifier: oid,
            property_identifier: PropertyIdentifier::ALARM_VALUES,
            property_array_index: None,
            property_value: vec![0x21, 4, 0x11], // Unsigned 4, Boolean true
            priority: None,
        };
        let mut encoded = BytesMut::new();
        request.encode(&mut encoded).unwrap();
        match handle_write_property(&mut db, &encoded).unwrap_err() {
            Error::Protocol { class, code } => {
                assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
                assert_eq!(code, ErrorCode::INVALID_DATA_TYPE.to_raw() as u32);
            }
            other => panic!("expected INVALID_DATA_TYPE, got {other:?}"),
        }
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::ALARM_VALUES, None)
                .unwrap(),
            PropertyValue::List(vec![PropertyValue::Unsigned(2), PropertyValue::Unsigned(3)]),
            "refused write leaves the list unchanged"
        );
    }
}
