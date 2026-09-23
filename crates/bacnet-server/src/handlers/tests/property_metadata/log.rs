use super::*;
use bacnet_objects::{
    event_log::EventLogObject,
    traits::BACnetObject,
    trend::{TrendLogMultipleObject, TrendLogObject},
};
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetLogRecord, LogDatum};
use bacnet_types::primitives::{Date, PropertyValue, Time};
use PropertyIdentifier as P;

fn log_objects(capacity: u32, configured: bool) -> [Box<dyn BACnetObject>; 3] {
    let mut trend = TrendLogObject::new(1, "TL-1", capacity).unwrap();
    let mut multiple = TrendLogMultipleObject::new(1, "TLM-1", capacity).unwrap();
    let mut event = EventLogObject::new(1, "EL-1", capacity).unwrap();
    if configured {
        let reference = BACnetDeviceObjectPropertyReference {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap(),
            property_identifier: P::PRESENT_VALUE.to_raw(),
            property_array_index: Some(2),
            device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap()),
        };
        trend.set_log_device_object_property(Some(reference.clone()));
        multiple.add_property_reference(reference);
        trend.set_logging_type(2);
        multiple.set_logging_type(2);
        for (log_datum, status_flags) in [
            (LogDatum::UnsignedValue(42), Some(0b1010)),
            (LogDatum::LogStatus(3), None),
        ] {
            let record = BACnetLogRecord {
                date: Date {
                    year: 126,
                    month: 9,
                    day: 13,
                    day_of_week: 7,
                },
                time: Time {
                    hour: 12,
                    minute: 0,
                    second: 0,
                    hundredths: 0,
                },
                log_datum,
                status_flags,
            };
            trend.add_record(record.clone()).unwrap();
            multiple.add_record(record.clone()).unwrap();
            event.add_record(record).unwrap();
        }
    }
    let mut objects: [Box<dyn BACnetObject>; 3] =
        [Box::new(trend), Box::new(multiple), Box::new(event)];
    for object in &mut objects {
        if configured {
            object
                .write_property(
                    P::DESCRIPTION,
                    None,
                    PropertyValue::CharacterString("log description".repeat(100)),
                    None,
                )
                .unwrap();
            if object.object_identifier().object_type() != ObjectType::TREND_LOG_MULTIPLE {
                object
                    .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
                    .unwrap();
            }
        }
    }
    objects
}

// Independent (identifier, optional, writable) fixtures in legacy order.
// Kept with the log-family consumer tests so existing near-cap PICS and RPM
// test files need no unrelated splits.
fn expected_rows(kind: ObjectType) -> Vec<(P, bool, bool)> {
    let mut rows = vec![
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::LOG_ENABLE, false, true),
        (
            P::LOG_INTERVAL,
            kind != ObjectType::TREND_LOG_MULTIPLE,
            true,
        ),
        (P::STOP_WHEN_FULL, false, true),
        (P::BUFFER_SIZE, false, false),
        (P::LOG_BUFFER, false, false),
        (P::RECORD_COUNT, false, true),
        (P::TOTAL_RECORD_COUNT, false, false),
        (P::STATUS_FLAGS, false, false),
        (P::EVENT_STATE, false, false),
        (
            P::OUT_OF_SERVICE,
            true,
            kind != ObjectType::TREND_LOG_MULTIPLE,
        ),
        (P::RELIABILITY, true, false),
    ];
    if kind == ObjectType::TREND_LOG {
        rows.swap(13, 14);
    }
    if kind != ObjectType::EVENT_LOG {
        rows.extend([
            (P::LOGGING_TYPE, false, false),
            (
                P::LOG_DEVICE_OBJECT_PROPERTY,
                kind == ObjectType::TREND_LOG,
                false,
            ),
        ]);
    }
    rows.push((P::PROPERTY_LIST, false, false));
    rows
}

#[test]
fn rpm_metadata_log_selectors_pics_rows_and_budgets_are_exact() {
    for capacity in [0, 1, 3] {
        for configured in [false, true] {
            for object in log_objects(capacity, configured) {
                let oid = object.object_identifier();
                let expected = expected_rows(oid.object_type());
                let all: Vec<_> = expected
                    .iter()
                    .map(|&(p, _, _)| p)
                    .filter(|&p| p != P::PROPERTY_LIST)
                    .collect();
                let required: Vec<_> = expected
                    .iter()
                    .filter_map(|&(p, optional, _)| {
                        (!optional && p != P::PROPERTY_LIST).then_some(p)
                    })
                    .collect();
                let optional: Vec<_> = expected
                    .iter()
                    .filter_map(|&(p, optional, _)| optional.then_some(p))
                    .collect();
                let mut db = ObjectDatabase::new();
                db.add(object).unwrap();
                let pics = crate::pics::generate_pics(
                    &db,
                    &crate::server::ServerConfig::default(),
                    &crate::pics::PicsConfig::default(),
                );
                assert_eq!(pics.supported_object_types.len(), 1);
                let support = &pics.supported_object_types[0];
                assert_eq!(support.object_type, oid.object_type());
                assert!(!support.createable);
                assert!(support.deleteable);
                let rows: Vec<_> = support
                    .supported_properties
                    .iter()
                    .map(|row| {
                        assert!(row.access.readable);
                        (row.property_id, row.access.optional, row.access.writable)
                    })
                    .collect();
                assert_eq!(rows, expected);
                for (selector, expected) in [
                    (P::ALL, all.as_slice()),
                    (P::REQUIRED, required.as_slice()),
                    (P::OPTIONAL, optional.as_slice()),
                    (P::PROPERTY_LIST, &[P::PROPERTY_LIST]),
                ] {
                    assert_rpm_selector_bytes(&db, oid, selector, expected);
                }
            }
        }
    }
}

fn request(oid: ObjectIdentifier, references: &[(P, Option<u32>)]) -> BytesMut {
    let request = ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: references
                .iter()
                .map(
                    |&(property_identifier, property_array_index)| PropertyReference {
                        property_identifier,
                        property_array_index,
                    },
                )
                .collect(),
        }],
    };
    let mut bytes = BytesMut::new();
    request.encode(&mut bytes);
    bytes
}

fn assert_budget_parity(db: &ObjectDatabase, request: &[u8], legacy: &[u8], count: usize) {
    use crate::handlers::rpm_budget::{handle_rpm_budgeted, RpmFailure};
    use crate::server::ReadPropertyMultipleBudget;
    let budget = ReadPropertyMultipleBudget {
        max_result_elements: count,
        max_service_ack_bytes: legacy.len(),
    };
    let mut bounded = BytesMut::new();
    handle_rpm_budgeted(db, request, &mut bounded, budget).unwrap();
    assert_eq!(bounded.as_ref(), legacy);
    let mut prefix = BytesMut::from(&b"prefix"[..]);
    assert!(matches!(
        handle_rpm_budgeted(
            db,
            request,
            &mut prefix,
            ReadPropertyMultipleBudget {
                max_result_elements: count - 1,
                ..budget
            }
        ),
        Err(RpmFailure::Work)
    ));
    assert_eq!(&prefix[..], b"prefix");
    assert!(matches!(
        handle_rpm_budgeted(
            db,
            request,
            &mut prefix,
            ReadPropertyMultipleBudget {
                max_service_ack_bytes: legacy.len() - 1,
                ..budget
            }
        ),
        Err(RpmFailure::Bytes)
    ));
    assert_eq!(&prefix[..], b"prefix");
}

#[test]
fn rpm_log_indexed_property_list_and_list_gates_preserve_bytes() {
    for object in log_objects(3, true) {
        let oid = object.object_identifier();
        let wire: Vec<_> = expected_rows(oid.object_type())
            .iter()
            .filter_map(|&(p, _, _)| {
                (!matches!(
                    p,
                    P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE | P::PROPERTY_LIST
                ))
                .then_some(PropertyValue::Enumerated(p.to_raw()))
            })
            .collect();
        let mut references = vec![(P::PROPERTY_LIST, None)];
        references.extend((0..=wire.len() as u32 + 1).map(|index| (P::PROPERTY_LIST, Some(index))));
        references.extend([
            (P::LOG_BUFFER, Some(0)),
            (P::LOG_BUFFER, Some(1)),
            (P::LOG_BUFFER, Some(u32::MAX)),
            (P::RECORD_COUNT, Some(0)),
            (P::TOTAL_RECORD_COUNT, Some(1)),
        ]);
        let bytes = request(oid, &references);
        let mut db = ObjectDatabase::new();
        db.add(object).unwrap();
        let mut legacy = BytesMut::new();
        handle_read_property_multiple(&db, &bytes, &mut legacy).unwrap();
        let ack = ReadPropertyMultipleACK::decode(&legacy).unwrap();
        let results = &ack.list_of_read_access_results[0].list_of_results;
        assert_eq!(results.len(), references.len());
        for (result, &(p, index)) in results.iter().zip(&references) {
            assert_eq!(result.property_identifier, p);
            assert_eq!(result.property_array_index, index);
            let expected = match (p, index) {
                (P::PROPERTY_LIST, None) => Ok(PropertyValue::List(wire.clone())),
                (P::PROPERTY_LIST, Some(0)) => Ok(PropertyValue::Unsigned(wire.len() as u64)),
                (P::PROPERTY_LIST, Some(i)) if i as usize <= wire.len() => {
                    Ok(wire[i as usize - 1].clone())
                }
                (P::PROPERTY_LIST, _) => Err(ErrorCode::INVALID_ARRAY_INDEX),
                _ => Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            };
            let rp = ReadPropertyRequest {
                object_identifier: oid,
                property_identifier: p,
                property_array_index: index,
            };
            let mut rp_bytes = BytesMut::new();
            rp.encode(&mut rp_bytes);
            let mut rp_ack = BytesMut::new();
            let rp_result = handle_read_property(&db, &rp_bytes, &mut rp_ack);
            match expected {
                Ok(value) => {
                    rp_result.unwrap();
                    let mut expected_bytes = BytesMut::new();
                    bacnet_encoding::primitives::encode_property_value(&mut expected_bytes, &value)
                        .unwrap();
                    assert!(result.error.is_none());
                    assert_eq!(
                        result.property_value.as_deref(),
                        Some(expected_bytes.as_ref())
                    );
                    assert_eq!(
                        ReadPropertyACK::decode(&rp_ack).unwrap().property_value,
                        expected_bytes
                    );
                }
                Err(code) => {
                    assert_eq!(result.error, Some((ErrorClass::PROPERTY, code)));
                    assert!(result.property_value.is_none());
                    assert!(matches!(rp_result, Err(Error::Protocol { class, code: c })
                        if class == ErrorClass::PROPERTY.to_raw() as u32 && c == code.to_raw() as u32));
                }
            }
        }
        assert_budget_parity(&db, &bytes, &legacy, references.len());
    }
}

#[test]
fn rpm_log_buffer_profile_bytes_match_rp_without_sequence_identity() {
    for object in log_objects(3, true) {
        let oid = object.object_identifier();
        // Independent legacy application-value bytes: Date, Time, Unsigned;
        // only single-channel Trend appends the supplied StatusFlags. The next
        // resident record is Date, Time, LogStatus. Neither contains a sequence.
        let mut expected = vec![0xa4, 126, 9, 13, 7, 0xb4, 12, 0, 0, 0, 0x21, 42];
        if oid.object_type() == ObjectType::TREND_LOG {
            expected.extend([0x82, 4, 0xa0]);
        }
        expected.extend([0xa4, 126, 9, 13, 7, 0xb4, 12, 0, 0, 0, 0x82, 5, 0x60]);
        let identities = object.log_record_identities_internal().unwrap();
        assert_eq!(
            identities
                .iter()
                .map(|id| id.sequence_number())
                .collect::<Vec<_>>(),
            [1, 2]
        );
        let mut db = ObjectDatabase::new();
        db.add(object).unwrap();
        let references = [
            (P::LOG_BUFFER, None),
            (P::RECORD_COUNT, None),
            (P::TOTAL_RECORD_COUNT, None),
        ];
        let bytes = request(oid, &references);
        let mut legacy = BytesMut::new();
        handle_read_property_multiple(&db, &bytes, &mut legacy).unwrap();
        let ack = ReadPropertyMultipleACK::decode(&legacy).unwrap();
        let results = &ack.list_of_read_access_results[0].list_of_results;
        assert_eq!(results.len(), 3);
        for (result, &(p, _)) in results.iter().zip(&references) {
            assert_eq!(result.property_identifier, p);
            assert!(result.error.is_none());
            let value = if p == P::LOG_BUFFER {
                expected.as_slice()
            } else {
                &[0x21, 2]
            };
            assert_eq!(result.property_value.as_deref(), Some(value));
            let rp = ReadPropertyRequest {
                object_identifier: oid,
                property_identifier: p,
                property_array_index: None,
            };
            let mut rp_bytes = BytesMut::new();
            rp.encode(&mut rp_bytes);
            let mut response = BytesMut::new();
            handle_read_property(&db, &rp_bytes, &mut response).unwrap();
            assert_eq!(
                ReadPropertyACK::decode(&response).unwrap().property_value,
                value
            );
        }
        assert_budget_parity(&db, &bytes, &legacy, references.len());
    }
}
