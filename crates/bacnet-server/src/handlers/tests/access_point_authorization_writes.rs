//! Access Point Active_Authentication_Policy and Authorization_Mode over
//! WriteProperty and WritePropertyMultiple, and the read-only
//! Number_Of_Authentication_Policies and Priority_For_Writing (Clauses
//! 12.31.10, 12.31.11, 12.31.14 and 12.31.33; #1307).

use super::*;
use PropertyIdentifier as P;
use bacnet_objects::access_control::AccessPointObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::enums::AuthorizationMode;
use bacnet_types::error::RemoteError;

/// One write: the property, its array index, the value and the priority.
type Write = (P, Option<u32>, PropertyValue, Option<u8>);

/// A new point, or with `configured` one defining three policies and
/// declaring GRANT_ACTIVE, DENY_ALL and NONE beside AUTHORIZE.
fn point_db(configured: bool) -> (ObjectDatabase, ObjectIdentifier) {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    if configured {
        point.set_number_of_authentication_policies(3).unwrap();
        point
            .set_supported_authorization_modes([
                AuthorizationMode::AUTHORIZE,
                AuthorizationMode::GRANT_ACTIVE,
                AuthorizationMode::DENY_ALL,
                AuthorizationMode::NONE,
            ])
            .unwrap();
    }
    let oid = point.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(point)).unwrap();
    (db, oid)
}

fn encode(value: &PropertyValue) -> Vec<u8> {
    let mut buf = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut buf, value).unwrap();
    buf.to_vec()
}

fn write_property(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    write: Write,
) -> Result<(), Error> {
    let (property, index, value, priority) = write;
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: index,
        property_value: encode(&value),
        priority,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).map(|_| ())
}

fn write_property_multiple(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    writes: Vec<Write>,
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: writes
                .into_iter()
                .map(|(property, index, value, priority)| BACnetPropertyValue {
                    property_identifier: property,
                    property_array_index: index,
                    value: encode(&value),
                    priority,
                })
                .collect(),
        }],
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property_multiple(db, &request).map(|_| ())
}

/// A write with no index and no priority.
fn plain(property: P, value: PropertyValue) -> Write {
    (property, None, value, None)
}

fn policy(policy: u64) -> Write {
    plain(
        P::ACTIVE_AUTHENTICATION_POLICY,
        PropertyValue::Unsigned(policy),
    )
}

fn mode(mode: AuthorizationMode) -> Write {
    plain(
        P::AUTHORIZATION_MODE,
        PropertyValue::Enumerated(mode.to_raw()),
    )
}

/// The ReadProperty-ACK value bytes of one property.
fn read_bytes(db: &ObjectDatabase, oid: ObjectIdentifier, property: P) -> Vec<u8> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut request);
    let mut response = BytesMut::new();
    handle_read_property(db, &request, &mut response).unwrap();
    ReadPropertyACK::decode(&response).unwrap().property_value
}

/// Active_Authentication_Policy, Number_Of_Authentication_Policies,
/// Authorization_Mode and Priority_For_Writing as served.
fn served(db: &ObjectDatabase, oid: ObjectIdentifier) -> [Vec<u8>; 4] {
    [
        P::ACTIVE_AUTHENTICATION_POLICY,
        P::NUMBER_OF_AUTHENTICATION_POLICIES,
        P::AUTHORIZATION_MODE,
        P::PRIORITY_FOR_WRITING,
    ]
    .map(|property| read_bytes(db, oid, property))
}

fn assert_property_error(result: Result<(), Error>, expected: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::PROPERTY
                && code == expected),
        "expected PROPERTY / {expected:?}, got {result:?}"
    );
}

#[test]
fn write_property_sets_the_access_point_policy_and_mode() {
    let (db, oid) = point_db(false);
    // One policy in effect, AUTHORIZE (0) and priority 16.
    assert_eq!(
        served(&db, oid),
        [vec![0x21, 1], vec![0x21, 1], vec![0x91, 0], vec![0x21, 16]]
    );

    let (mut db, oid) = point_db(true);
    write_property(&mut db, oid, policy(3)).unwrap();
    // DENY_ALL (2), then NONE (5).
    for (written, wire) in [
        (AuthorizationMode::DENY_ALL, 2),
        (AuthorizationMode::NONE, 5),
    ] {
        write_property(&mut db, oid, mode(written)).unwrap();
        assert_eq!(
            read_bytes(&db, oid, P::AUTHORIZATION_MODE),
            vec![0x91, wire]
        );
    }
    // Neither row is commandable, so a priority is taken and ignored: a
    // later write at a lower priority still replaces the value.
    for (written, priority) in [
        (AuthorizationMode::DENY_ALL, 8),
        (AuthorizationMode::AUTHORIZE, 16),
    ] {
        let (property, index, value, _) = mode(written);
        write_property(&mut db, oid, (property, index, value, Some(priority))).unwrap();
    }
    let (property, index, value, _) = policy(2);
    write_property(&mut db, oid, (property, index, value, Some(8))).unwrap();
    assert_eq!(
        served(&db, oid),
        [vec![0x21, 2], vec![0x21, 3], vec![0x91, 0], vec![0x21, 16]]
    );
    // Both in one WritePropertyMultiple, the mode with a priority:
    // policy 1 and GRANT_ACTIVE (1).
    let (property, index, value, _) = mode(AuthorizationMode::GRANT_ACTIVE);
    write_property_multiple(
        &mut db,
        oid,
        vec![policy(1), (property, index, value, Some(8))],
    )
    .unwrap();
    assert_eq!(
        served(&db, oid),
        [vec![0x21, 1], vec![0x21, 3], vec![0x91, 1], vec![0x21, 16]]
    );
}

#[test]
fn write_property_refuses_other_policy_mode_and_read_only_writes() {
    let (mut db, oid) = point_db(false);
    let unchanged = served(&db, oid);
    let null = || PropertyValue::Null;
    let refusals = [
        // Zero and 2 name no policy of the one a new point defines.
        (policy(0), ErrorCode::VALUE_OUT_OF_RANGE),
        (policy(2), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            plain(
                P::ACTIVE_AUTHENTICATION_POLICY,
                PropertyValue::Enumerated(1),
            ),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            (
                P::ACTIVE_AUTHENTICATION_POLICY,
                Some(0),
                PropertyValue::Unsigned(1),
                None,
            ),
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
        // A new point supports AUTHORIZE alone, so DENY_ALL is refused like
        // a reserved mode or an undeclared proprietary one.
        (
            mode(AuthorizationMode::DENY_ALL),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            mode(AuthorizationMode::from_raw(6)),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            mode(AuthorizationMode::from_raw(64)),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            plain(P::AUTHORIZATION_MODE, PropertyValue::Unsigned(0)),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            (
                P::AUTHORIZATION_MODE,
                Some(1),
                PropertyValue::Enumerated(0),
                None,
            ),
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
        // The application's rows.
        (
            plain(
                P::NUMBER_OF_AUTHENTICATION_POLICIES,
                PropertyValue::Unsigned(2),
            ),
            ErrorCode::WRITE_ACCESS_DENIED,
        ),
        (
            plain(P::PRIORITY_FOR_WRITING, PropertyValue::Unsigned(8)),
            ErrorCode::WRITE_ACCESS_DENIED,
        ),
        (
            plain(P::PRIORITY_FOR_WRITING, null()),
            ErrorCode::WRITE_ACCESS_DENIED,
        ),
    ];
    for (write, expected) in refusals {
        let property = write.0;
        assert_property_error(write_property(&mut db, oid, write), expected);
        assert_eq!(served(&db, oid), unchanged, "{property:?}");
    }
    // Neither writable row is commandable or takes a NULL, so a NULL, with
    // or without a priority, succeeds and changes nothing (#1396).
    for write in [
        plain(P::ACTIVE_AUTHENTICATION_POLICY, null()),
        (P::ACTIVE_AUTHENTICATION_POLICY, None, null(), Some(8)),
        plain(P::AUTHORIZATION_MODE, null()),
    ] {
        let property = write.0;
        write_property(&mut db, oid, write).unwrap();
        assert_eq!(served(&db, oid), unchanged, "{property:?}");
    }
}

#[test]
fn write_property_multiple_refuses_other_policy_mode_values() {
    let (mut db, oid) = point_db(true);
    write_property_multiple(&mut db, oid, vec![mode(AuthorizationMode::DENY_ALL)]).unwrap();
    let mode_bytes = |db: &ObjectDatabase| read_bytes(db, oid, P::AUTHORIZATION_MODE);
    // Each request stops at its refused write; the writes before it stand.
    let refusals = [
        // Another datatype.
        (
            plain(P::AUTHORIZATION_MODE, PropertyValue::Unsigned(1)),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        // An array index on either row.
        (
            (
                P::AUTHORIZATION_MODE,
                Some(0),
                PropertyValue::Enumerated(0),
                None,
            ),
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
        (
            (
                P::ACTIVE_AUTHENTICATION_POLICY,
                Some(1),
                PropertyValue::Unsigned(1),
                None,
            ),
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
        // Past the policy count, and a mode the point didn't declare.
        (policy(4), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            mode(AuthorizationMode::VERIFICATION_REQUIRED),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        // The application's rows.
        (
            plain(P::PRIORITY_FOR_WRITING, PropertyValue::Unsigned(8)),
            ErrorCode::WRITE_ACCESS_DENIED,
        ),
    ];
    for (refused, expected) in refusals {
        let property = refused.0;
        write_property_multiple(&mut db, oid, vec![mode(AuthorizationMode::DENY_ALL)]).unwrap();
        assert_property_error(
            write_property_multiple(&mut db, oid, vec![mode(AuthorizationMode::NONE), refused]),
            expected,
        );
        assert_eq!(mode_bytes(&db), vec![0x91, 5], "{property:?}");
    }
    assert_eq!(
        served(&db, oid),
        [vec![0x21, 1], vec![0x21, 3], vec![0x91, 5], vec![0x21, 16]]
    );
    // A NULL leaves the policy as it is and the request goes on to the
    // mode (#1396).
    write_property_multiple(&mut db, oid, vec![mode(AuthorizationMode::DENY_ALL)]).unwrap();
    write_property_multiple(
        &mut db,
        oid,
        vec![
            plain(P::ACTIVE_AUTHENTICATION_POLICY, PropertyValue::Null),
            mode(AuthorizationMode::NONE),
        ],
    )
    .unwrap();
    assert_eq!(
        served(&db, oid),
        [vec![0x21, 1], vec![0x21, 3], vec![0x91, 5], vec![0x21, 16]]
    );
}
