use super::*;
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::BACnetTimeStamp;

const BACKUP_FAILURE_TIMEOUT: u16 = 90;

/// The properties a device has only while it performs backup and restore.
const BACKUP_PROPERTIES: [PropertyIdentifier; 4] = [
    PropertyIdentifier::CONFIGURATION_FILES,
    PropertyIdentifier::LAST_RESTORE_TIME,
    PropertyIdentifier::BACKUP_FAILURE_TIMEOUT,
    PropertyIdentifier::BACKUP_AND_RESTORE_STATE,
];

fn backup_device() -> DeviceObject {
    DeviceObject::new(DeviceConfig {
        backup_and_restore: true,
        backup_failure_timeout: BACKUP_FAILURE_TIMEOUT,
        ..DeviceConfig::default()
    })
    .unwrap()
}

fn file(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::FILE, instance).unwrap()
}

fn assert_property_error(result: Result<impl std::fmt::Debug, Error>, code: ErrorCode) {
    match result {
        Err(Error::Remote(RemoteError::Error {
            class,
            code: actual,
        })) => {
            assert_eq!(class, ErrorClass::PROPERTY);
            assert_eq!(actual, code, "expected {code:?}");
        }
        other => panic!("expected PROPERTY / {code:?}, got {other:?}"),
    }
}

fn write_timeout(device: &mut DeviceObject, value: PropertyValue) -> Result<(), Error> {
    device.write_property(
        PropertyIdentifier::BACKUP_FAILURE_TIMEOUT,
        None,
        value,
        None,
    )
}

fn backup_failure_timeout(device: &DeviceObject) -> PropertyValue {
    device
        .read_property(PropertyIdentifier::BACKUP_FAILURE_TIMEOUT, None)
        .unwrap()
}

/// Table 12-13 footnotes 7 and 8: present only if the device performs backup and restore.
#[test]
fn a_device_without_backup_has_no_backup_properties() {
    let mut device = make_device();

    for property in BACKUP_PROPERTIES {
        assert!(!device.property_list().contains(&property), "{property:?}");
        assert_property_error(
            device.read_property(property, None),
            ErrorCode::UNKNOWN_PROPERTY,
        );
    }
    assert_property_error(
        write_timeout(&mut device, PropertyValue::Unsigned(30)),
        ErrorCode::UNKNOWN_PROPERTY,
    );
}

#[test]
fn a_backup_device_serves_the_backup_properties() {
    let device = backup_device();

    for property in BACKUP_PROPERTIES {
        assert!(device.property_list().contains(&property), "{property:?}");
    }
    assert_eq!(
        device
            .read_property(PropertyIdentifier::CONFIGURATION_FILES, None)
            .unwrap(),
        PropertyValue::List(Vec::new())
    );
    assert_eq!(
        device
            .read_property(PropertyIdentifier::LAST_RESTORE_TIME, None)
            .unwrap(),
        // An unspecified dateTime, choice [2].
        PropertyValue::ApplicationData(vec![
            0x2E, 0xA4, 0xFF, 0xFF, 0xFF, 0xFF, 0xB4, 0xFF, 0xFF, 0xFF, 0xFF, 0x2F
        ])
    );
    assert_eq!(
        backup_failure_timeout(&device),
        PropertyValue::Unsigned(u64::from(BACKUP_FAILURE_TIMEOUT))
    );
    assert_eq!(
        device
            .read_property(PropertyIdentifier::BACKUP_AND_RESTORE_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(BackupAndRestoreState::IDLE.to_raw())
    );
}

/// A BACnetARRAY: index 0 is the count, 1..=N the elements.
#[test]
fn configuration_files_reads_as_an_array() {
    let mut device = backup_device();
    device.set_configuration_files(vec![file(1), file(2)]);
    let read = |index| device.read_property(PropertyIdentifier::CONFIGURATION_FILES, index);

    assert_eq!(
        read(None).unwrap(),
        PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(file(1)),
            PropertyValue::ObjectIdentifier(file(2)),
        ])
    );
    assert_eq!(read(Some(0)).unwrap(), PropertyValue::Unsigned(2));
    assert_eq!(
        read(Some(2)).unwrap(),
        PropertyValue::ObjectIdentifier(file(2))
    );
    assert_property_error(read(Some(3)), ErrorCode::INVALID_ARRAY_INDEX);
    assert!(device.is_array_property(PropertyIdentifier::CONFIGURATION_FILES));
}

#[test]
fn the_procedure_state_is_set_through_the_device_authority() {
    let mut device: Box<dyn BACnetObject> = Box::new(backup_device());
    let restored_at = BACnetTimeStamp::SequenceNumber(7);

    let mut authority = device
        .device_authority_internal()
        .expect("the Device has an authority");
    authority.set_backup_and_restore_state(BackupAndRestoreState::PERFORMING_A_BACKUP);
    authority.set_last_restore_time(restored_at);
    authority.set_configuration_files(vec![file(1)]);

    assert_eq!(
        device
            .read_property(PropertyIdentifier::BACKUP_AND_RESTORE_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(BackupAndRestoreState::PERFORMING_A_BACKUP.to_raw())
    );
    assert_eq!(
        device
            .read_property(PropertyIdentifier::LAST_RESTORE_TIME, None)
            .unwrap(),
        // SequenceNumber 7, choice [1].
        PropertyValue::ApplicationData(vec![0x19, 0x07])
    );
    assert_eq!(
        device
            .read_property(PropertyIdentifier::CONFIGURATION_FILES, None)
            .unwrap(),
        PropertyValue::List(vec![PropertyValue::ObjectIdentifier(file(1))])
    );
}

#[test]
fn a_zero_backup_failure_timeout_is_refused_at_construction() {
    let device = DeviceObject::new(DeviceConfig {
        backup_and_restore: true,
        backup_failure_timeout: 0,
        ..DeviceConfig::default()
    });

    assert!(matches!(device, Err(Error::Encoding(_))));
}

/// Table 12-13 footnote 8: Backup_Failure_Timeout is writable when present.
#[test]
fn backup_failure_timeout_accepts_a_write() {
    let mut device = backup_device();

    write_timeout(&mut device, PropertyValue::Unsigned(300)).unwrap();
    assert_eq!(
        backup_failure_timeout(&device),
        PropertyValue::Unsigned(300)
    );

    // Relinquishing a non-commandable property leaves it as it is.
    write_timeout(&mut device, PropertyValue::Null).unwrap();
    assert_eq!(
        backup_failure_timeout(&device),
        PropertyValue::Unsigned(300)
    );
}

#[test]
fn backup_failure_timeout_refuses_what_is_not_a_timeout() {
    let mut device = backup_device();
    let before = backup_failure_timeout(&device);

    for (value, code) in [
        (PropertyValue::Unsigned(0), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            PropertyValue::Unsigned(u64::from(u16::MAX) + 1),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (PropertyValue::Real(30.0), ErrorCode::INVALID_DATA_TYPE),
    ] {
        assert_property_error(write_timeout(&mut device, value), code);
        assert_eq!(backup_failure_timeout(&device), before);
    }
    assert_property_error(
        device.write_property(
            PropertyIdentifier::BACKUP_FAILURE_TIMEOUT,
            Some(1),
            PropertyValue::Unsigned(30),
            None,
        ),
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
    );
}
