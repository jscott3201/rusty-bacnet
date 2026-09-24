use super::*;
use bacnet_objects::{schedule::CalendarObject, traits::BACnetObject};
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use bacnet_types::constructed::{BACnetCalendarEntry, BACnetDateRange, BACnetWeekNDay};
use bacnet_types::primitives::Date;
use PropertyIdentifier as P;

#[test]
fn rpm_calendar_indexed_reads_and_date_list_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = CalendarObject::new(7, "CAL-7").unwrap();
        if configured {
            let date = Date {
                year: 126,
                month: 9,
                day: 14,
                day_of_week: 1,
            };
            object.add_date_entry(BACnetCalendarEntry::Date(date));
            object.add_date_entry(BACnetCalendarEntry::DateRange(BACnetDateRange {
                start_date: date,
                end_date: date,
            }));
            object.add_date_entry(BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
                month: 255,
                week_of_month: 255,
                day_of_week: 1,
            }));
            object.set_present_value(true);
        }
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent bytes pin the existing Date/OctetString projection, not a
        // new constructed CalendarEntry encoding or positional list access.
        type ExpectedRead = Result<&'static [u8], ErrorCode>;
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (
                P::DATE_LIST,
                None,
                Ok(if configured {
                    &[
                        0xa4, 126, 9, 14, 1, 0x65, 8, 126, 9, 14, 1, 126, 9, 14, 1, 0x63, 255, 255,
                        1,
                    ]
                } else {
                    &[]
                }),
            ),
            (
                P::DATE_LIST,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::DATE_LIST,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::DATE_LIST,
                Some(u32::MAX),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PRESENT_VALUE,
                None,
                Ok(if configured { &[0x11] } else { &[0x10] }),
            ),
            (
                P::PRESENT_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::STATUS_FLAGS, None, Ok(&[0x82, 4, 0])),
            (P::EVENT_STATE, None, Ok(&[0x91, 0])),
            (P::OUT_OF_SERVICE, None, Ok(&[0x10])),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[0x91, 28, 0x91, 85, 0x91, 23, 0x91, 111, 0x91, 36, 0x91, 81]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 6])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 85])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 23])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x91, 36])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x91, 81])),
            (
                P::PROPERTY_LIST,
                Some(7),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (P::RELIABILITY, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
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
