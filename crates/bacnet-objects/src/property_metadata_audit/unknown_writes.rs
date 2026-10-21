use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode};

fn code(error: Error) -> (ErrorClass, ErrorCode) {
    match error {
        Error::Protocol { class, code } => (class, code),
        other => panic!("expected protocol error, got {other:?}"),
    }
}

#[test]
fn unknown_writes_classify_every_builtin_with_null_and_ordinary_values() {
    let mut objects = supported_representatives();
    audit_object_type_coverage(&objects);
    let mut failures = Vec::new();
    for object in &mut objects {
        for value in [PropertyValue::Null, PropertyValue::Unsigned(17)] {
            for (property, expected) in [
                (
                    PropertyIdentifier::from_raw(5555),
                    ErrorCode::UNKNOWN_PROPERTY,
                ),
                (
                    PropertyIdentifier::OBJECT_IDENTIFIER,
                    ErrorCode::WRITE_ACCESS_DENIED,
                ),
                (
                    PropertyIdentifier::PROPERTY_LIST,
                    ErrorCode::WRITE_ACCESS_DENIED,
                ),
            ] {
                let actual = code(
                    object
                        .write_property(property, None, value.clone(), None)
                        .unwrap_err(),
                );
                if actual != (ErrorClass::PROPERTY, expected) {
                    failures.push(format!(
                        "{:?} {property:?} {value:?}: {actual:?}, expected {expected:?}",
                        object.object_identifier()
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    for object in &mut objects {
        // Network Port already distinguished absent indexed properties before #870.
        let expected = match object.object_identifier().object_type() {
            ObjectType::NETWORK_PORT => ErrorCode::UNKNOWN_PROPERTY,
            ObjectType::STAGING | ObjectType::CHANNEL => ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
            _ => ErrorCode::WRITE_ACCESS_DENIED,
        };
        for index in [0, 1] {
            assert_eq!(
                code(
                    object
                        .write_property(
                            PropertyIdentifier::from_raw(5555),
                            Some(index),
                            PropertyValue::Null,
                            None
                        )
                        .unwrap_err()
                ),
                (ErrorClass::PROPERTY, expected),
                "{:?}",
                object.object_identifier()
            );
        }
    }
}

#[test]
fn unknown_writes_optional_staging_names_use_current_presence() {
    let mut config = staging_config();
    config.stage_names = None;
    let mut absent = StagingObject::new(1, "absent", config).unwrap();
    assert!(!absent
        .property_metadata()
        .iter()
        .any(|row| row.property_identifier == PropertyIdentifier::STAGE_NAMES));
    for value in [PropertyValue::Null, PropertyValue::List(vec![])] {
        assert_eq!(
            code(
                absent
                    .write_property(PropertyIdentifier::STAGE_NAMES, None, value, None)
                    .unwrap_err()
            ),
            (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)
        );
    }
    // Indexed precedence is deliberately unchanged by this unindexed correction.
    assert_eq!(
        code(
            absent
                .write_property(
                    PropertyIdentifier::STAGE_NAMES,
                    Some(0),
                    PropertyValue::Null,
                    None
                )
                .unwrap_err()
        ),
        (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
    );
    let mut present = StagingObject::new(2, "present", staging_config()).unwrap();
    assert_eq!(
        code(
            present
                .write_property(
                    PropertyIdentifier::STAGE_NAMES,
                    Some(0),
                    PropertyValue::Unsigned(3),
                    None
                )
                .unwrap_err()
        ),
        (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
    );
    let names = PropertyValue::List(vec![PropertyValue::CharacterString("new".into()); 3]);
    present
        .write_property(PropertyIdentifier::STAGE_NAMES, None, names.clone(), None)
        .unwrap();
    assert_eq!(
        present
            .read_property(PropertyIdentifier::STAGE_NAMES, None)
            .unwrap(),
        names
    );
}

#[test]
fn unknown_writes_file_record_count_distinguishes_absence_from_read_only() {
    use bacnet_types::enums::FileAccessMethod;
    for records in [false, true] {
        let mut file = FileObject::new(1, "File", "raw").unwrap();
        if records {
            file.set_file_access_method(FileAccessMethod::RECORD_ACCESS.to_raw());
            file.set_read_only(true);
        }
        assert_eq!(
            file.property_metadata()
                .iter()
                .any(|row| row.property_identifier == PropertyIdentifier::RECORD_COUNT),
            records
        );
        for value in [PropertyValue::Null, PropertyValue::Unsigned(17)] {
            let expected = if records {
                ErrorCode::WRITE_ACCESS_DENIED
            } else {
                ErrorCode::UNKNOWN_PROPERTY
            };
            assert_eq!(
                code(
                    file.write_property(PropertyIdentifier::RECORD_COUNT, None, value, None)
                        .unwrap_err()
                ),
                (ErrorClass::PROPERTY, expected)
            );
        }
        assert_eq!(
            code(
                file.write_property(
                    PropertyIdentifier::RECORD_COUNT,
                    Some(0),
                    PropertyValue::Null,
                    None
                )
                .unwrap_err()
            ),
            (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
        );
    }
}
