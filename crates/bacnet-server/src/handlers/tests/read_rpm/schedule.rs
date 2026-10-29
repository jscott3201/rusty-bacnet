use super::*;
use PropertyIdentifier as P;
use bacnet_objects::{schedule::ScheduleObject, traits::BACnetObject};
use bacnet_types::constructed::{
    BACnetCalendarEntry, BACnetObjectPropertyReference, BACnetSpecialEvent, BACnetTimeValue,
    PropertyReference, ReadAccessSpecification, SpecialEventPeriod,
};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::{Date, PropertyValue, Time};

#[test]
fn rpm_schedule_indexed_reads_and_reference_wire_bytes() {
    for configured in [false, true] {
        let mut object = ScheduleObject::new(7, "SCH-7", PropertyValue::Unsigned(42)).unwrap();
        if configured {
            let tv = BACnetTimeValue {
                time: Time {
                    hour: 8,
                    minute: 30,
                    second: 0,
                    hundredths: 0,
                },
                value: PropertyValue::Unsigned(42),
            };
            object.set_weekly_schedule(0, vec![tv.clone()]).unwrap();
            object.set_weekly_schedule(6, vec![tv.clone()]).unwrap();
            object
                .add_exception(BACnetSpecialEvent {
                    period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::Date(Date {
                        year: 126,
                        month: 9,
                        day: 14,
                        day_of_week: 1,
                    })),
                    list_of_time_values: vec![tv],
                    event_priority: 3,
                })
                .unwrap();
            object
                .add_object_property_reference(BACnetObjectPropertyReference::new(
                    ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 2).unwrap(),
                    P::PRESENT_VALUE.to_raw(),
                ))
                .unwrap();
            object
                .add_object_property_reference(BACnetObjectPropertyReference::new_indexed(
                    ObjectIdentifier::new(ObjectType::MULTI_STATE_OUTPUT, 7).unwrap(),
                    P::STATE_TEXT.to_raw(),
                    2,
                ))
                .unwrap();
        }
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Pin reference members as context-tagged local DOPR bodies, and the
        // Clause 21 schedule forms (#996): each BACnetDailySchedule is a [0]
        // frame (0x0E ... 0x0F) around its time-values, an application Time
        // (0xB4) then the value under its own tag (Unsigned 42 is 0x21 0x2A);
        // a special event is its period (here the
        // calendar-entry [0] frame around date [0], 0x0C), the [2] time-value
        // frame and event-priority [3] (0x39).
        type ExpectedRead = Result<&'static [u8], ErrorCode>;
        let empty_day: &[u8] = &[0x0e, 0x0f];
        let day: &[u8] = if configured {
            &[0x0e, 0xb4, 8, 30, 0, 0, 0x21, 42, 0x0f]
        } else {
            empty_day
        };
        let event: &[u8] = &[
            0x0e, 0x0c, 126, 9, 14, 1, 0x0f, 0x2e, 0xb4, 8, 30, 0, 0, 0x21, 42, 0x2f, 0x39, 3,
        ];
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (
                P::WEEKLY_SCHEDULE,
                None,
                Ok(if configured {
                    &[
                        0x0e, 0xb4, 8, 30, 0, 0, 0x21, 42, 0x0f, 0x0e, 0x0f, 0x0e, 0x0f, 0x0e,
                        0x0f, 0x0e, 0x0f, 0x0e, 0x0f, 0x0e, 0xb4, 8, 30, 0, 0, 0x21, 42, 0x0f,
                    ]
                } else {
                    &[
                        0x0e, 0x0f, 0x0e, 0x0f, 0x0e, 0x0f, 0x0e, 0x0f, 0x0e, 0x0f, 0x0e, 0x0f,
                        0x0e, 0x0f,
                    ]
                }),
            ),
            (P::WEEKLY_SCHEDULE, Some(0), Ok(&[0x21, 7])),
            (P::WEEKLY_SCHEDULE, Some(1), Ok(day)),
            (P::WEEKLY_SCHEDULE, Some(2), Ok(empty_day)),
            (P::WEEKLY_SCHEDULE, Some(7), Ok(day)),
            (
                P::WEEKLY_SCHEDULE,
                Some(8),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::WEEKLY_SCHEDULE,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::EXCEPTION_SCHEDULE,
                None,
                Ok(if configured { event } else { &[] }),
            ),
            (
                P::EXCEPTION_SCHEDULE,
                Some(0),
                Ok(if configured { &[0x21, 1] } else { &[0x21, 0] }),
            ),
            (
                P::EXCEPTION_SCHEDULE,
                Some(1),
                if configured {
                    Ok(event)
                } else {
                    Err(ErrorCode::INVALID_ARRAY_INDEX)
                },
            ),
            (
                P::EXCEPTION_SCHEDULE,
                Some(2),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::EXCEPTION_SCHEDULE,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
                None,
                Ok(if configured {
                    &[
                        0x0c, 0, 0x40, 0, 2, 0x19, 85, 0x0c, 3, 0x80, 0, 7, 0x19, 110, 0x29, 2,
                    ]
                } else {
                    &[]
                }),
            ),
            (
                P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::PRESENT_VALUE, None, Ok(&[0x21, 42])),
            (P::SCHEDULE_DEFAULT, None, Ok(&[0x21, 42])),
            // Both dates unspecified: the range covering every date.
            (
                P::EFFECTIVE_PERIOD,
                None,
                Ok(&[0xa4, 0xff, 0xff, 0xff, 0xff, 0xa4, 0xff, 0xff, 0xff, 0xff]),
            ),
            (P::PRIORITY_FOR_WRITING, None, Ok(&[0x21, 16])),
            (
                P::PRIORITY_FOR_WRITING,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x91, 85, 0x91, 174, 0x91, 123, 0x91, 38, 0x91, 32, 0x91, 54, 0x91,
                    111, 0x91, 36, 0x91, 103, 0x91, 81, 0x91, 88,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 12])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(11), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(12), Ok(&[0x91, 88])),
            (
                P::PROPERTY_LIST,
                Some(13),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::RELIABILITY_EVALUATION_INHIBIT,
                None,
                Err(ErrorCode::UNKNOWN_PROPERTY),
            ),
        ];
        let mut request = BytesMut::new();
        ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: oid,
                list_of_property_references: cases
                    .iter()
                    .map(|&(p, i, _)| PropertyReference {
                        property_identifier: p,
                        property_array_index: i,
                    })
                    .collect(),
            }],
        }
        .encode(&mut request)
        .unwrap();
        let mut legacy = BytesMut::new();
        handle_read_property_multiple(&db, &request, &mut legacy).unwrap();
        let ack = ReadPropertyMultipleACK::decode(&legacy).unwrap();
        assert_eq!(ack.list_of_read_access_results.len(), 1);
        let access = &ack.list_of_read_access_results[0];
        assert_eq!(access.object_identifier, oid);
        assert_eq!(access.list_of_results.len(), cases.len());
        for (result, &(p, i, expected)) in access.list_of_results.iter().zip(cases) {
            assert_eq!(result.property_identifier, p);
            // These table errors identify non-arrays or absent optional rows.
            let response_index = if matches!(
                expected,
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY | ErrorCode::UNKNOWN_PROPERTY)
            ) {
                None
            } else {
                i
            };
            assert_eq!(result.property_array_index, response_index);
            let mut rp_request = BytesMut::new();
            ReadPropertyRequest {
                object_identifier: oid,
                property_identifier: p,
                property_array_index: i,
            }
            .encode(&mut rp_request);
            let mut response = BytesMut::new();
            let rp = handle_read_property(&db, &rp_request, &mut response);
            match expected {
                Ok(bytes) => {
                    assert!(result.error.is_none(), "{p:?} {i:?}");
                    assert_eq!(result.property_value.as_deref(), Some(bytes), "{p:?} {i:?}");
                    rp.unwrap();
                    let rp_ack = ReadPropertyACK::decode(&response).unwrap();
                    assert_eq!(rp_ack.object_identifier, oid);
                    assert_eq!(rp_ack.property_identifier, p);
                    assert_eq!(rp_ack.property_array_index, i);
                    assert_eq!(rp_ack.property_value, bytes);
                }
                Err(expected) => {
                    assert!(result.property_value.is_none());
                    assert_eq!(result.error, Some((ErrorClass::PROPERTY, expected)));
                    assert!(
                        matches!(rp, Err(Error::Remote(RemoteError::Error { class, code }))
                        if class == ErrorClass::PROPERTY && code == expected)
                    );
                    assert!(response.is_empty());
                }
            }
        }
        use crate::handlers::{ReadFailure, rpm_budget::handle_rpm_budgeted};
        let budget = crate::server::ReadPropertyMultipleBudget {
            max_result_elements: cases.len(),
            max_service_ack_bytes: legacy.len(),
        };
        let mut bounded = BytesMut::new();
        handle_rpm_budgeted(&db, &request, &mut bounded, budget).unwrap();
        assert_eq!(bounded, legacy);
        let mut prefix = BytesMut::from(&b"prefix"[..]);
        assert!(matches!(
            handle_rpm_budgeted(
                &db,
                &request,
                &mut prefix,
                crate::server::ReadPropertyMultipleBudget {
                    max_result_elements: cases.len() - 1,
                    ..budget
                }
            ),
            Err(ReadFailure::Work)
        ));
        assert_eq!(&prefix[..], b"prefix");
        assert!(matches!(
            handle_rpm_budgeted(
                &db,
                &request,
                &mut prefix,
                crate::server::ReadPropertyMultipleBudget {
                    max_service_ack_bytes: legacy.len() - 1,
                    ..budget
                }
            ),
            Err(ReadFailure::Bytes)
        ));
        assert_eq!(&prefix[..], b"prefix");
    }
}
