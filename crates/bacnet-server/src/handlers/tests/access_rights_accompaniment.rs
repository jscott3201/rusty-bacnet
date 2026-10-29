//! Access Rights Accompaniment over the wire (#1393). Left out until the
//! application sets it: ReadProperty and WriteProperty get UNKNOWN_PROPERTY,
//! and Property_List, ReadPropertyMultiple ALL and OPTIONAL leave it out.
//! Once set it is served as a BACnetDeviceObjectReference and takes
//! WriteProperty and WritePropertyMultiple, with the setter's checks; a
//! refused write leaves it as ReadProperty and ReadPropertyMultiple read it
//! before.

use super::access_control_arrays::{Expected, assert_reads, db_with};
use super::property_metadata::assert_rpm_selector_bytes;
use super::*;
use PropertyIdentifier as P;
use bacnet_objects::access_control::AccessRightsObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::error::RemoteError;

/// Access User 3 in this device: object [1] alone.
const USER: &[u8] = &[0x1C, 0x08, 0xC0, 0x00, 0x03];
/// Access Credential 5 in Device 99: device [0], then object [1].
const REMOTE_CREDENTIAL: &[u8] = &[0x0C, 0x02, 0x00, 0x00, 0x63, 0x1C, 0x08, 0x00, 0x00, 0x05];
/// Property_List without Accompaniment: Description, Global_Identifier, the
/// two rule arrays, Status_Flags, Reliability and Enable.
const BASE_LIST: &[u8] = &[
    0x91, 28, 0x92, 0x01, 0x43, 0x92, 0x01, 0x2E, 0x92, 0x01, 0x20, 0x91, 111, 0x91, 103, 0x91, 133,
];
/// Accompaniment, property 252, as a Property_List element.
const ACCOMPANIMENT_ENTRY: &[u8] = &[0x91, 252];

/// The rows ReadPropertyMultiple ALL expands to, before Accompaniment.
const BASE_ALL: [P; 10] = [
    P::OBJECT_IDENTIFIER,
    P::OBJECT_NAME,
    P::DESCRIPTION,
    P::OBJECT_TYPE,
    P::GLOBAL_IDENTIFIER,
    P::POSITIVE_ACCESS_RULES,
    P::NEGATIVE_ACCESS_RULES,
    P::STATUS_FLAGS,
    P::RELIABILITY,
    P::LOG_ENABLE,
];

/// Access Rights 7 with Accompaniment set to Access User 3.
fn with_accompaniment() -> AccessRightsObject {
    let mut rights = AccessRightsObject::new(7, "AR-7").unwrap();
    let user = ObjectIdentifier::new(ObjectType::ACCESS_USER, 3).unwrap();
    rights.set_accompaniment(Some(user.into())).unwrap();
    rights
}

fn write(
    db: &mut ObjectDatabase,
    rights: ObjectIdentifier,
    index: Option<u32>,
    property_value: Vec<u8>,
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: rights,
        property_identifier: P::ACCOMPANIMENT,
        property_array_index: index,
        property_value,
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).map(|_| ())
}

fn write_multiple(
    db: &mut ObjectDatabase,
    rights: ObjectIdentifier,
    writes: Vec<(P, Vec<u8>)>,
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: rights,
            list_of_properties: writes
                .into_iter()
                .map(|(property, value)| BACnetPropertyValue {
                    property_identifier: property,
                    property_array_index: None,
                    value,
                    priority: None,
                })
                .collect(),
        }],
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property_multiple(db, &request).map(|_| ())
}

fn assert_refused(result: Result<(), Error>, code: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class, code: actual }))
            if class == ErrorClass::PROPERTY
                && actual == code),
        "expected PROPERTY / {code:?}, got {result:?}"
    );
}

/// Accompaniment read as `octets`.
fn served(octets: &[u8]) -> Vec<(P, Option<u32>, Expected)> {
    vec![(P::ACCOMPANIMENT, None, Ok(octets.to_vec()))]
}

#[test]
fn accompaniment_is_absent_until_the_application_sets_it() {
    let (mut db, rights) = db_with(Box::new(AccessRightsObject::new(7, "AR-7").unwrap()));
    assert_reads(
        &db,
        rights,
        &[
            (P::ACCOMPANIMENT, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::PROPERTY_LIST, None, Ok(BASE_LIST.to_vec())),
            (P::PROPERTY_LIST, Some(0), Ok(vec![0x21, 7])),
        ],
    );
    assert_rpm_selector_bytes(&db, rights, P::ALL, &BASE_ALL);
    assert_rpm_selector_bytes(&db, rights, P::OPTIONAL, &[P::DESCRIPTION]);
    // A write can't add the row, whole or indexed, nor can a WPM.
    for index in [None, Some(1)] {
        assert_refused(
            write(&mut db, rights, index, REMOTE_CREDENTIAL.to_vec()),
            ErrorCode::UNKNOWN_PROPERTY,
        );
    }
    assert_refused(
        write_multiple(
            &mut db,
            rights,
            vec![(P::ACCOMPANIMENT, REMOTE_CREDENTIAL.to_vec())],
        ),
        ErrorCode::UNKNOWN_PROPERTY,
    );
    assert_reads(
        &db,
        rights,
        &[(P::ACCOMPANIMENT, None, Err(ErrorCode::UNKNOWN_PROPERTY))],
    );
}

#[test]
fn a_configured_accompaniment_is_served_listed_and_written() {
    let (mut db, rights) = db_with(Box::new(with_accompaniment()));
    let list = [BASE_LIST, ACCOMPANIMENT_ENTRY].concat();
    let mut cases = served(USER);
    cases.extend([
        (P::PROPERTY_LIST, None, Ok(list)),
        (P::PROPERTY_LIST, Some(0), Ok(vec![0x21, 8])),
        (P::PROPERTY_LIST, Some(8), Ok(ACCOMPANIMENT_ENTRY.to_vec())),
    ]);
    assert_reads(&db, rights, &cases);
    let all = [&BASE_ALL[..], &[P::ACCOMPANIMENT]].concat();
    assert_rpm_selector_bytes(&db, rights, P::ALL, &all);
    assert_rpm_selector_bytes(
        &db,
        rights,
        P::OPTIONAL,
        &[P::DESCRIPTION, P::ACCOMPANIMENT],
    );
    let required: Vec<P> = BASE_ALL
        .into_iter()
        .filter(|&p| p != P::DESCRIPTION)
        .collect();
    assert_rpm_selector_bytes(&db, rights, P::REQUIRED, &required);

    // WriteProperty, then a WritePropertyMultiple alongside Enable.
    write(&mut db, rights, None, REMOTE_CREDENTIAL.to_vec()).unwrap();
    assert_reads(&db, rights, &served(REMOTE_CREDENTIAL));
    write_multiple(
        &mut db,
        rights,
        vec![
            (P::ACCOMPANIMENT, USER.to_vec()),
            (P::LOG_ENABLE, vec![0x10]),
        ],
    )
    .unwrap();
    let mut cases = served(USER);
    cases.push((P::LOG_ENABLE, None, Ok(vec![0x10])));
    assert_reads(&db, rights, &cases);
    // An unspecified Access Credential asks for no accompaniment; the row
    // stays.
    let unspecified = [0x1C, 0x08, 0x3F, 0xFF, 0xFF];
    write(&mut db, rights, None, unspecified.to_vec()).unwrap();
    assert_reads(&db, rights, &served(&unspecified));
}

#[test]
fn refused_accompaniment_writes_leave_it_unchanged() {
    let (mut db, rights) = db_with(Box::new(with_accompaniment()));
    let cases: [(Option<u32>, Vec<u8>, ErrorCode); 8] = [
        // An Access Point: no object Clause 12.34.11 gives a meaning to.
        (
            None,
            vec![0x1C, 0x08, 0x40, 0x00, 0x01],
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        // A "device" that is Analog Value 99 (#1285).
        (
            None,
            vec![0x0C, 0x00, 0x80, 0x00, 0x63, 0x1C, 0x08, 0x00, 0x00, 0x05],
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        // An application Unsigned is no reference.
        (None, vec![0x21, 0x05], ErrorCode::INVALID_DATA_TYPE),
        // A device with no object, two references, and nothing at all.
        (
            None,
            REMOTE_CREDENTIAL[..5].to_vec(),
            ErrorCode::INVALID_DATA_ENCODING,
        ),
        (
            None,
            [REMOTE_CREDENTIAL, USER].concat(),
            ErrorCode::INVALID_DATA_ENCODING,
        ),
        (None, Vec::new(), ErrorCode::INVALID_DATA_ENCODING),
        // No array, so no index.
        (
            Some(1),
            REMOTE_CREDENTIAL.to_vec(),
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
        (
            Some(0),
            vec![0x21, 0x01],
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
    ];
    for (index, octets, code) in cases {
        assert_refused(write(&mut db, rights, index, octets.clone()), code);
        assert_reads(&db, rights, &served(USER));
    }
    // A WritePropertyMultiple stops at the refused element: Enable, before
    // it, is written, and Accompaniment stays.
    assert_refused(
        write_multiple(
            &mut db,
            rights,
            vec![
                (P::LOG_ENABLE, vec![0x10]),
                (P::ACCOMPANIMENT, vec![0x1C, 0x08, 0x40, 0x00, 0x01]),
            ],
        ),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    let mut cases = served(USER);
    cases.push((P::LOG_ENABLE, None, Ok(vec![0x10])));
    assert_reads(&db, rights, &cases);
    // NULL is no value of a BACnetDeviceObjectReference, so the write
    // succeeds and changes nothing (#1396).
    write(&mut db, rights, None, vec![0x00]).unwrap();
    assert_reads(&db, rights, &served(USER));
}
