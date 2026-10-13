//! Access Point Authentication_Policy_List and Authentication_Policy_Names
//! over ReadProperty and ReadPropertyMultiple (#1325): absent until the
//! application sets them, then two arrays as long as the policy count.

use super::*;
use bacnet_types::constructed::{BACnetAuthenticationPolicy, BACnetAuthenticationPolicyEntry};

/// A card read by Credential Data Input 1, in order, within 30 seconds.
fn card() -> BACnetAuthenticationPolicy {
    BACnetAuthenticationPolicy {
        policy: vec![BACnetAuthenticationPolicyEntry {
            credential_data_input: ObjectIdentifier::new(ObjectType::CREDENTIAL_DATA_INPUT, 1)
                .unwrap()
                .into(),
            index: 1,
        }],
        order_enforced: true,
        timeout: 30,
    }
}

#[test]
fn rpm_access_point_policy_arrays_follow_the_application() {
    // policy [0] { credential-data-input [0] { object [1] CDI 1 } index [1] 1 }
    // order-enforced [1] TRUE, timeout [2] 30.
    const CARD: &[u8] = &[
        0x0E, 0x0E, 0x1C, 0x09, 0x40, 0x00, 0x01, 0x0F, 0x19, 0x01, 0x0F, 0x19, 0x01, 0x29, 0x1E,
    ];
    // The empty policy a grown array adds: invalid, so never in effect.
    const EMPTY_POLICY: &[u8] = &[0x0E, 0x0F, 0x19, 0x00, 0x29, 0x00];
    const CARD_NAME: &[u8] = &[0x75, 0x05, 0x00, b'c', b'a', b'r', b'd'];
    const NO_NAME: &[u8] = &[0x71, 0x00];
    for configured in [false, true] {
        let mut object = AccessPointObject::new(7, "AP-7").unwrap();
        if configured {
            object
                .set_authentication_policies([("card", card())])
                .unwrap();
            object.set_number_of_authentication_policies(2).unwrap();
        }
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        let cases: &[(P, Option<u32>, ExpectedRead)] = if configured {
            &[
                (P::NUMBER_OF_AUTHENTICATION_POLICIES, None, Ok(&[0x21, 2])),
                (P::ACTIVE_AUTHENTICATION_POLICY, None, Ok(&[0x21, 1])),
                // The empty policy the count added is a configuration
                // error (Clause 12.31.12).
                (P::RELIABILITY, None, Ok(&[0x91, 10])),
                (
                    P::AUTHENTICATION_POLICY_LIST,
                    None,
                    Ok(&[
                        0x0E, 0x0E, 0x1C, 0x09, 0x40, 0x00, 0x01, 0x0F, 0x19, 0x01, 0x0F, 0x19,
                        0x01, 0x29, 0x1E, 0x0E, 0x0F, 0x19, 0x00, 0x29, 0x00,
                    ]),
                ),
                (P::AUTHENTICATION_POLICY_LIST, Some(0), Ok(&[0x21, 2])),
                (P::AUTHENTICATION_POLICY_LIST, Some(1), Ok(CARD)),
                (P::AUTHENTICATION_POLICY_LIST, Some(2), Ok(EMPTY_POLICY)),
                (
                    P::AUTHENTICATION_POLICY_LIST,
                    Some(3),
                    Err(ErrorCode::INVALID_ARRAY_INDEX),
                ),
                (
                    P::AUTHENTICATION_POLICY_NAMES,
                    None,
                    Ok(&[0x75, 0x05, 0x00, b'c', b'a', b'r', b'd', 0x71, 0x00]),
                ),
                (P::AUTHENTICATION_POLICY_NAMES, Some(0), Ok(&[0x21, 2])),
                (P::AUTHENTICATION_POLICY_NAMES, Some(1), Ok(CARD_NAME)),
                (P::AUTHENTICATION_POLICY_NAMES, Some(2), Ok(NO_NAME)),
                (
                    P::AUTHENTICATION_POLICY_NAMES,
                    Some(u32::MAX),
                    Err(ErrorCode::INVALID_ARRAY_INDEX),
                ),
                // The two rows (258 and 259) close the list, before
                // Property_List.
                (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 18])),
                (P::PROPERTY_LIST, Some(17), Ok(&[0x92, 0x01, 0x02])),
                (P::PROPERTY_LIST, Some(18), Ok(&[0x92, 0x01, 0x03])),
            ]
        } else {
            &[
                (P::NUMBER_OF_AUTHENTICATION_POLICIES, None, Ok(&[0x21, 1])),
                (
                    P::AUTHENTICATION_POLICY_LIST,
                    None,
                    Err(ErrorCode::UNKNOWN_PROPERTY),
                ),
                (
                    P::AUTHENTICATION_POLICY_LIST,
                    Some(0),
                    Err(ErrorCode::UNKNOWN_PROPERTY),
                ),
                (
                    P::AUTHENTICATION_POLICY_NAMES,
                    None,
                    Err(ErrorCode::UNKNOWN_PROPERTY),
                ),
                (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 16])),
            ]
        };
        assert_cases(&db, oid, cases);
    }
}
