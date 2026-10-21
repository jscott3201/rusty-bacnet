use super::*;
use bacnet_objects::{
    group::{GlobalGroupObject, GroupObject, StructuredViewObject},
    traits::BACnetObject,
};
use bacnet_types::constructed::{
    AccessResult, BACnetDeviceObjectPropertyReference, PropertyReference, ReadAccessSpecification,
};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

const EMPTY: &[u8] = &[];

// Shared RP-vs-RPM parity plus budget parity over one case table.
pub(super) type ExpectedRead = Result<&'static [u8], ErrorCode>;

pub(super) fn assert_cases(
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
                    if class == ErrorClass::PROPERTY && code == expected));
                assert!(response.is_empty());
            }
        }
    }
    use crate::handlers::{rpm_budget::handle_rpm_budgeted, ReadFailure};
    let members = group_member_rows(db, oid, cases.iter().map(|&(p, i, _)| (p, i)));
    let budget = crate::server::ReadPropertyMultipleBudget {
        max_result_elements: cases.len() + members,
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
                max_result_elements: budget.max_result_elements - 1,
                ..budget
            }
        ),
        Err(ReadFailure::Work)
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
        Err(ReadFailure::Bytes)
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
    // Only Global Group has Out_Of_Service (Table 12-57).
    if object.object_identifier().object_type() == ObjectType::GLOBAL_GROUP {
        object
            .write_property(
                P::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(configured),
                None,
            )
            .unwrap();
    }
}

#[test]
fn rpm_group_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = GroupObject::new(7, "GRP-7").unwrap();
        if configured {
            for instance in [1, 2] {
                object
                    .add_member(ReadAccessSpecification {
                        object_identifier: ObjectIdentifier::new(
                            ObjectType::ANALOG_INPUT,
                            instance,
                        )
                        .unwrap(),
                        list_of_property_references: vec![PropertyReference {
                            property_identifier: P::PRESENT_VALUE,
                            property_array_index: None,
                        }],
                    })
                    .unwrap();
            }
        }
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent bytes pin the projection. List_Of_Group_Members and
        // Present_Value are BACnetLIST (Table 12-17), so any index is
        // PROPERTY_IS_NOT_AN_ARRAY. The members are ReadAccessSpecifications
        // (#1134); neither AI is in this database, so each Present_Value
        // result carries OBJECT / UNKNOWN_OBJECT.
        let members: &[u8] = if configured {
            &[
                0x0C, 0, 0, 0, 1, 0x1E, 0x09, 85, 0x1F, 0x0C, 0, 0, 0, 2, 0x1E, 0x09, 85, 0x1F,
            ]
        } else {
            EMPTY
        };
        let present_value: &[u8] = if configured {
            &[
                0x0C, 0, 0, 0, 1, 0x1E, 0x29, 85, 0x5E, 0x91, 1, 0x91, 31, 0x5F, 0x1F, 0x0C, 0, 0,
                0, 2, 0x1E, 0x29, 85, 0x5E, 0x91, 1, 0x91, 31, 0x5F, 0x1F,
            ]
        } else {
            EMPTY
        };
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
            // Table 12-17 has no Status_Flags, Out_Of_Service or Reliability
            // (#1064).
            (P::STATUS_FLAGS, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::STATUS_FLAGS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::OUT_OF_SERVICE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::OUT_OF_SERVICE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::RELIABILITY, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::RELIABILITY,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::PROPERTY_LIST, None, Ok(&[0x91, 28, 0x91, 53, 0x91, 85])),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 3])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 53])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 85])),
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
fn rpm_global_group_indexed_reads_serve_array_elements() {
    for configured in [false, true] {
        let object = global_group(configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Group_Members, Group_Member_Names and Present_Value are BACnetARRAY
        // (Table 12-57): index 0 is the size, then one element per index,
        // and past the end is INVALID_ARRAY_INDEX. Group_Members elements are
        // BACnetDeviceObjectPropertyReference and Present_Value elements
        // BACnetPropertyAccessResult (#1107), unlike Group, whose
        // Present_Value is a list and rejects any index.
        let mut cases: Vec<(P, Option<u32>, ExpectedRead)> = if configured {
            vec![
                (P::GROUP_MEMBERS, None, Ok(GG_MEMBERS)),
                (P::GROUP_MEMBERS, Some(0), Ok(&[0x21, 2])),
                (P::GROUP_MEMBERS, Some(1), Ok(GG_MEMBER_1)),
                (P::GROUP_MEMBERS, Some(2), Ok(GG_MEMBER_2)),
                (
                    P::GROUP_MEMBERS,
                    Some(3),
                    Err(ErrorCode::INVALID_ARRAY_INDEX),
                ),
                (
                    P::GROUP_MEMBER_NAMES,
                    None,
                    Ok(&[0x72, 0x00, b'a', 0x72, 0x00, b'b']),
                ),
                (P::GROUP_MEMBER_NAMES, Some(0), Ok(&[0x21, 2])),
                (P::GROUP_MEMBER_NAMES, Some(2), Ok(&[0x72, 0x00, b'b'])),
                (
                    P::GROUP_MEMBER_NAMES,
                    Some(3),
                    Err(ErrorCode::INVALID_ARRAY_INDEX),
                ),
                (P::PRESENT_VALUE, None, Ok(GG_PRESENT_VALUE)),
                (P::PRESENT_VALUE, Some(0), Ok(&[0x21, 2])),
                (P::PRESENT_VALUE, Some(1), Ok(GG_RESULT_1)),
                (P::PRESENT_VALUE, Some(2), Ok(GG_RESULT_2)),
                (
                    P::PRESENT_VALUE,
                    Some(3),
                    Err(ErrorCode::INVALID_ARRAY_INDEX),
                ),
            ]
        } else {
            [P::GROUP_MEMBERS, P::GROUP_MEMBER_NAMES, P::PRESENT_VALUE]
                .into_iter()
                .flat_map(|p| {
                    [
                        (p, None, Ok(EMPTY)),
                        (p, Some(0), Ok(&[0x21, 0][..])),
                        (p, Some(1), Err(ErrorCode::INVALID_ARRAY_INDEX)),
                    ]
                })
                .collect()
        };
        cases.extend_from_slice(&[
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
            // Event_State stays NORMAL without intrinsic reporting, and no
            // member here references Status_Flags, so Member_Status_Flags is
            // all clear (#1092).
            (P::EVENT_STATE, None, Ok(&[0x91, 0])),
            (
                P::EVENT_STATE,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::MEMBER_STATUS_FLAGS, None, Ok(&[0x82, 4, 0])),
            (
                P::MEMBER_STATUS_FLAGS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x92, 0x01, 0x59, 0x91, 85, 0x92, 0x01, 0x5A, 0x91, 111, 0x91, 36,
                    0x92, 0x01, 0x5B, 0x91, 81, 0x91, 103,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 9])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x92, 0x01, 0x59])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 85])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x92, 0x01, 0x5A])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x91, 36])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x92, 0x01, 0x5B])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x91, 103])),
            (
                P::PROPERTY_LIST,
                Some(10),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // The COVU rows stay unknown: the object sends no unsubscribed
            // COV.
            (P::COVU_PERIOD, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
        ]);
        assert_cases(&db, oid, &cases);
    }
}

// The configured Global Group's arrays, written out from the Clause 21 tags:
// member 1 is AI-1 Present_Value, member 2 AI-2 Present_Value in device 9.
// Member 1's read returned ENUMERATED 1 and member 2's failed with
// OBJECT / UNKNOWN_OBJECT.
const GG_MEMBER_1: &[u8] = &[0x0C, 0, 0, 0, 1, 0x19, 85];
const GG_MEMBER_2: &[u8] = &[0x0C, 0, 0, 0, 2, 0x19, 85, 0x3C, 0x02, 0, 0, 9];
const GG_MEMBERS: &[u8] = &[
    0x0C, 0, 0, 0, 1, 0x19, 85, 0x0C, 0, 0, 0, 2, 0x19, 85, 0x3C, 0x02, 0, 0, 9,
];
const GG_RESULT_1: &[u8] = &[0x0C, 0, 0, 0, 1, 0x19, 85, 0x4E, 0x91, 1, 0x4F];
const GG_RESULT_2: &[u8] = &[
    0x0C, 0, 0, 0, 2, 0x19, 85, 0x3C, 0x02, 0, 0, 9, 0x5E, 0x91, 1, 0x91, 31, 0x5F,
];
const GG_PRESENT_VALUE: &[u8] = &[
    0x0C, 0, 0, 0, 1, 0x19, 85, 0x4E, 0x91, 1, 0x4F, 0x0C, 0, 0, 0, 2, 0x19, 85, 0x3C, 0x02, 0, 0,
    9, 0x5E, 0x91, 1, 0x91, 31, 0x5F,
];

/// A Global Group with the two members above, or none.
fn global_group(configured: bool) -> GlobalGroupObject {
    let mut object = GlobalGroupObject::new(7, "GG-7").unwrap();
    if configured {
        let ai = |instance| ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap();
        let device = ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap();
        object
            .set_group_members(vec![
                BACnetDeviceObjectPropertyReference::new_local(ai(1), P::PRESENT_VALUE.to_raw()),
                BACnetDeviceObjectPropertyReference::new_remote(
                    ai(2),
                    P::PRESENT_VALUE.to_raw(),
                    device,
                ),
            ])
            .unwrap();
        object.group_member_names = vec!["a".into(), "b".into()];
        object.present_value = vec![
            AccessResult::Value(PropertyValue::Enumerated(1)),
            AccessResult::Error {
                class: ErrorClass::OBJECT,
                code: ErrorCode::UNKNOWN_OBJECT,
            },
        ];
    }
    write_common(&mut object, configured);
    object
}

#[test]
fn rpm_all_global_group_serves_the_array_productions() {
    let object = global_group(true);
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    let mut request = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: vec![PropertyReference {
                property_identifier: P::ALL,
                property_array_index: None,
            }],
        }],
    }
    .encode(&mut request)
    .unwrap();
    let mut ack = BytesMut::new();
    handle_read_property_multiple(&db, &request, &mut ack).unwrap();
    let ack = ReadPropertyMultipleACK::decode(&ack).unwrap();
    let results = &ack.list_of_read_access_results[0].list_of_results;
    let value = |p: P| {
        let result = results
            .iter()
            .find(|result| result.property_identifier == p)
            .unwrap_or_else(|| panic!("{p:?} missing from RPM ALL"));
        assert_eq!(result.property_array_index, None);
        result.property_value.as_deref().unwrap()
    };
    assert_eq!(value(P::GROUP_MEMBERS), GG_MEMBERS);
    assert_eq!(value(P::PRESENT_VALUE), GG_PRESENT_VALUE);
    assert_eq!(
        value(P::GROUP_MEMBER_NAMES),
        &[0x72, 0x00, b'a', 0x72, 0x00, b'b']
    );
    // No member references Status_Flags, so Member_Status_Flags is clear.
    assert_eq!(value(P::MEMBER_STATUS_FLAGS), &[0x82, 4, 0]);
}

#[test]
fn rp_and_rpm_global_group_member_status_flags_combine_status_flags_members() {
    // Two members reference Status_Flags and one Present_Value; only the two
    // Status_Flags values combine (Clause 12.50.10, #1092).
    let mut object = GlobalGroupObject::new(7, "GG-7").unwrap();
    for (instance, property) in [
        (1, P::STATUS_FLAGS),
        (2, P::PRESENT_VALUE),
        (3, P::STATUS_FLAGS),
    ] {
        object
            .add_group_member(BACnetDeviceObjectPropertyReference {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance)
                    .unwrap(),
                property_identifier: property.to_raw(),
                property_array_index: None,
                device_identifier: None,
            })
            .unwrap();
    }
    let flags = |octet| PropertyValue::BitString {
        unused_bits: 4,
        data: vec![octet],
    };
    // IN_ALARM on the first member, OVERRIDDEN on the Present_Value member
    // (ignored) and FAULT on the third.
    object.present_value = [0x80, 0x20, 0x40]
        .map(|octet| AccessResult::Value(flags(octet)))
        .to_vec();
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    let cases: &[(P, Option<u32>, ExpectedRead)] = &[
        (P::MEMBER_STATUS_FLAGS, None, Ok(&[0x82, 4, 0xC0])),
        (P::EVENT_STATE, None, Ok(&[0x91, 0])),
        // The group's own IN_ALARM follows its NORMAL Event_State.
        (P::STATUS_FLAGS, None, Ok(&[0x82, 4, 0])),
    ];
    assert_cases(&db, oid, cases);
}

/// The configured Structured View's Subordinate_List elements, each a
/// BACnetDeviceObjectReference (Table 12-34): the object under [1], after
/// the device under [0] for a subordinate in another device.
const SV_SUBORDINATE_1: &[u8] = &[0x1C, 0, 0, 0, 1];
const SV_SUBORDINATE_2: &[u8] = &[0x0C, 0x02, 0, 0, 9, 0x1C, 0, 0xC0, 0, 1];
const SV_SUBORDINATE_3: &[u8] = &[0x1C, 0, 0x80, 0, 2];
const SV_SUBORDINATES: &[u8] = &[
    0x1C, 0, 0, 0, 1, 0x0C, 0x02, 0, 0, 9, 0x1C, 0, 0xC0, 0, 1, 0x1C, 0, 0x80, 0, 2,
];
const SV_ANNOTATIONS: &[u8] = &[0x72, 0, b'a', 0x72, 0, b'b', 0x72, 0, b'c'];

/// Index 0 is the size, 1..=N one element, and past N is
/// INVALID_ARRAY_INDEX; no index is the elements back to back.
fn array_cases(
    property: P,
    whole: &'static [u8],
    elements: &[&'static [u8]],
) -> Vec<(P, Option<u32>, ExpectedRead)> {
    let size: &'static [u8] = match elements.len() {
        0 => &[0x21, 0],
        3 => &[0x21, 3],
        n => panic!("no size octets for {n} elements"),
    };
    let mut cases = vec![(property, None, Ok(whole)), (property, Some(0), Ok(size))];
    cases.extend(
        (1..)
            .zip(elements)
            .map(|(i, &e)| (property, Some(i), Ok(e))),
    );
    for index in [elements.len() as u32 + 1, u32::MAX] {
        cases.push((property, Some(index), Err(ErrorCode::INVALID_ARRAY_INDEX)));
    }
    cases
}

#[test]
fn rpm_structured_view_arrays_serve_one_element_per_index() {
    assert_eq!(
        SV_SUBORDINATES,
        [SV_SUBORDINATE_1, SV_SUBORDINATE_2, SV_SUBORDINATE_3].concat()
    );
    for configured in [false, true] {
        let mut object = StructuredViewObject::new(7, "SV-7").unwrap();
        if configured {
            let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
            let bi1 = ObjectIdentifier::new(ObjectType::BINARY_INPUT, 1).unwrap();
            let av2 = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 2).unwrap();
            object.add_subordinate(ai1, "a").unwrap();
            object
                .add_subordinate(
                    bacnet_types::constructed::BACnetDeviceObjectReference {
                        device_identifier: Some(
                            ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap(),
                        ),
                        object_identifier: bi1,
                    },
                    "b",
                )
                .unwrap();
            object.add_subordinate(av2, "c").unwrap();
        }
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the projection.
        // Subordinate_List and Subordinate_Annotations are BACnetARRAY
        // (Table 12-34); the scalar Node_Type/Node_Subtype reject an index.
        let mut cases = if configured {
            let mut cases = array_cases(
                P::SUBORDINATE_LIST,
                SV_SUBORDINATES,
                &[SV_SUBORDINATE_1, SV_SUBORDINATE_2, SV_SUBORDINATE_3],
            );
            cases.extend(array_cases(
                P::SUBORDINATE_ANNOTATIONS,
                SV_ANNOTATIONS,
                &[&[0x72, 0, b'a'], &[0x72, 0, b'b'], &[0x72, 0, b'c']],
            ));
            cases
        } else {
            let mut cases = array_cases(P::SUBORDINATE_LIST, EMPTY, &[]);
            cases.extend(array_cases(P::SUBORDINATE_ANNOTATIONS, EMPTY, &[]));
            cases
        };
        cases.extend_from_slice(&[
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
            // Table 12-34 has no Status_Flags, Out_Of_Service or Reliability
            // (#1064).
            (P::STATUS_FLAGS, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::STATUS_FLAGS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::OUT_OF_SERVICE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::OUT_OF_SERVICE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::RELIABILITY, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::RELIABILITY,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[0x91, 28, 0x91, 208, 0x91, 207, 0x91, 211, 0x91, 210]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 5])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 208])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 207])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x91, 211])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x91, 210])),
            (
                P::PROPERTY_LIST,
                Some(6),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // Unserved StructuredView table rows stay unknown.
            // An array this object doesn't hold: absent, indexed or not.
            (P::SUBORDINATE_TAGS, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::SUBORDINATE_TAGS,
                Some(1),
                Err(ErrorCode::UNKNOWN_PROPERTY),
            ),
        ]);
        assert_cases(&db, oid, &cases);
    }
}
