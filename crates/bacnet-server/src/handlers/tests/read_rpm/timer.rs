use super::*;
use PropertyIdentifier as P;
use bacnet_objects::{timer::TimerObject, traits::BACnetObject};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::{Date, PropertyValue, Time};

/// A Device clock fixed at 2026-09-14 (a Monday) 12:30:15.25.
struct FixedClock;
impl bacnet_objects::clock::ClockReader for FixedClock {
    fn read_clock(&self) -> Option<bacnet_objects::clock::ClockFrame> {
        Some(bacnet_objects::clock::ClockFrame {
            local_date: Date {
                year: 126,
                month: 9,
                day: 14,
                day_of_week: 1,
            },
            local_time: Time {
                hour: 12,
                minute: 30,
                second: 15,
                hundredths: 25,
            },
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

#[test]
fn rpm_timer_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let object = TimerObject::new(7, "TMR-7").unwrap();
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.set_clock_reader(Some(std::sync::Arc::new(FixedClock)));
        db.add(Box::new(object)).unwrap();
        if configured {
            // Out of service first, so the 5 s countdown is held.
            let timer = db.get_mut(&oid).unwrap();
            for (p, value) in [
                (P::OUT_OF_SERVICE, PropertyValue::Boolean(true)),
                (P::PRESENT_VALUE, PropertyValue::Unsigned(5000)),
            ] {
                timer.write_property(p, None, value, None).unwrap();
            }
        }
        // Independent application-value bytes pin the projection.
        type ExpectedRead = Result<&'static [u8], ErrorCode>;
        let unspec_datetime: &[u8] = &[0xa4, 0xff, 0xff, 0xff, 0xff, 0xb4, 0xff, 0xff, 0xff, 0xff];
        let started: &[u8] = &[0xa4, 126, 9, 14, 1, 0xb4, 12, 30, 15, 25];
        let expires: &[u8] = &[0xa4, 126, 9, 14, 1, 0xb4, 12, 30, 20, 25];
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (
                P::PRESENT_VALUE,
                None,
                Ok(if configured {
                    &[0x22, 0x13, 0x88]
                } else {
                    &[0x21, 0]
                }),
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
                P::LAST_STATE_CHANGE,
                None,
                Ok(if configured { &[0x91, 1] } else { &[0x91, 0] }),
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
            (P::DEFAULT_TIMEOUT, None, Ok(&[0x22, 0xea, 0x60])),
            (P::MIN_PRES_VALUE, None, Ok(&[0x21, 1])),
            (P::MAX_PRES_VALUE, None, Ok(&[0x24, 0xff, 0xff, 0xff, 0xff])),
            (
                P::UPDATE_TIME,
                None,
                Ok(if configured { started } else { unspec_datetime }),
            ),
            (
                P::UPDATE_TIME,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::EXPIRATION_TIME,
                None,
                Ok(if configured { expires } else { unspec_datetime }),
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
                    0x91, 28, 0x91, 85, 0x91, 111, 0x91, 36, 0x91, 103, 0x91, 81, 0x92, 0x01, 0x8e,
                    0x92, 0x01, 0x8d, 0x91, 189, 0x92, 0x01, 0x8b, 0x92, 0x01, 0x0e, 0x92, 0x01,
                    0x8a, 0x92, 0x01, 0x89, 0x91, 69, 0x91, 65,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 15])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 85])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x92, 0x01, 0x8e])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x92, 0x01, 0x8d])),
            (P::PROPERTY_LIST, Some(10), Ok(&[0x92, 0x01, 0x8b])),
            (P::PROPERTY_LIST, Some(13), Ok(&[0x92, 0x01, 0x89])),
            (P::PROPERTY_LIST, Some(15), Ok(&[0x91, 65])),
            (
                P::PROPERTY_LIST,
                Some(16),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (P::RESOLUTION, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::STATE_CHANGE_VALUES,
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
        use crate::handlers::rpm_budget::handle_rpm_budgeted;
        use bacnet_device_core::read::ReadFailure;
        let budget = bacnet_device_core::read::ReadPropertyMultipleBudget {
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
                bacnet_device_core::read::ReadPropertyMultipleBudget {
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
                bacnet_device_core::read::ReadPropertyMultipleBudget {
                    max_service_ack_bytes: legacy.len() - 1,
                    ..budget
                }
            ),
            Err(ReadFailure::Bytes)
        ));
        assert_eq!(&prefix[..], b"prefix");
    }
}
