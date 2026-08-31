//! WPM rollback for retained multi-state command configuration after count shrink.

use super::*;
use bacnet_objects::multistate::{MultiStateOutputObject, MultiStateValueObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_types::enums::Reliability;
use bytes::BytesMut;

fn failed_wpm_after_unsigned_repair(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    repaired_value: u64,
) {
    let mut repair = BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut repair, repaired_value);
    let mut read_only_value = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut read_only_value, 0);
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: property,
                    property_array_index: array_index,
                    value: repair.to_vec(),
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_TYPE,
                    property_array_index: None,
                    value: read_only_value.to_vec(),
                    priority: None,
                },
            ],
        }],
    };
    let mut request_bytes = BytesMut::new();
    request.encode(&mut request_bytes);

    assert!(
        matches!(
            handle_write_property_multiple(db, &request_bytes),
            Err(Error::Protocol { class, code })
                if class == ErrorClass::PROPERTY.to_raw() as u32
                    && code == ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32
        ),
        "rollback must succeed and preserve the later write's protocol error"
    );
}

fn assert_invalid_default_restored(mut db: ObjectDatabase, oid: ObjectIdentifier) {
    let priority_before = db
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, None)
        .unwrap();

    failed_wpm_after_unsigned_repair(
        &mut db,
        oid,
        PropertyIdentifier::RELINQUISH_DEFAULT,
        None,
        2,
    );

    let object = db.get_mut(&oid).unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, None)
            .unwrap(),
        priority_before
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
            .unwrap(),
        PropertyValue::Unsigned(3)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(3)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );

    object
        .write_property(
            PropertyIdentifier::RELINQUISH_DEFAULT,
            None,
            PropertyValue::Unsigned(2),
            None,
        )
        .unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
        "recovery proves rollback restored configuration-fault ownership"
    );
}

fn assert_invalid_inactive_priority_restored(mut db: ObjectDatabase, oid: ObjectIdentifier) {
    let default_before = db
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
        .unwrap();

    failed_wpm_after_unsigned_repair(
        &mut db,
        oid,
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(16),
        2,
    );

    let object = db.get_mut(&oid).unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
            .unwrap(),
        PropertyValue::Unsigned(1),
        "the active command must remain unchanged"
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(16))
            .unwrap(),
        PropertyValue::Unsigned(3),
        "the invalid inactive command must be restored without client validation"
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
            .unwrap(),
        default_before
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );

    object
        .write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(16),
            PropertyValue::Unsigned(2),
            None,
        )
        .unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
        "recovery proves rollback restored configuration-fault ownership"
    );
}

fn mso_with_invalid_default(instance: u32) -> (ObjectDatabase, ObjectIdentifier) {
    let mut object = MultiStateOutputObject::new(instance, "MSO-default-rollback", 3).unwrap();
    object.set_relinquish_default(3).unwrap();
    object.set_number_of_states(2).unwrap();
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    (db, oid)
}

fn msv_with_invalid_default(instance: u32) -> (ObjectDatabase, ObjectIdentifier) {
    let mut object = MultiStateValueObject::new(instance, "MSV-default-rollback", 3).unwrap();
    object.set_relinquish_default(3).unwrap();
    object.set_number_of_states(2).unwrap();
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    (db, oid)
}

fn mso_with_invalid_inactive_priority(instance: u32) -> (ObjectDatabase, ObjectIdentifier) {
    let mut object = MultiStateOutputObject::new(instance, "MSO-priority-rollback", 3).unwrap();
    object
        .write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(16),
            PropertyValue::Unsigned(3),
            None,
        )
        .unwrap();
    object
        .write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(8),
            PropertyValue::Unsigned(1),
            None,
        )
        .unwrap();
    object.set_number_of_states(2).unwrap();
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    (db, oid)
}

fn msv_with_invalid_inactive_priority(instance: u32) -> (ObjectDatabase, ObjectIdentifier) {
    let mut object = MultiStateValueObject::new(instance, "MSV-priority-rollback", 3).unwrap();
    object
        .write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(16),
            PropertyValue::Unsigned(3),
            None,
        )
        .unwrap();
    object
        .write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(8),
            PropertyValue::Unsigned(1),
            None,
        )
        .unwrap();
    object.set_number_of_states(2).unwrap();
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    (db, oid)
}

#[test]
fn mso_retained_invalid_default_is_restored_after_failed_wpm_repair() {
    let (db, oid) = mso_with_invalid_default(20);
    assert_invalid_default_restored(db, oid);
}

#[test]
fn msv_retained_invalid_default_is_restored_after_failed_wpm_repair() {
    let (db, oid) = msv_with_invalid_default(21);
    assert_invalid_default_restored(db, oid);
}

#[test]
fn mso_retained_invalid_inactive_priority_is_restored_after_failed_wpm_repair() {
    let (db, oid) = mso_with_invalid_inactive_priority(22);
    assert_invalid_inactive_priority_restored(db, oid);
}

#[test]
fn msv_retained_invalid_inactive_priority_is_restored_after_failed_wpm_repair() {
    let (db, oid) = msv_with_invalid_inactive_priority(23);
    assert_invalid_inactive_priority_restored(db, oid);
}
