//! The database's selected Device and the local-reference rule built on it
//! (#1183, #1184).
use super::*;
use crate::analog::AnalogValueObject;
use crate::device::{DeviceConfig, DeviceObject};
use bacnet_types::enums::EngineeringUnits;

fn device(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

fn database(instances: &[u32]) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        AnalogValueObject::new(1, "AV-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    for &instance in instances {
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                identity: crate::DeviceIdentity::new(instance, 0)
                    .expect("valid Device identity")
                    .with_name(format!("Device-{instance}")),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();
    }
    db
}

#[test]
fn selected_device_is_independent_of_iteration_order() {
    let low = device(813);
    let high = device(900);
    let analog = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap();
    for candidates in [
        [analog, high, low],
        [analog, low, high],
        [high, low, analog],
    ] {
        assert_eq!(select_device(candidates), Some(low));
    }
}

#[test]
fn selected_device_preserves_empty_single_and_wildcard_cases() {
    let concrete = device(813);
    let wildcard = device(ObjectIdentifier::WILDCARD_INSTANCE);
    let analog = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap();
    assert_eq!(select_device([]), None);
    assert_eq!(select_device([analog]), None);
    assert_eq!(select_device([analog, concrete]), Some(concrete));
    // The pure choice passes the wildcard through; it's ObjectDatabase::add
    // that keeps a Device there out of every database (#2171).
    assert_eq!(select_device([analog, wildcard]), Some(wildcard));
}

#[test]
fn the_database_selects_its_lowest_device() {
    assert_eq!(database(&[]).selected_device(), None);
    assert_eq!(database(&[900, 813]).selected_device(), Some(device(813)));
}

#[test]
fn neither_an_identity_nor_the_database_takes_the_reserved_instance() {
    // Clause 12.1.1 reserves instance 4194303 (#2171): no Device identity
    // names it, and the database refuses any object there and stays as it
    // was.
    let wildcard = ObjectIdentifier::WILDCARD_INSTANCE;
    assert!(matches!(
        crate::DeviceIdentity::new(wildcard, 0),
        Err(bacnet_types::error::Error::Range(_))
    ));
    crate::DeviceIdentity::new(ObjectIdentifier::MAX_ADDRESSABLE_INSTANCE, 0).unwrap();
    let mut db = database(&[900]);
    let refused = db.add(Box::new(
        AnalogValueObject::new(wildcard, "Wildcard", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ));
    assert!(
        matches!(refused, Err(bacnet_types::error::Error::Range(_))),
        "{refused:?}"
    );
    let reserved = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, wildcard).unwrap();
    assert!(db.get(&reserved).is_none());
    assert!(db.find_by_name("Wildcard").is_none());
    assert_eq!(db.len(), 2);
    assert_eq!(db.selected_device(), Some(device(900)));
}

#[test]
fn a_reference_is_local_without_a_device_or_naming_the_selected_one() {
    let local = database(&[900, 813]).local_device();
    assert_eq!(local.identifier(), Some(device(813)));
    assert!(local.is_local(None));
    assert!(local.is_local(Some(device(813))));
    // Another Device in the same database is not the one this device is.
    assert!(!local.is_local(Some(device(900))));
    assert!(!local.is_local(Some(device(7))));
    // A non-Device identifier in the Device member names no device here.
    let analog = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 813).unwrap();
    assert!(!local.is_local(Some(analog)));
}

#[test]
fn without_a_device_only_unqualified_references_are_local() {
    let local = database(&[]).local_device();
    assert_eq!(local.identifier(), None);
    assert!(local.is_local(None));
    assert!(!local.is_local(Some(device(ObjectIdentifier::WILDCARD_INSTANCE))));
    assert!(!local.is_local(Some(device(813))));
}
