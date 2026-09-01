//! Lossless WPM rollback for destructive File_Size / Record_Count writes.

use super::*;
use bacnet_objects::file::FileObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_types::enums::FileAccessMethod;

fn failed_resize_wpm(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    target: u64,
) -> (Result<Vec<ObjectIdentifier>, Error>, Vec<ObjectIdentifier>) {
    let mut target_value = BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut target_value, target);
    let mut read_only = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut read_only, 0);
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: property,
                    property_array_index: None,
                    value: target_value.to_vec(),
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_TYPE,
                    property_array_index: None,
                    value: read_only.to_vec(),
                    priority: None,
                },
            ],
        }],
    };
    let mut request_bytes = BytesMut::new();
    request.encode(&mut request_bytes);
    handle_write_property_multiple_with_residuals(db, &request_bytes)
}

fn metadata(
    object: &dyn BACnetObject,
) -> (
    PropertyValue,
    Option<PropertyValue>,
    PropertyValue,
    PropertyValue,
) {
    (
        object
            .read_property(PropertyIdentifier::FILE_SIZE, None)
            .unwrap(),
        object
            .read_property(PropertyIdentifier::RECORD_COUNT, None)
            .ok(),
        object
            .read_property(PropertyIdentifier::MODIFICATION_DATE, None)
            .unwrap(),
        object
            .read_property(PropertyIdentifier::ARCHIVE, None)
            .unwrap(),
    )
}

fn arm_metadata(file: &mut FileObject) {
    file.set_modification_date(
        bacnet_types::primitives::Date {
            year: 126,
            month: 8,
            day: 31,
            day_of_week: 1,
        },
        bacnet_types::primitives::Time {
            hour: 5,
            minute: 4,
            second: 3,
            hundredths: 2,
        },
    );
    file.set_archive(true);
}

#[test]
fn stream_resize_wpm_rollback_restores_exact_bytes_size_and_metadata() {
    let original = vec![1, 2, 3, 4, 5, 6];
    for target in [0, 3, 9] {
        let mut file =
            FileObject::new(target as u32 + 1, format!("STREAM-{target}"), "raw").unwrap();
        file.set_data(original.clone());
        arm_metadata(&mut file);
        let oid = file.object_identifier();
        let expected_metadata = metadata(&file);
        let mut db = ObjectDatabase::new();
        db.add(Box::new(file)).unwrap();

        let (result, residuals) =
            failed_resize_wpm(&mut db, oid, PropertyIdentifier::FILE_SIZE, target);

        assert!(result.is_err(), "target {target}");
        assert!(residuals.is_empty(), "target {target}");
        let object = db.get(&oid).unwrap();
        assert_eq!(metadata(object), expected_metadata, "target {target}");
        assert_eq!(
            object
                .file_storage_internal()
                .unwrap()
                .read_stream(0, original.len() as u64)
                .unwrap()
                .data,
            original,
            "target {target}"
        );
    }
}

#[test]
fn record_resize_wpm_rollback_restores_exact_records_count_size_and_metadata() {
    let original = vec![vec![1, 2], vec![3], vec![4, 5, 6]];
    for target in [0, 2, 5] {
        let mut file =
            FileObject::new(target as u32 + 20, format!("RECORD-{target}"), "raw").unwrap();
        file.set_file_access_method(FileAccessMethod::RECORD_ACCESS.to_raw());
        file.set_records(original.clone());
        arm_metadata(&mut file);
        let oid = file.object_identifier();
        let expected_metadata = metadata(&file);
        let mut db = ObjectDatabase::new();
        db.add(Box::new(file)).unwrap();

        let (result, residuals) =
            failed_resize_wpm(&mut db, oid, PropertyIdentifier::RECORD_COUNT, target);

        assert!(result.is_err(), "target {target}");
        assert!(residuals.is_empty(), "target {target}");
        let object = db.get(&oid).unwrap();
        assert_eq!(metadata(object), expected_metadata, "target {target}");
        assert_eq!(
            object
                .file_storage_internal()
                .unwrap()
                .read_records(0, original.len() as u64)
                .unwrap()
                .records,
            original,
            "target {target}"
        );
    }
}
