use super::*;
use bacnet_objects::{timer::TimerObject, traits::BACnetObject};
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use bacnet_types::primitives::{Date, Time};
use PropertyIdentifier as P;

#[test]
fn rpm_timer_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = TimerObject::new(7, "TMR-7").unwrap();
        if configured {
            object.start();
            object.set_initial_timeout(5000);
            let date = Date {
                year: 126,
                month: 9,
                day: 14,
                day_of_week: 1,
            };
            let time = Time {
                hour: 12,
                minute: 30,
                second: 15,
                hundredths: 25,
            };
            object.set_update_time(date, time);
            object.set_expiration_time(date, time);
        }
        object
            .write_property(
                P::OUT_OF_SERVICE,
                None,
                bacnet_types::primitives::PropertyValue::Boolean(configured),
                None,
            )
            .unwrap();
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the existing projection.
        type ExpectedRead = Result<&'static [u8], ErrorCode>;
        let unspec_datetime: &[u8] = &[0xa4, 0xff, 0xff, 0xff, 0xff, 0xb4, 0xff, 0xff, 0xff, 0xff];
        let configured_datetime: &[u8] = &[0xa4, 126, 9, 14, 1, 0xb4, 12, 30, 15, 25];
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (
                P::PRESENT_VALUE,
                None,
                Ok(if configured { &[0x91, 1] } else { &[0x91, 0] }),
            ),
            (
                P::PRESENT_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PRESENT_VALUE,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::TIMER_STATE,
                None,
                Ok(if configured { &[0x91, 1] } else { &[0x91, 0] }),
            ),
            (
                P::TIMER_STATE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::TIMER_RUNNING,
                None,
                Ok(if configured { &[0x11] } else { &[0x10] }),
            ),
            (
                P::TIMER_RUNNING,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::INITIAL_TIMEOUT,
                None,
                Ok(if configured {
                    &[0x22, 0x13, 0x88]
                } else {
                    &[0x21, 0]
                }),
            ),
            (
                P::INITIAL_TIMEOUT,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::UPDATE_TIME,
                None,
                Ok(if configured {
                    configured_datetime
                } else {
                    unspec_datetime
                }),
            ),
            (
                P::UPDATE_TIME,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::EXPIRATION_TIME,
                None,
                Ok(if configured {
                    configured_datetime
                } else {
                    unspec_datetime
                }),
            ),
            (
                P::EXPIRATION_TIME,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::STATUS_FLAGS,
                None,
                Ok(if configured {
                    &[0x82, 4, 0x10]
                } else {
                    &[0x82, 4, 0]
                }),
            ),
            (P::EVENT_STATE, None, Ok(&[0x91, 0])),
            (
                P::EVENT_STATE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::OUT_OF_SERVICE,
                None,
                Ok(if configured { &[0x11] } else { &[0x10] }),
            ),
            (P::RELIABILITY, None, Ok(&[0x91, 0])),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x91, 85, 0x92, 0x01, 0x8e, 0x92, 0x01, 0x8d, 0x92, 0x01, 0x8a, 0x91,
                    189, 0x92, 0x01, 0x0e, 0x91, 111, 0x91, 81, 0x91, 103, 0x91, 36,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 11])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 85])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x92, 0x01, 0x8e])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x92, 0x01, 0x8d])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x92, 0x01, 0x8a])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x91, 189])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x92, 0x01, 0x0e])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(10), Ok(&[0x91, 103])),
            (P::PROPERTY_LIST, Some(11), Ok(&[0x91, 36])),
            (
                P::PROPERTY_LIST,
                Some(12),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (P::DEFAULT_TIMEOUT, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::RESOLUTION, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
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
