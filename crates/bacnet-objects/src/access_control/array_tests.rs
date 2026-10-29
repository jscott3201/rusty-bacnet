//! The access-control BACnetARRAY rows read per index (#1169): Credential
//! Data Input Supported_Formats (BACnetAuthenticationFactorFormat elements)
//! and Supported_Format_Classes (Unsigned elements) from Table 12-43, Access
//! Door Door_Members (Table 12-30) and Access Point Access_Doors
//! (Table 12-36), both BACnetDeviceObjectReference elements.
//!
//! Each array reads whole with no index, its size at index 0, one element at
//! 1 to N and INVALID_ARRAY_INDEX past N.

use bacnet_types::constructed::{BACnetAuthenticationFactorFormat, BACnetDeviceObjectReference};
use bacnet_types::enums::PropertyIdentifier as P;
use bacnet_types::enums::{AuthenticationFactorType as F, ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;

use super::*;

fn assert_property_error(result: Result<PropertyValue, Error>, code: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class, code: actual }))
            if class == ErrorClass::PROPERTY
                && actual == code),
        "expected PROPERTY / {code:?}, got {result:?}"
    );
}

/// Read `property` of `object` whole, at index 0, at every index and past
/// the end, and check each against `elements`.
fn assert_array(object: &dyn BACnetObject, property: P, elements: &[PropertyValue]) {
    assert!(object.is_array_property(property), "{property:?}");
    assert!(!object.is_list_property(property), "{property:?}");
    assert_eq!(
        object.read_property(property, None).unwrap(),
        PropertyValue::List(elements.to_vec()),
        "{property:?}"
    );
    assert_eq!(
        object.read_property(property, Some(0)).unwrap(),
        PropertyValue::Unsigned(elements.len() as u64),
        "{property:?}"
    );
    for (index, element) in elements.iter().enumerate() {
        assert_eq!(
            object
                .read_property(property, Some(index as u32 + 1))
                .unwrap(),
            *element,
            "{property:?}[{}]",
            index + 1
        );
    }
    for index in [elements.len() as u32 + 1, u32::MAX] {
        assert_property_error(
            object.read_property(property, Some(index)),
            ErrorCode::INVALID_ARRAY_INDEX,
        );
    }
}

fn data(bytes: &[u8]) -> PropertyValue {
    PropertyValue::ApplicationData(bytes.to_vec())
}

fn local(object_type: ObjectType, instance: u32) -> BACnetDeviceObjectReference {
    ObjectIdentifier::new(object_type, instance).unwrap().into()
}

fn remote(device: u32, object_type: ObjectType, instance: u32) -> BACnetDeviceObjectReference {
    BACnetDeviceObjectReference {
        device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, device).unwrap()),
        object_identifier: ObjectIdentifier::new(object_type, instance).unwrap(),
    }
}

#[test]
fn credential_data_input_supported_formats_read_as_arrays() {
    let mut reader = CredentialDataInputObject::new(1, "CDI-1").unwrap();
    assert_array(&reader, P::SUPPORTED_FORMATS, &[]);
    assert_array(&reader, P::SUPPORTED_FORMAT_CLASSES, &[]);

    reader
        .set_supported_formats([
            (BACnetAuthenticationFactorFormat::standard(F::WIEGAND26), 0),
            (BACnetAuthenticationFactorFormat::custom(260, 7), 3),
            (
                BACnetAuthenticationFactorFormat {
                    format_type: F::ABA_TRACK2,
                    vendor_id: Some(0),
                    vendor_format: Some(0),
                },
                70_000,
            ),
        ])
        .unwrap();
    assert_array(
        &reader,
        P::SUPPORTED_FORMATS,
        &[
            // format-type [0] WIEGAND26.
            data(&[0x09, 0x08]),
            // CUSTOM, vendor-id [1] 260, vendor-format [2] 7.
            data(&[0x09, 0x02, 0x1A, 0x01, 0x04, 0x29, 0x07]),
            // ABA_TRACK2 with both vendor members zero.
            data(&[0x09, 0x07, 0x19, 0x00, 0x29, 0x00]),
        ],
    );
    assert_array(
        &reader,
        P::SUPPORTED_FORMAT_CLASSES,
        &[
            PropertyValue::Unsigned(0),
            PropertyValue::Unsigned(3),
            PropertyValue::Unsigned(70_000),
        ],
    );
}

#[test]
fn credential_data_input_set_supported_formats_refuses_ill_formed_formats() {
    let mut reader = CredentialDataInputObject::new(1, "CDI-1").unwrap();
    let declared = [(BACnetAuthenticationFactorFormat::standard(F::GUID), 1)];
    reader.set_supported_formats(declared).unwrap();
    for format in [
        // 25 lies past the closed production.
        BACnetAuthenticationFactorFormat::standard(F::from_raw(25)),
        // CUSTOM needs both vendor members.
        BACnetAuthenticationFactorFormat::standard(F::CUSTOM),
        BACnetAuthenticationFactorFormat {
            format_type: F::CUSTOM,
            vendor_id: Some(260),
            vendor_format: None,
        },
        // Any other format carries them only as zero.
        BACnetAuthenticationFactorFormat {
            format_type: F::WIEGAND26,
            vendor_id: Some(260),
            vendor_format: None,
        },
        BACnetAuthenticationFactorFormat {
            format_type: F::WIEGAND26,
            vendor_id: None,
            vendor_format: Some(1),
        },
    ] {
        let result = reader.set_supported_formats([
            (BACnetAuthenticationFactorFormat::standard(F::WIEGAND26), 0),
            (format, 0),
        ]);
        assert!(
            matches!(result, Err(Error::Remote(RemoteError::Error { code, .. }))
                if code == ErrorCode::VALUE_OUT_OF_RANGE),
            "{format:?}: {result:?}"
        );
        assert_array(&reader, P::SUPPORTED_FORMATS, &[data(&[0x09, 0x14])]);
        assert_array(
            &reader,
            P::SUPPORTED_FORMAT_CLASSES,
            &[PropertyValue::Unsigned(1)],
        );
    }
    // An empty list clears both arrays together.
    reader
        .set_supported_formats(Vec::<(BACnetAuthenticationFactorFormat, u32)>::new())
        .unwrap();
    assert_array(&reader, P::SUPPORTED_FORMATS, &[]);
    assert_array(&reader, P::SUPPORTED_FORMAT_CLASSES, &[]);
}

#[test]
fn access_door_door_members_read_as_an_array() {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    assert_array(&door, P::DOOR_MEMBERS, &[]);

    door.set_door_members([
        local(ObjectType::BINARY_INPUT, 3),
        remote(99, ObjectType::CREDENTIAL_DATA_INPUT, 1),
    ])
    .unwrap();
    assert_array(
        &door,
        P::DOOR_MEMBERS,
        &[
            // object-identifier [1] Binary Input 3, no device.
            data(&[0x1C, 0x00, 0xC0, 0x00, 0x03]),
            // device-identifier [0] Device 99, object-identifier [1] CDI 1.
            data(&[0x0C, 0x02, 0x00, 0x00, 0x63, 0x1C, 0x09, 0x40, 0x00, 0x01]),
        ],
    );
}

#[test]
fn access_point_access_doors_read_as_an_array() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    assert_array(&point, P::ACCESS_DOORS, &[]);

    point
        .set_access_doors([
            local(ObjectType::ACCESS_DOOR, 1),
            remote(99, ObjectType::ACCESS_DOOR, 2),
        ])
        .unwrap();
    let doors = [
        // object-identifier [1] Access Door 1, no device.
        data(&[0x1C, 0x07, 0x80, 0x00, 0x01]),
        // device-identifier [0] Device 99, object-identifier [1] Access Door 2.
        data(&[0x0C, 0x02, 0x00, 0x00, 0x63, 0x1C, 0x07, 0x80, 0x00, 0x02]),
    ];
    assert_array(&point, P::ACCESS_DOORS, &doors);

    // Only Access Door objects are commanded doors (Clause 12.31.32).
    let result = point.set_access_doors([
        local(ObjectType::ACCESS_DOOR, 3),
        local(ObjectType::BINARY_OUTPUT, 1),
    ]);
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { code, .. }))
            if code == ErrorCode::VALUE_OUT_OF_RANGE),
        "{result:?}"
    );
    assert_array(&point, P::ACCESS_DOORS, &doors);
}
