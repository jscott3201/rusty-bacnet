use super::*;
use PropertyIdentifier as P;
use bacnet_objects::{command::CommandObject, traits::BACnetObject};
use bacnet_types::constructed::{
    BACnetActionCommand, BACnetActionList, PropertyReference, ReadAccessSpecification,
};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::PropertyValue;

/// The three Action elements the configured fixture holds, each one
/// BACnetActionList framed in its own [0] pair (Table 12-12, Clause 21).
/// Written out from the production tags, not produced by the codec.
const ACTION_1: &[u8] = &[
    0x0E, // [0] open
    0x1C, 0x00, 0x40, 0x00, 0x01, // [1] AO-1
    0x29, 0x55, // [2] Present_Value
    0x4E, 0x44, 0x42, 0x48, 0x00, 0x00, 0x4F, // [4] REAL 50.0
    0x59, 0x08, // [5] priority 8
    0x79, 0x00, // [7] quit-on-failure FALSE
    0x89, 0x01, // [8] write-successful TRUE
    0x0F, // [0] close
];
/// An empty action list: Present_Value 2 writes nothing.
const ACTION_2: &[u8] = &[0x0E, 0x0F];
const ACTION_3: &[u8] = &[
    0x0E, // [0] open
    0x0C, 0x02, 0x00, 0x00, 0x09, // [0] Device 9
    0x1C, 0x01, 0x40, 0x00, 0x03, // [1] BV-3
    0x29, 0x55, // [2] Present_Value
    0x4E, 0x91, 0x01, 0x4F, // [4] ENUMERATED 1
    0x6A, 0x01, 0x2C, // [6] post-delay 300
    0x79, 0x01, // [7] TRUE
    0x89, 0x00, // [8] FALSE
    0x0F, // [0] close
];

fn action_lists() -> Vec<BACnetActionList> {
    let ao1 = BACnetActionCommand {
        device_identifier: None,
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
        property_identifier: P::PRESENT_VALUE,
        property_array_index: None,
        property_value: PropertyValue::Real(50.0),
        priority: Some(8),
        post_delay: None,
        quit_on_failure: false,
        write_successful: true,
    };
    let bv3 = BACnetActionCommand {
        device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap()),
        object_identifier: ObjectIdentifier::new(ObjectType::BINARY_VALUE, 3).unwrap(),
        property_value: PropertyValue::Enumerated(1),
        priority: None,
        post_delay: Some(300),
        quit_on_failure: true,
        write_successful: false,
        ..ao1.clone()
    };
    vec![
        BACnetActionList {
            commands: vec![ao1],
        },
        BACnetActionList::default(),
        BACnetActionList {
            commands: vec![bv3],
        },
    ]
}

#[test]
fn rpm_command_action_serves_one_action_list_per_index() {
    let whole_action = [ACTION_1, ACTION_2, ACTION_3].concat();
    for configured in [false, true] {
        let mut object = CommandObject::new(7, "CMD-7").unwrap();
        if configured {
            object
                .write_property(
                    P::DESCRIPTION,
                    None,
                    bacnet_types::primitives::PropertyValue::CharacterString(
                        "long command label".repeat(100),
                    ),
                    None,
                )
                .unwrap();
            object.set_action(action_lists()).unwrap();
            // List 2 is empty, so the write completes at once.
            object
                .write_property(
                    P::PRESENT_VALUE,
                    None,
                    bacnet_types::primitives::PropertyValue::Unsigned(2),
                    None,
                )
                .unwrap();
        }
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the projection.
        type ExpectedRead<'a> = Result<&'a [u8], ErrorCode>;
        // Action is a BACnetARRAY (Table 12-12): index 0 reads the size, 1..=N
        // one action list, and past N is INVALID_ARRAY_INDEX. The whole array
        // is the elements' own octets back to back.
        let action: &[(Option<u32>, ExpectedRead)] = if configured {
            &[
                (None, Ok(&whole_action)),
                (Some(0), Ok(&[0x21, 3])),
                (Some(1), Ok(ACTION_1)),
                (Some(2), Ok(ACTION_2)),
                (Some(3), Ok(ACTION_3)),
                (Some(4), Err(ErrorCode::INVALID_ARRAY_INDEX)),
                (Some(u32::MAX), Err(ErrorCode::INVALID_ARRAY_INDEX)),
            ]
        } else {
            &[
                (None, Ok(&[])),
                (Some(0), Ok(&[0x21, 0])),
                (Some(1), Err(ErrorCode::INVALID_ARRAY_INDEX)),
                (Some(u32::MAX), Err(ErrorCode::INVALID_ARRAY_INDEX)),
            ]
        };
        let mut cases: Vec<(P, Option<u32>, ExpectedRead)> = action
            .iter()
            .map(|&(index, expected)| (P::ACTION, index, expected))
            .collect();
        cases.extend_from_slice(&[
            (
                P::PRESENT_VALUE,
                None,
                Ok(if configured { &[0x21, 2] } else { &[0x21, 0] }),
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
            (P::IN_PROCESS, None, Ok(&[0x10])),
            (
                P::IN_PROCESS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::IN_PROCESS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::ALL_WRITES_SUCCESSFUL, None, Ok(&[0x11])),
            (
                P::ALL_WRITES_SUCCESSFUL,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            // Table 12-12 has no Out_Of_Service (#1064), so the flag stays
            // clear.
            (P::STATUS_FLAGS, None, Ok(&[0x82, 4, 0])),
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
                    0x91, 28, 0x91, 85, 0x91, 47, 0x91, 9, 0x91, 2, 0x91, 111, 0x91, 103,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 7])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 85])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 47])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x91, 9])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x91, 2])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x91, 111])),
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
            // Action_Text is an array on Command, absent until configured.
            (P::ACTION_TEXT, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::ACTION_TEXT, Some(1), Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::EVENT_STATE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::EVENT_STATE,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
        ]);
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
        for (result, &(p, i, expected)) in access.list_of_results.iter().zip(&cases) {
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
