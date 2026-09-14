use super::*;
use bacnet_objects::{schedule::ScheduleObject, traits::BACnetObject};
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use bacnet_types::constructed::{
    BACnetCalendarEntry, BACnetObjectPropertyReference, BACnetSpecialEvent, BACnetTimeValue,
    SpecialEventPeriod,
};
use bacnet_types::primitives::{Date, PropertyValue, Time};
use PropertyIdentifier as P;

#[test]
fn rpm_schedule_indexed_reads_and_constructed_bytes_are_unchanged() {
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
                value: vec![0x21, 42],
            };
            object.set_weekly_schedule(0, vec![tv.clone()]);
            object.set_weekly_schedule(6, vec![tv.clone()]);
            object.add_exception(BACnetSpecialEvent {
                period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::Date(Date {
                    year: 126,
                    month: 9,
                    day: 14,
                    day_of_week: 1,
                })),
                list_of_time_values: vec![tv],
                event_priority: 3,
            });
            object.add_object_property_reference(BACnetObjectPropertyReference::new(
                ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 2).unwrap(),
                P::PRESENT_VALUE.to_raw(),
            ));
        }
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Pin the existing application-value representation, not a new wire codec.
        type ExpectedRead = Result<&'static [u8], ErrorCode>;
        let day: &[u8] = if configured {
            &[0xb4, 8, 30, 0, 0, 0x62, 0x21, 42]
        } else {
            &[]
        };
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (
                P::WEEKLY_SCHEDULE,
                None,
                Ok(if configured {
                    &[
                        0xb4, 8, 30, 0, 0, 0x62, 0x21, 42, 0xb4, 8, 30, 0, 0, 0x62, 0x21, 42,
                    ]
                } else {
                    &[]
                }),
            ),
            (P::WEEKLY_SCHEDULE, Some(0), Ok(&[0x21, 7])),
            (P::WEEKLY_SCHEDULE, Some(1), Ok(day)),
            (P::WEEKLY_SCHEDULE, Some(2), Ok(&[])),
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
                Ok(if configured {
                    &[0x21, 3, 0xb4, 8, 30, 0, 0, 0x62, 0x21, 42]
                } else {
                    &[]
                }),
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
                    Ok(&[0x21, 3, 0xb4, 8, 30, 0, 0, 0x62, 0x21, 42])
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
                    &[0xc4, 0, 0x40, 0, 2, 0x91, 85]
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
            (P::EFFECTIVE_PERIOD, None, Ok(&[0])),
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
        .encode(&mut request);
        let mut legacy = BytesMut::new();
        handle_read_property_multiple(&db, &request, &mut legacy).unwrap();
        let ack = ReadPropertyMultipleACK::decode(&legacy).unwrap();
        assert_eq!(ack.list_of_read_access_results.len(), 1);
        let access = &ack.list_of_read_access_results[0];
        assert_eq!(access.object_identifier, oid);
        assert_eq!(access.list_of_results.len(), cases.len());
        for (result, &(p, i, expected)) in access.list_of_results.iter().zip(cases) {
            assert_eq!(result.property_identifier, p);
            assert_eq!(result.property_array_index, i);
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
                    assert!(matches!(rp, Err(Error::Protocol { class, code })
                        if class == ErrorClass::PROPERTY.to_raw() as u32 && code == expected.to_raw() as u32));
                    assert!(response.is_empty());
                }
            }
        }
        use crate::handlers::rpm_budget::{handle_rpm_budgeted, RpmFailure};
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
            Err(RpmFailure::Work)
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
            Err(RpmFailure::Bytes)
        ));
        assert_eq!(&prefix[..], b"prefix");
    }
}
