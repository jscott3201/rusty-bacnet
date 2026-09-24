use super::*;
use bacnet_objects::{
    group::{GlobalGroupObject, GroupObject, StructuredViewObject},
    traits::BACnetObject,
};
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

const EMPTY: &[u8] = &[];

// Shared RP-vs-RPM parity plus budget parity over one case table.
type ExpectedRead = Result<&'static [u8], ErrorCode>;

fn assert_cases(
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
    .encode(&mut request)
    .unwrap();
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

fn write_common(object: &mut dyn BACnetObject, configured: bool) {
    object
        .write_property(
            P::DESCRIPTION,
            None,
            PropertyValue::CharacterString("long group label".repeat(100)),
            None,
        )
        .unwrap();
    object
        .write_property(
            P::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(configured),
            None,
        )
        .unwrap();
}

#[test]
fn rpm_group_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = GroupObject::new(7, "GRP-7").unwrap();
        if configured {
            let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
            let ai2 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap();
            object.add_member(ai1);
            object.add_member(ai2);
            object.present_value.push(PropertyValue::Enumerated(3));
        }
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the existing projection.
        // List_Of_Group_Members and Present_Value are BACnetLIST
        // (Table 12-17), so any index is PROPERTY_IS_NOT_AN_ARRAY.
        let members: &[u8] = if configured {
            &[0xC4, 0, 0, 0, 1, 0xC4, 0, 0, 0, 2]
        } else {
            EMPTY
        };
        let present_value: &[u8] = if configured { &[0x91, 3] } else { EMPTY };
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (P::LIST_OF_GROUP_MEMBERS, None, Ok(members)),
            (
                P::LIST_OF_GROUP_MEMBERS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::LIST_OF_GROUP_MEMBERS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::PRESENT_VALUE, None, Ok(present_value)),
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
                P::STATUS_FLAGS,
                None,
                Ok(if configured {
                    &[0x82, 4, 0x10]
                } else {
                    &[0x82, 4, 0]
                }),
            ),
            (
                P::STATUS_FLAGS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::OUT_OF_SERVICE,
                None,
                Ok(if configured { &[0x11] } else { &[0x10] }),
            ),
            (
                P::OUT_OF_SERVICE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::RELIABILITY, None, Ok(&[0x91, 0])),
            (
                P::RELIABILITY,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[0x91, 28, 0x91, 53, 0x91, 85, 0x91, 111, 0x91, 81, 0x91, 103]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 6])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 53])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 85])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x91, 103])),
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
            // Unserved Group table rows stay unknown.
            (P::PROFILE_NAME, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::PROFILE_NAME,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
        ];
        assert_cases(&db, oid, cases);
    }
}

#[test]
fn rpm_global_group_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = GlobalGroupObject::new(7, "GG-7").unwrap();
        if configured {
            let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
            object
                .group_members
                .push(BACnetDeviceObjectPropertyReference {
                    object_identifier: ai1,
                    property_identifier: P::PRESENT_VALUE.to_raw(),
                    property_array_index: None,
                    device_identifier: None,
                });
            object.group_member_names.push("a".into());
            object.present_value.push(PropertyValue::Enumerated(1));
            object.present_value.push(PropertyValue::Enumerated(2));
        }
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the existing projection.
        // Group_Members, Group_Member_Names, and Present_Value are
        // BACnetARRAY (Table 12-57), so the service gate admits an index and
        // the object arms return the whole value (the same documented residue
        // as Command/Staging arrays) — unlike Group, where Present_Value
        // rejects the index.
        let members: &[u8] = if configured {
            &[0xC4, 0, 0, 0, 1, 0x21, 85, 0x00, 0x00]
        } else {
            EMPTY
        };
        let names: &[u8] = if configured {
            &[0x72, 0x00, b'a']
        } else {
            EMPTY
        };
        let present_value: &[u8] = if configured {
            &[0x91, 1, 0x91, 2]
        } else {
            EMPTY
        };
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (P::GROUP_MEMBERS, None, Ok(members)),
            (P::GROUP_MEMBERS, Some(0), Ok(members)),
            (P::GROUP_MEMBERS, Some(1), Ok(members)),
            (P::GROUP_MEMBER_NAMES, None, Ok(names)),
            (P::GROUP_MEMBER_NAMES, Some(0), Ok(names)),
            (P::PRESENT_VALUE, None, Ok(present_value)),
            (P::PRESENT_VALUE, Some(0), Ok(present_value)),
            (P::PRESENT_VALUE, Some(1), Ok(present_value)),
            (
                P::STATUS_FLAGS,
                None,
                Ok(if configured {
                    &[0x82, 4, 0x10]
                } else {
                    &[0x82, 4, 0]
                }),
            ),
            (
                P::STATUS_FLAGS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::OUT_OF_SERVICE,
                None,
                Ok(if configured { &[0x11] } else { &[0x10] }),
            ),
            (
                P::OUT_OF_SERVICE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::RELIABILITY, None, Ok(&[0x91, 0])),
            (
                P::RELIABILITY,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x92, 0x01, 0x59, 0x91, 85, 0x92, 0x01, 0x5A, 0x91, 111, 0x91, 81,
                    0x91, 103,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 7])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x92, 0x01, 0x59])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 85])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x92, 0x01, 0x5A])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x91, 103])),
            (
                P::PROPERTY_LIST,
                Some(8),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // Unserved GlobalGroup table rows stay unknown: Event_State and
            // Member_Status_Flags have no read arm.
            (P::EVENT_STATE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::EVENT_STATE,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::MEMBER_STATUS_FLAGS,
                None,
                Err(ErrorCode::UNKNOWN_PROPERTY),
            ),
        ];
        assert_cases(&db, oid, cases);
    }
}

#[test]
fn rpm_structured_view_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = StructuredViewObject::new(7, "SV-7").unwrap();
        if configured {
            let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
            object.add_subordinate(ai1, "a");
        }
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the existing projection.
        // Subordinate_List and Subordinate_Annotations are BACnetARRAY
        // (Table 12-34), so the gate admits an index and the arms return the
        // whole value; the scalar Node_Type/Node_Subtype reject one.
        let subordinates: &[u8] = if configured {
            &[0xC4, 0, 0, 0, 1]
        } else {
            EMPTY
        };
        let annotations: &[u8] = if configured {
            &[0x72, 0x00, b'a']
        } else {
            EMPTY
        };
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (P::NODE_TYPE, None, Ok(&[0x91, 0])),
            (
                P::NODE_TYPE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            // Node_Subtype is never written by this fixture, so it reads
            // back empty in both states.
            (P::NODE_SUBTYPE, None, Ok(&[0x71, 0x00])),
            (
                P::NODE_SUBTYPE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::SUBORDINATE_LIST, None, Ok(subordinates)),
            (P::SUBORDINATE_LIST, Some(0), Ok(subordinates)),
            (P::SUBORDINATE_LIST, Some(1), Ok(subordinates)),
            (P::SUBORDINATE_ANNOTATIONS, None, Ok(annotations)),
            (P::SUBORDINATE_ANNOTATIONS, Some(0), Ok(annotations)),
            (
                P::STATUS_FLAGS,
                None,
                Ok(if configured {
                    &[0x82, 4, 0x10]
                } else {
                    &[0x82, 4, 0]
                }),
            ),
            (
                P::STATUS_FLAGS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::OUT_OF_SERVICE,
                None,
                Ok(if configured { &[0x11] } else { &[0x10] }),
            ),
            (
                P::OUT_OF_SERVICE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::RELIABILITY, None, Ok(&[0x91, 0])),
            (
                P::RELIABILITY,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x91, 208, 0x91, 207, 0x91, 211, 0x91, 210, 0x91, 111, 0x91, 81,
                    0x91, 103,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 8])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 208])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 207])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x91, 211])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x91, 210])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x91, 103])),
            (
                P::PROPERTY_LIST,
                Some(9),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // Unserved StructuredView table rows stay unknown.
            (P::SUBORDINATE_TAGS, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::SUBORDINATE_TAGS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
        ];
        assert_cases(&db, oid, cases);
    }
}
