use super::*;
use PropertyIdentifier as P;
use bacnet_objects::{schedule::CalendarObject, traits::BACnetObject};
use bacnet_types::constructed::{
    BACnetCalendarEntry, BACnetDateRange, BACnetWeekNDay, PropertyReference,
    ReadAccessSpecification,
};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::Date;

#[test]
fn rpm_calendar_indexed_reads_and_date_list_wire_bytes() {
    for configured in [false, true] {
        let mut object = CalendarObject::new(7, "CAL-7").unwrap();
        if configured {
            let date = Date {
                year: 126,
                month: 9,
                day: 14,
                day_of_week: 1,
            };
            for entry in [
                BACnetCalendarEntry::Date(date),
                BACnetCalendarEntry::DateRange(BACnetDateRange {
                    start_date: date,
                    end_date: date,
                }),
                BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
                    month: 255,
                    week_of_month: 255,
                    day_of_week: 1,
                }),
            ] {
                object.add_date_entry(entry).unwrap();
            }
        }
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        // Monday 14 September 2026 matches every configured entry, so
        // Present_Value is TRUE exactly when Date_List holds them (#1029).
        db.set_clock_reader(Some(crate::schedule::tests::SettableClock::at(
            2026, 9, 14, 12, 0,
        )));
        db.add(Box::new(object)).unwrap();
        // Independent bytes pin each entry under its Clause 21 CHOICE tag
        // (#996): date [0] (0x0C), the date-range [1] frame (0x1E ... 0x1F)
        // around two application Dates (0xA4), and weekNDay [2] (0x2B). The
        // old projection was an application Date and two Octet Strings.
        type ExpectedRead = Result<&'static [u8], ErrorCode>;
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (
                P::DATE_LIST,
                None,
                Ok(if configured {
                    &[
                        0x0c, 126, 9, 14, 1, 0x1e, 0xa4, 126, 9, 14, 1, 0xa4, 126, 9, 14, 1, 0x1f,
                        0x2b, 255, 255, 1,
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
            // Table 12-11 defines none of these, so Calendar doesn't serve them
            // (#984).
            (P::STATUS_FLAGS, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::EVENT_STATE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::OUT_OF_SERVICE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::PROPERTY_LIST, None, Ok(&[0x91, 28, 0x91, 85, 0x91, 23])),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 3])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 85])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 23])),
            (
                P::PROPERTY_LIST,
                Some(4),
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
