use super::*;
use bacnet_objects::{
    life_safety::{LifeSafetyPointObject, LifeSafetyZoneObject},
    traits::BACnetObject,
};
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use PropertyIdentifier as P;

type ExpectedRead = Result<&'static [u8], ErrorCode>;

#[test]
fn rpm_life_safety_point_indexed_reads_and_list_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = LifeSafetyPointObject::new(7, "LSP-7").unwrap();
        if configured {
            object.set_direct_reading(42.5);
            object.add_member(ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, 9).unwrap());
        }
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent bytes pin the existing enumerated/Boolean/Real/list
        // projection, not a new wire codec or positional list access.
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (P::PRESENT_VALUE, None, Ok(&[0x91, 0])),
            (
                P::PRESENT_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::MODE, None, Ok(&[0x91, 0])),
            (P::SILENCED, None, Ok(&[0x91, 0])),
            (P::OPERATION_EXPECTED, None, Ok(&[0x91, 0])),
            (P::TRACKING_VALUE, None, Ok(&[0x91, 0])),
            (
                P::MEMBER_OF,
                None,
                Ok(if configured {
                    &[0xC4, 0x05, 0x80, 0x00, 0x09]
                } else {
                    &[]
                }),
            ),
            (
                P::MEMBER_OF,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::MEMBER_OF,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::DIRECT_READING,
                None,
                Ok(if configured {
                    &[0x44, 0x42, 0x2A, 0x00, 0x00]
                } else {
                    &[0x44, 0, 0, 0, 0]
                }),
            ),
            (
                P::DIRECT_READING,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::MAINTENANCE_REQUIRED, None, Ok(&[0x10])),
            (P::EVENT_STATE, None, Ok(&[0x91, 0])),
            (P::STATUS_FLAGS, None, Ok(&[0x82, 4, 0])),
            (P::OUT_OF_SERVICE, None, Ok(&[0x10])),
            (P::RELIABILITY, None, Ok(&[0x91, 0])),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x91, 85, 0x91, 160, 0x91, 163, 0x91, 161, 0x91, 164, 0x91, 159,
                    0x91, 156, 0x91, 158, 0x91, 36, 0x91, 111, 0x91, 81, 0x91, 103,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 13])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(13), Ok(&[0x91, 103])),
            (
                P::PROPERTY_LIST,
                Some(14),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (P::ACCEPTED_MODES, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::RELIABILITY_EVALUATION_INHIBIT,
                None,
                Err(ErrorCode::UNKNOWN_PROPERTY),
            ),
        ];
        assert_indexed_cases(&db, oid, cases);
    }
}

#[test]
fn rpm_life_safety_zone_indexed_reads_and_list_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = LifeSafetyZoneObject::new(7, "LSZ-7").unwrap();
        if configured {
            object
                .add_zone_member(ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 3).unwrap());
        }
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (P::PRESENT_VALUE, None, Ok(&[0x91, 0])),
            (P::MODE, None, Ok(&[0x91, 0])),
            (P::SILENCED, None, Ok(&[0x91, 0])),
            (P::OPERATION_EXPECTED, None, Ok(&[0x91, 0])),
            (
                P::ZONE_MEMBERS,
                None,
                Ok(if configured {
                    &[0xC4, 0x05, 0x40, 0x00, 0x03]
                } else {
                    &[]
                }),
            ),
            (
                P::ZONE_MEMBERS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::EVENT_STATE, None, Ok(&[0x91, 0])),
            (P::STATUS_FLAGS, None, Ok(&[0x82, 4, 0])),
            (P::OUT_OF_SERVICE, None, Ok(&[0x10])),
            (P::RELIABILITY, None, Ok(&[0x91, 0])),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x91, 85, 0x91, 160, 0x91, 163, 0x91, 161, 0x91, 165, 0x91, 36, 0x91,
                    111, 0x91, 81, 0x91, 103,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 10])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(10), Ok(&[0x91, 103])),
            (
                P::PROPERTY_LIST,
                Some(11),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // The zone serves no Tracking_Value or Member_Of; both stay unknown.
            (P::TRACKING_VALUE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::MEMBER_OF, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::ACCEPTED_MODES, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
        ];
        assert_indexed_cases(&db, oid, cases);
    }
}

fn assert_indexed_cases(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    cases: &[(P, Option<u32>, ExpectedRead)],
) {
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
    handle_read_property_multiple(db, &request, &mut legacy).unwrap();
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
        let rp = handle_read_property(db, &rp_request, &mut response);
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
    handle_rpm_budgeted(db, &request, &mut bounded, budget).unwrap();
    assert_eq!(bounded, legacy);
    let mut prefix = BytesMut::from(&b"prefix"[..]);
    assert!(matches!(
        handle_rpm_budgeted(
            db,
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
            db,
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
