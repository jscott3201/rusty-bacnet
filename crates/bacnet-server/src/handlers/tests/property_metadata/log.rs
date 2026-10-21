use super::*;
use bacnet_objects::{
    event_log::EventLogObject,
    traits::BACnetObject,
    trend::{TrendLogMultipleObject, TrendLogObject},
};
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventLogRecord, BACnetLogMultipleRecord,
    BACnetLogRecord, EventLogDatum, LogData, LogDatum, LogValue,
};
use bacnet_types::enums::LoggingType;
use bacnet_types::primitives::{Date, PropertyValue, StatusFlags, Time};
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
        trend
            .set_log_device_object_property(Some(reference.clone()))
            .unwrap();
        multiple.add_property_reference(reference).unwrap();
        trend.set_logging_type(LoggingType::TRIGGERED).unwrap();
        multiple.set_logging_type(LoggingType::TRIGGERED).unwrap();
        for (log_datum, status_flags, log_data, event_datum) in [
            (
                LogDatum::UnsignedValue(42),
                Some(StatusFlags::IN_ALARM | StatusFlags::OVERRIDDEN),
                LogData::Values(vec![LogValue::UnsignedValue(42)]),
                EventLogDatum::TimeChange(42.0),
            ),
            (
                LogDatum::LogStatus(LogStatus::LOG_DISABLED | LogStatus::BUFFER_PURGED),
                None,
                LogData::LogStatus(LogStatus::LOG_DISABLED | LogStatus::BUFFER_PURGED),
                EventLogDatum::LogStatus(LogStatus::LOG_DISABLED | LogStatus::BUFFER_PURGED),
            ),
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
            multiple
                .add_record(BACnetLogMultipleRecord {
                    date: record.date,
                    time: record.time,
                    log_data,
                })
                .unwrap();
            event
                .add_record(BACnetEventLogRecord {
                    date: record.date,
                    time: record.time,
                    log_datum: event_datum,
                })
                .unwrap();
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
        }
    }
    objects
}

// Independent (identifier, optional, writable) fixtures in legacy order.
// Kept with the log-family consumer tests so existing near-cap PICS and RPM
// test files need no unrelated splits.
/// `triggered`: the trend logs' Logging_Type is TRIGGERED, which makes their
/// Log_Interval read-only (Table 12-29 footnote 3, Table 12-35 footnote 2).
fn expected_rows(kind: ObjectType, triggered: bool) -> Vec<(P, bool, bool)> {
    // Every Trend Log samples a BACnet property, which makes its window,
    // Log_Interval and Log_DeviceObjectProperty required (Table 12-29
    // footnotes 1 and 8, #1481).
    let trend = kind == ObjectType::TREND_LOG;
    let mut rows = vec![
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::LOG_ENABLE, false, true),
        (P::LOG_INTERVAL, false, !triggered),
        (P::STOP_WHEN_FULL, false, true),
        (P::BUFFER_SIZE, false, false),
        (P::LOG_BUFFER, false, false),
        (P::RECORD_COUNT, false, true),
        (P::TOTAL_RECORD_COUNT, false, false),
        (P::STATUS_FLAGS, false, false),
        (P::EVENT_STATE, false, false),
        (P::RELIABILITY, true, false),
    ];
    if kind == ObjectType::EVENT_LOG {
        // Table 12-31 has no Log_Interval (#1064); its window is writable
        // (#1353).
        rows.retain(|row| row.0 != P::LOG_INTERVAL);
        rows.extend([P::START_TIME, P::STOP_TIME].map(|p| (p, true, true)));
    } else {
        rows.extend([
            // Both trend objects take POLLED or TRIGGERED (#1235, #1354), a
            // Trend Log COV too (#1480).
            (P::LOGGING_TYPE, false, true),
            // Writable on both trend objects (#1234).
            (P::LOG_DEVICE_OBJECT_PROPERTY, false, true),
        ]);
        // The window, clock alignment and Trigger, all writable (#1235,
        // #1353, #1354).
        rows.extend([P::START_TIME, P::STOP_TIME].map(|p| (p, !trend, true)));
        rows.extend([P::ALIGN_INTERVALS, P::INTERVAL_OFFSET, P::TRIGGER].map(|p| (p, true, true)));
        if trend {
            // A Trend Log's two COV rows, optional and writable (#1480).
            rows.extend(
                [P::COV_RESUBSCRIPTION_INTERVAL, P::CLIENT_COV_INCREMENT].map(|p| (p, true, true)),
            );
        }
    }
    // Every log's BUFFER_READY rows, the configuration ones writable (#1347).
    // Tables 12-29 and 12-35 (footnote 4) and 12-31 (footnote 3) require
    // them of a log that reports intrinsically, as these do, all but
    // Event_Message_Texts, which they only permit (#1485).
    rows.extend([
        (P::NOTIFICATION_THRESHOLD, false, true),
        (P::RECORDS_SINCE_NOTIFICATION, false, false),
        (P::LAST_NOTIFY_RECORD, false, false),
        (P::NOTIFICATION_CLASS, false, true),
        (P::EVENT_ENABLE, false, true),
        (P::ACKED_TRANSITIONS, false, false),
        (P::NOTIFY_TYPE, false, true),
        (P::EVENT_TIME_STAMPS, false, false),
        (P::EVENT_MESSAGE_TEXTS, true, false),
        // The message texts and the inhibit pair, only permitted (#1329).
        (P::EVENT_MESSAGE_TEXTS_CONFIG, true, true),
        (P::EVENT_DETECTION_ENABLE, false, true),
        (P::EVENT_ALGORITHM_INHIBIT_REF, true, true),
        (P::EVENT_ALGORITHM_INHIBIT, true, true),
    ]);
    rows.push((P::PROPERTY_LIST, false, false));
    rows
}

#[test]
fn rpm_metadata_log_selectors_pics_rows_and_budgets_are_exact() {
    for capacity in [0, 1, 3] {
        for configured in [false, true] {
            for object in log_objects(capacity, configured) {
                let oid = object.object_identifier();
                let expected = expected_rows(oid.object_type(), configured);
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
                let mut pics_expected = expected.clone();
                pics_expected.sort_by_key(|row| row.0.to_raw());
                assert_eq!(rows, pics_expected);
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
    request.encode(&mut bytes).unwrap();
    bytes
}

fn assert_budget_parity(db: &ObjectDatabase, request: &[u8], legacy: &[u8], count: usize) {
    use crate::handlers::{rpm_budget::handle_rpm_budgeted, ReadFailure};
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
        Err(ReadFailure::Work)
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
        Err(ReadFailure::Bytes)
    ));
    assert_eq!(&prefix[..], b"prefix");
}

#[test]
fn rpm_log_indexed_property_list_and_list_gates_preserve_bytes() {
    for object in log_objects(3, true) {
        let oid = object.object_identifier();
        let wire: Vec<_> = expected_rows(oid.object_type(), true)
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
            assert_eq!(
                result.property_array_index,
                if p == P::PROPERTY_LIST { index } else { None }
            );
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
                        if class == ErrorClass::PROPERTY && c == code));
                }
            }
        }
        assert_budget_parity(&db, &bytes, &legacy, references.len());
    }
}

/// Clauses 12.25.14, 12.27.13 and 12.30.19 open a log buffer to ReadRange
/// only (#1237): RP and RPM refuse Log_Buffer with READ_ACCESS_DENIED, and
/// ReadRange serves each record framed as its Clause 21 production with no
/// sequence number inside (#1233).
#[test]
fn rpm_and_rp_refuse_log_buffer_while_read_range_serves_framed_records() {
    use bacnet_services::read_range::{ReadRangeAck, ReadRangeRequest};
    for object in log_objects(3, true) {
        let oid = object.object_identifier();
        // Each record opens with its timestamp [0]; the datum [1] follows.
        // Only a Trend Log appends the supplied StatusFlags as [2]. The
        // second resident record is a log-status [0] for every family.
        let timestamp = [0x0e, 0xa4, 126, 9, 13, 7, 0xb4, 12, 0, 0, 0, 0x0f];
        let first: &[u8] = match oid.object_type() {
            // unsigned-value [4], then status-flags [2].
            ObjectType::TREND_LOG => &[0x1e, 0x49, 42, 0x1f, 0x2a, 0x04, 0xa0],
            // log-data [1] holding the member list [1].
            ObjectType::TREND_LOG_MULTIPLE => &[0x1e, 0x1e, 0x39, 42, 0x1f, 0x1f],
            // time-change [2]: 42.0 s.
            _ => &[0x1e, 0x2c, 0x42, 0x28, 0, 0, 0x1f],
        };
        // log-disabled and buffer-purged, bits 0 and 1, are the top two bits.
        let status = [0x1e, 0x0a, 5, 0xc0, 0x1f];
        let expected = [&timestamp[..], first, &timestamp, &status].concat();
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
            let rp = ReadPropertyRequest {
                object_identifier: oid,
                property_identifier: p,
                property_array_index: None,
            };
            let mut rp_bytes = BytesMut::new();
            rp.encode(&mut rp_bytes);
            let mut response = BytesMut::new();
            let rp_result = handle_read_property(&db, &rp_bytes, &mut response);
            if p == P::LOG_BUFFER {
                assert_eq!(
                    result.error,
                    Some((ErrorClass::PROPERTY, ErrorCode::READ_ACCESS_DENIED))
                );
                assert!(result.property_value.is_none());
                assert!(matches!(rp_result, Err(Error::Protocol { class, code })
                    if class == ErrorClass::PROPERTY
                        && code == ErrorCode::READ_ACCESS_DENIED));
            } else {
                assert!(result.error.is_none());
                assert_eq!(result.property_value.as_deref(), Some(&[0x21, 2][..]));
                rp_result.unwrap();
                assert_eq!(
                    ReadPropertyACK::decode(&response).unwrap().property_value,
                    [0x21, 2]
                );
            }
        }
        assert_budget_parity(&db, &bytes, &legacy, references.len());
        // ALL and REQUIRED both name Log_Buffer and report it inline.
        for selector in [P::ALL, P::REQUIRED] {
            let mut response = BytesMut::new();
            handle_read_property_multiple(&db, &request(oid, &[(selector, None)]), &mut response)
                .unwrap();
            let ack = ReadPropertyMultipleACK::decode(&response).unwrap();
            let results = &ack.list_of_read_access_results[0].list_of_results;
            let log_buffer = results
                .iter()
                .find(|result| result.property_identifier == P::LOG_BUFFER)
                .unwrap();
            assert_eq!(
                log_buffer.error,
                Some((ErrorClass::PROPERTY, ErrorCode::READ_ACCESS_DENIED)),
                "{oid:?} {selector:?}"
            );
            assert!(results
                .iter()
                .filter(|result| result.property_identifier != P::LOG_BUFFER)
                .all(|result| result.error.is_none()));
        }

        let mut range_request = BytesMut::new();
        ReadRangeRequest {
            object_identifier: oid,
            property_identifier: P::LOG_BUFFER,
            property_array_index: None,
            range: None,
        }
        .encode(&mut range_request)
        .unwrap();
        let mut range_ack = BytesMut::new();
        handle_read_range(&db, &range_request, &mut range_ack).unwrap();
        let range_ack = ReadRangeAck::decode(&range_ack).unwrap();
        assert_eq!(range_ack.item_count, 2);
        assert_eq!(range_ack.item_data, expected, "{oid:?}");
    }
}
