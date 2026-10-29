//! Encoded indexed writes distinguish an absent row from a served non-array.

use super::*;
use bacnet_objects::network_port::{BipPortConfig, NetworkPortObject};
use bacnet_objects::staging::{StagingConfig, StagingObject};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::error::RemoteError;

const VENDOR: PropertyIdentifier = PropertyIdentifier::from_raw(5555);

#[path = "indexed_write_effects.rs"]
mod effects;

fn staging(names: bool) -> StagingObject {
    StagingObject::new(
        1,
        "stages",
        StagingConfig {
            present_value: 0.0,
            min_present_value: -1.0,
            units: EngineeringUnits::DEGREES_CELSIUS,
            priority_for_writing: 8,
            stages: vec![
                BACnetStageLimitValue {
                    limit: 1.0,
                    values: vec![false],
                    deadband: 0.0,
                },
                BACnetStageLimitValue {
                    limit: 2.0,
                    values: vec![true],
                    deadband: 0.0,
                },
            ],
            target_references: vec![BACnetDeviceObjectReference {
                device_identifier: None,
                object_identifier: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1).unwrap(),
            }],
            stage_names: names.then(|| vec!["low".into(), "high".into()]),
        },
    )
    .unwrap()
}

fn value(value: PropertyValue) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    encode_property_value(&mut bytes, &value).unwrap();
    bytes.to_vec()
}

fn write(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: u32,
    value: Vec<u8>,
    multiple: bool,
) -> Result<(), Error> {
    let mut bytes = BytesMut::new();
    if multiple {
        WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: oid,
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: property,
                    property_array_index: Some(index),
                    value,
                    priority: None,
                }],
            }],
        }
        .encode(&mut bytes)
        .unwrap();
        handle_write_property_multiple(db, &bytes).map(|_| ())
    } else {
        WritePropertyRequest {
            object_identifier: oid,
            property_identifier: property,
            property_array_index: Some(index),
            property_value: value,
            priority: None,
        }
        .encode(&mut bytes)
        .unwrap();
        handle_write_property(db, &bytes).map(|_| ())
    }
}

fn assert_error(result: Result<(), Error>, expected: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
        if class == ErrorClass::PROPERTY && code == expected),
        "expected PROPERTY/{expected:?}, got {result:?}"
    );
}

fn assert_absent(object: Box<dyn BACnetObject>, property: PropertyIdentifier) {
    let oid = object.object_identifier();
    let watched = if oid.object_type() == ObjectType::STAGING {
        vec![
            PropertyIdentifier::STAGES,
            PropertyIdentifier::TARGET_REFERENCES,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::PRESENT_STAGE,
        ]
    } else {
        vec![PropertyIdentifier::DESCRIPTION]
    };
    // Present_Stage is deliberately uninitialized in this standalone fixture.
    // Preserve both values and protocol errors instead of initializing it here.
    let snapshot = |object: &dyn BACnetObject| {
        watched
            .iter()
            .map(|property| {
                object
                    .read_property(*property, None)
                    .map_err(|error| match error {
                        Error::Remote(RemoteError::Error { class, code }) => (class, code),
                        other => panic!("unexpected snapshot error: {other:?}"),
                    })
            })
            .collect::<Vec<_>>()
    };
    let before = snapshot(object.as_ref());
    let mut db = ObjectDatabase::new();
    db.add(object).unwrap();
    let mut results = Vec::new();
    for multiple in [false, true] {
        // REAL with a one-octet payload is framed correctly but not a valid
        // application value. The indexed presence gate must run before decode.
        for input in [value(PropertyValue::Unsigned(1)), vec![0x41, 0]] {
            results.push((
                multiple,
                input.clone(),
                write(&mut db, oid, property, 1, input, multiple),
            ));
            let after = snapshot(db.get(&oid).unwrap());
            assert_eq!(
                after, before,
                "absent writes must leave object state intact"
            );
        }
    }
    assert!(
        results.iter().all(|(_, _, result)| matches!(result,
        Err(Error::Remote(RemoteError::Error { class, code })) if *class == ErrorClass::PROPERTY
            && *code == ErrorCode::UNKNOWN_PROPERTY)),
        "WP/WPM must report UNKNOWN_PROPERTY before value decode: {results:?}"
    );
}

#[test]
fn indexed_write_absent_analog_input_property_is_unknown() {
    assert_absent(
        Box::new(AnalogInputObject::new(1, "input", EngineeringUnits::DEGREES_CELSIUS).unwrap()),
        VENDOR,
    );
}

#[test]
fn indexed_write_absent_bip_network_port_property_is_unknown() {
    assert_absent(
        Box::new(NetworkPortObject::new_bip(1, "port", BipPortConfig::default()).unwrap()),
        VENDOR,
    );
}

#[test]
fn indexed_write_unprovisioned_stage_names_is_unknown() {
    assert_absent(Box::new(staging(false)), PropertyIdentifier::STAGE_NAMES);
}

#[test]
fn indexed_write_present_array_preserves_element_count_and_range_rules() {
    let object = staging(true);
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    for multiple in [false, true] {
        let text = if multiple { "multiple" } else { "single" };
        write(
            &mut db,
            oid,
            PropertyIdentifier::STAGE_NAMES,
            1,
            value(PropertyValue::CharacterString(text.into())),
            multiple,
        )
        .unwrap();
        let before = db
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::STAGE_NAMES, None)
            .unwrap();
        assert_eq!(
            before,
            PropertyValue::List(vec![
                PropertyValue::CharacterString(text.into()),
                PropertyValue::CharacterString("high".into())
            ])
        );
        for (index, input, error) in [
            (
                0,
                value(PropertyValue::Unsigned(3)),
                ErrorCode::WRITE_ACCESS_DENIED,
            ),
            (
                3,
                value(PropertyValue::CharacterString("out of range".into())),
                ErrorCode::INVALID_ARRAY_INDEX,
            ),
            (1, vec![0x41, 0], ErrorCode::INVALID_DATA_ENCODING),
        ] {
            assert_error(
                write(
                    &mut db,
                    oid,
                    PropertyIdentifier::STAGE_NAMES,
                    index,
                    input,
                    multiple,
                ),
                error,
            );
            assert_eq!(
                db.get(&oid)
                    .unwrap()
                    .read_property(PropertyIdentifier::STAGE_NAMES, None)
                    .unwrap(),
                before
            );
        }
    }
}

#[test]
fn indexed_write_served_scalar_and_list_keep_predecode_not_array_error() {
    let objects: Vec<(Box<dyn BACnetObject>, PropertyIdentifier)> = vec![
        (
            Box::new(
                AnalogInputObject::new(1, "input", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
            ),
            PropertyIdentifier::OBJECT_IDENTIFIER,
        ),
        (
            Box::new(bacnet_objects::schedule::CalendarObject::new(1, "calendar").unwrap()),
            PropertyIdentifier::DATE_LIST,
        ),
    ];
    for (object, property) in objects {
        let oid = object.object_identifier();
        let before = object.read_property(property, None).unwrap();
        let mut db = ObjectDatabase::new();
        db.add(object).unwrap();
        for multiple in [false, true] {
            for input in [value(PropertyValue::Unsigned(1)), vec![0x41, 0]] {
                assert_error(
                    write(&mut db, oid, property, 1, input, multiple),
                    ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
                );
                assert_eq!(
                    db.get(&oid).unwrap().read_property(property, None).unwrap(),
                    before
                );
            }
        }
    }
}

#[test]
fn indexed_write_bip_dns_array_remains_read_only() {
    let object = NetworkPortObject::new_bip(1, "port", BipPortConfig::default()).unwrap();
    let oid = object.object_identifier();
    let before = object
        .read_property(PropertyIdentifier::IP_DNS_SERVER, None)
        .unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    for multiple in [false, true] {
        // The read-only object dispatch, not a new service range policy, owns
        // both count and element writes (including the out-of-range element).
        for index in [0, 1, 2] {
            assert_error(
                write(
                    &mut db,
                    oid,
                    PropertyIdentifier::IP_DNS_SERVER,
                    index,
                    value(PropertyValue::OctetString(vec![192, 0, 2, 1])),
                    multiple,
                ),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
        }
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::IP_DNS_SERVER, None)
                .unwrap(),
            before
        );
    }
}

#[test]
fn indexed_write_staging_structured_arrays_retain_object_dispatch() {
    use bacnet_encoding::constructed::{encode_device_object_reference, encode_stage_limit_value};
    let mut stage = BytesMut::new();
    encode_stage_limit_value(
        &mut stage,
        &BACnetStageLimitValue {
            limit: 0.5,
            values: vec![true],
            deadband: 0.0,
        },
    );
    let mut target = BytesMut::new();
    encode_device_object_reference(
        &mut target,
        &BACnetDeviceObjectReference {
            device_identifier: None,
            object_identifier: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 2).unwrap(),
        },
    );
    for multiple in [false, true] {
        let object = staging(false);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        for (property, input, past_end) in [
            (PropertyIdentifier::STAGES, stage.to_vec(), 3),
            (PropertyIdentifier::TARGET_REFERENCES, target.to_vec(), 2),
        ] {
            write(&mut db, oid, property, 1, input.clone(), multiple).unwrap();
            assert_eq!(
                db.get(&oid)
                    .unwrap()
                    .read_property(property, Some(1))
                    .unwrap(),
                PropertyValue::ApplicationData(input.clone())
            );
            let before = db.get(&oid).unwrap().read_property(property, None).unwrap();
            assert_error(
                write(
                    &mut db,
                    oid,
                    property,
                    0,
                    value(PropertyValue::Unsigned(3)),
                    multiple,
                ),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            assert_error(
                write(&mut db, oid, property, past_end, input, multiple),
                ErrorCode::INVALID_ARRAY_INDEX,
            );
            assert_eq!(
                db.get(&oid).unwrap().read_property(property, None).unwrap(),
                before
            );
        }
    }
}

struct VendorArray {
    value: PropertyValue,
}

impl BACnetObject for VendorArray {
    fn object_identifier(&self) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::from_raw(128), 1).unwrap()
    }
    fn object_name(&self) -> &str {
        "vendor-array"
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        assert_eq!((property, index), (VENDOR, Some(1)));
        Ok(self.value.clone())
    }
    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        index: Option<u32>,
        value: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        assert_eq!((property, index), (VENDOR, Some(1)));
        self.value = value;
        Ok(())
    }
    fn property_list(&self) -> std::borrow::Cow<'static, [PropertyIdentifier]> {
        std::borrow::Cow::Owned(vec![VENDOR])
    }
    fn is_array_property(&self, property: PropertyIdentifier) -> bool {
        property == VENDOR
    }
    // The optional metadata default remains empty; it is not an absence claim.
}

#[test]
fn indexed_write_empty_metadata_vendor_array_delegates_to_custom_writer() {
    let object = VendorArray {
        value: PropertyValue::Unsigned(0),
    };
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    for (multiple, number) in [(false, 17), (true, 29)] {
        write(
            &mut db,
            oid,
            VENDOR,
            1,
            value(PropertyValue::Unsigned(number)),
            multiple,
        )
        .unwrap();
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(VENDOR, Some(1))
                .unwrap(),
            PropertyValue::Unsigned(number)
        );
        assert_error(
            write(
                &mut db,
                oid,
                PropertyIdentifier::DESCRIPTION,
                1,
                value(PropertyValue::Unsigned(1)),
                multiple,
            ),
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        );
    }
}
