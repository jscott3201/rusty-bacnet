//! The access-control BACnetARRAY rows over ReadProperty and
//! ReadPropertyMultiple (#1169): Credential Data Input Supported_Formats and
//! Supported_Format_Classes (Table 12-43), Access Door Door_Members
//! (Table 12-30) and Access Point Access_Doors (Table 12-36).
//!
//! For each array the two services agree on the whole array, its size at
//! index 0, the first and last elements and INVALID_ARRAY_INDEX past the end,
//! filled and empty.

use super::*;
use bacnet_objects::access_control::{
    AccessDoorObject, AccessPointObject, CredentialDataInputObject,
};
use bacnet_types::constructed::{
    BACnetAuthenticationFactorFormat, BACnetDeviceObjectReference, PropertyReference,
    ReadAccessSpecification,
};
use bacnet_types::enums::AuthenticationFactorType;
use PropertyIdentifier as P;

pub(super) type Expected = Result<Vec<u8>, ErrorCode>;

/// Read each `(property, index)` of `oid` with ReadProperty and with one
/// ReadPropertyMultiple, and check both against the expected bytes or error.
pub(super) fn assert_reads(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    cases: &[(P, Option<u32>, Expected)],
) {
    let mut request = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: cases
                .iter()
                .map(
                    |&(property_identifier, property_array_index, _)| PropertyReference {
                        property_identifier,
                        property_array_index,
                    },
                )
                .collect(),
        }],
    }
    .encode(&mut request)
    .unwrap();
    let mut response = BytesMut::new();
    handle_read_property_multiple(db, &request, &mut response).unwrap();
    let ack = ReadPropertyMultipleACK::decode(&response).unwrap();
    let results = &ack.list_of_read_access_results[0].list_of_results;
    assert_eq!(results.len(), cases.len());
    for (result, (property, index, expected)) in results.iter().zip(cases) {
        assert_eq!(result.property_identifier, *property);
        assert_eq!(result.property_array_index, *index);
        let mut rp_request = BytesMut::new();
        ReadPropertyRequest {
            object_identifier: oid,
            property_identifier: *property,
            property_array_index: *index,
        }
        .encode(&mut rp_request);
        let mut rp_response = BytesMut::new();
        let rp = handle_read_property(db, &rp_request, &mut rp_response);
        match expected {
            Ok(bytes) => {
                assert_eq!(
                    result.property_value.as_deref(),
                    Some(bytes.as_slice()),
                    "{property:?} {index:?}"
                );
                rp.unwrap();
                let rp_ack = ReadPropertyACK::decode(&rp_response).unwrap();
                assert_eq!(rp_ack.property_array_index, *index);
                assert_eq!(rp_ack.property_value, *bytes, "{property:?} {index:?}");
            }
            Err(code) => {
                assert_eq!(
                    result.error,
                    Some((ErrorClass::PROPERTY, *code)),
                    "{property:?} {index:?}"
                );
                assert!(
                    matches!(rp, Err(Error::Protocol { class, code: actual })
                        if class == ErrorClass::PROPERTY
                            && actual == *code),
                    "{property:?} {index:?}: {rp:?}"
                );
            }
        }
    }
}

/// The read cases of an array holding `elements`: whole, size, first, last
/// and one past the end.
pub(super) fn array_cases(property: P, elements: &[&[u8]]) -> Vec<(P, Option<u32>, Expected)> {
    let size = elements.len() as u32;
    let mut cases = vec![
        (property, None, Ok(elements.concat())),
        (property, Some(0), Ok(vec![0x21, size as u8])),
    ];
    if let (Some(first), Some(last)) = (elements.first(), elements.last()) {
        cases.push((property, Some(1), Ok(first.to_vec())));
        cases.push((property, Some(size), Ok(last.to_vec())));
    }
    cases.push((
        property,
        Some(size + 1),
        Err(ErrorCode::INVALID_ARRAY_INDEX),
    ));
    cases
}

pub(super) fn db_with(object: Box<dyn BACnetObject>) -> (ObjectDatabase, ObjectIdentifier) {
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(object).unwrap();
    (db, oid)
}

fn reference(
    device: Option<u32>,
    object_type: ObjectType,
    instance: u32,
) -> BACnetDeviceObjectReference {
    BACnetDeviceObjectReference {
        device_identifier: device
            .map(|device| ObjectIdentifier::new(ObjectType::DEVICE, device).unwrap()),
        object_identifier: ObjectIdentifier::new(object_type, instance).unwrap(),
    }
}

#[test]
fn credential_data_input_format_arrays_read_per_index() {
    let empty = CredentialDataInputObject::new(1, "CDI-1").unwrap();
    let (db, oid) = db_with(Box::new(empty));
    let mut cases = array_cases(P::SUPPORTED_FORMATS, &[]);
    cases.extend(array_cases(P::SUPPORTED_FORMAT_CLASSES, &[]));
    assert_reads(&db, oid, &cases);

    let mut reader = CredentialDataInputObject::new(1, "CDI-1").unwrap();
    reader
        .set_supported_formats([
            (
                BACnetAuthenticationFactorFormat::standard(AuthenticationFactorType::WIEGAND26),
                0,
            ),
            (
                BACnetAuthenticationFactorFormat::standard(AuthenticationFactorType::FASC_N),
                1,
            ),
            (BACnetAuthenticationFactorFormat::custom(260, 7), 3),
        ])
        .unwrap();
    let (db, oid) = db_with(Box::new(reader));
    let mut cases = array_cases(
        P::SUPPORTED_FORMATS,
        &[
            // format-type [0] WIEGAND26.
            &[0x09, 0x08],
            // format-type [0] FASC_N.
            &[0x09, 0x0D],
            // CUSTOM, vendor-id [1] 260, vendor-format [2] 7.
            &[0x09, 0x02, 0x1A, 0x01, 0x04, 0x29, 0x07],
        ],
    );
    cases.extend(array_cases(
        P::SUPPORTED_FORMAT_CLASSES,
        &[&[0x21, 0x00], &[0x21, 0x01], &[0x21, 0x03]],
    ));
    assert_reads(&db, oid, &cases);
}

#[test]
fn access_door_door_members_read_per_index() {
    let (db, oid) = db_with(Box::new(AccessDoorObject::new(1, "DOOR-1").unwrap()));
    assert_reads(&db, oid, &array_cases(P::DOOR_MEMBERS, &[]));

    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    door.set_door_members([
        reference(None, ObjectType::BINARY_INPUT, 3),
        reference(None, ObjectType::BINARY_OUTPUT, 4),
        reference(Some(99), ObjectType::CREDENTIAL_DATA_INPUT, 1),
    ])
    .unwrap();
    let (db, oid) = db_with(Box::new(door));
    assert_reads(
        &db,
        oid,
        &array_cases(
            P::DOOR_MEMBERS,
            &[
                // object-identifier [1] Binary Input 3.
                &[0x1C, 0x00, 0xC0, 0x00, 0x03],
                // object-identifier [1] Binary Output 4.
                &[0x1C, 0x01, 0x00, 0x00, 0x04],
                // device-identifier [0] Device 99, then CDI 1.
                &[0x0C, 0x02, 0x00, 0x00, 0x63, 0x1C, 0x09, 0x40, 0x00, 0x01],
            ],
        ),
    );
}

#[test]
fn access_point_access_doors_read_per_index() {
    let (db, oid) = db_with(Box::new(AccessPointObject::new(1, "AP-1").unwrap()));
    assert_reads(&db, oid, &array_cases(P::ACCESS_DOORS, &[]));

    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    point
        .set_access_doors([
            reference(None, ObjectType::ACCESS_DOOR, 1),
            reference(Some(99), ObjectType::ACCESS_DOOR, 2),
        ])
        .unwrap();
    let (mut db, oid) = db_with(Box::new(point));
    assert_reads(
        &db,
        oid,
        &array_cases(
            P::ACCESS_DOORS,
            &[
                // object-identifier [1] Access Door 1.
                &[0x1C, 0x07, 0x80, 0x00, 0x01],
                // device-identifier [0] Device 99, then Access Door 2.
                &[0x0C, 0x02, 0x00, 0x00, 0x63, 0x1C, 0x07, 0x80, 0x00, 0x02],
            ],
        ),
    );

    // The arrays are read-only: an indexed write passes the array gate and
    // the object refuses it.
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: P::ACCESS_DOORS,
        property_array_index: Some(1),
        property_value: vec![0x1C, 0x07, 0x80, 0x00, 0x05],
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    assert!(matches!(
        handle_write_property(&mut db, &request),
        Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::WRITE_ACCESS_DENIED
    ));
}
