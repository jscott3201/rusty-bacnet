//! The Object_List and Database_Revision the database keeps (#2669).

use super::*;
use crate::analog::AnalogInputObject;
use crate::binary::BinaryValueObject;
use crate::device::{DeviceConfig, DeviceObject};
use crate::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::error::RemoteError;
use std::cell::Cell;

std::thread_local! {
    static WHOLE_READS: Cell<usize> = const { Cell::new(0) };
}

/// Counts each whole Object_List the database builds, on this thread.
pub(super) fn count_whole_read() {
    WHOLE_READS.with(|reads| reads.set(reads.get() + 1));
}

fn whole_reads() -> usize {
    WHOLE_READS.with(Cell::get)
}

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn device(instance: u32) -> Box<DeviceObject> {
    Box::new(
        DeviceObject::new(DeviceConfig {
            identity: DeviceIdentity::new(instance, 0)
                .unwrap()
                .with_name(format!("device-{instance}")),
            ..DeviceConfig::default()
        })
        .unwrap(),
    )
}

fn ai(instance: u32) -> Box<AnalogInputObject> {
    Box::new(
        AnalogInputObject::new(
            instance,
            format!("AI-{instance}"),
            EngineeringUnits::NO_UNITS,
        )
        .unwrap(),
    )
}

fn bv(instance: u32) -> Box<BinaryValueObject> {
    Box::new(BinaryValueObject::new(instance, format!("BV-{instance}")).unwrap())
}

fn protocol(error: Error) -> (ErrorClass, ErrorCode) {
    match error {
        Error::Remote(RemoteError::Error { class, code }) => (class, code),
        other => panic!("expected a protocol error, got {other:?}"),
    }
}

fn listed(db: &ObjectDatabase) -> Vec<ObjectIdentifier> {
    db.object_list().collect()
}

fn whole(db: &ObjectDatabase) -> Vec<ObjectIdentifier> {
    let PropertyValue::List(entries) = db
        .read_device_property(PropertyIdentifier::OBJECT_LIST, None)
        .unwrap()
    else {
        panic!("Object_List reads as a list");
    };
    entries
        .into_iter()
        .map(|entry| match entry {
            PropertyValue::ObjectIdentifier(oid) => oid,
            other => panic!("Object_List holds {other:?}"),
        })
        .collect()
}

/// The order an unchanged database lists is the one each builder seeded
/// before #2669: the Device, its ports, then the extra objects.
#[test]
fn a_built_database_lists_the_device_its_ports_then_its_objects() {
    let identity = DeviceIdentity::new(1001, 42)
        .unwrap()
        .with_bip_port(1, 10, core::net::Ipv4Addr::new(192, 168, 1, 10), 47808)
        .unwrap();
    let db = build_database_with_extra(&identity, vec![bv(2), ai(1)]).unwrap();
    let expected = [
        identity.device_oid(),
        oid(ObjectType::NETWORK_PORT, 1),
        oid(ObjectType::BINARY_VALUE, 2),
        oid(ObjectType::ANALOG_INPUT, 1),
    ];
    assert_eq!(listed(&db), expected);
    assert_eq!(whole(&db), expected);
    assert_eq!(db.database_revision(), 4);
}

/// A Device goes ahead of every object that isn't one, and after the
/// Devices already listed; any other object goes last. The Python server
/// adds its Device after its objects, and its list starts with the Device.
#[test]
fn devices_lead_and_the_rest_keep_the_order_they_came_in() {
    let mut db = ObjectDatabase::new();
    db.add(ai(3)).unwrap();
    db.add(bv(1)).unwrap();
    db.add(device(7)).unwrap();
    db.add(ai(1)).unwrap();
    db.add(device(2)).unwrap();
    assert_eq!(
        listed(&db),
        [
            oid(ObjectType::DEVICE, 7),
            oid(ObjectType::DEVICE, 2),
            oid(ObjectType::ANALOG_INPUT, 3),
            oid(ObjectType::BINARY_VALUE, 1),
            oid(ObjectType::ANALOG_INPUT, 1),
        ]
    );
    // Each entry by index, across the turn from the Devices to the rest.
    let entries = |db: &ObjectDatabase| {
        let count = db.len() as u32;
        (1..=count)
            .map(|index| {
                match db
                    .read_device_property(PropertyIdentifier::OBJECT_LIST, Some(index))
                    .unwrap()
                {
                    PropertyValue::ObjectIdentifier(entry) => entry,
                    other => panic!("an entry reads as one identifier, not {other:?}"),
                }
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(entries(&db), listed(&db));
    // A Device removed leaves the list; the next Device takes its place.
    drop(db.remove(&oid(ObjectType::DEVICE, 7)).unwrap());
    assert_eq!(
        listed(&db),
        [
            oid(ObjectType::DEVICE, 2),
            oid(ObjectType::ANALOG_INPUT, 3),
            oid(ObjectType::BINARY_VALUE, 1),
            oid(ObjectType::ANALOG_INPUT, 1),
        ]
    );
    assert_eq!(entries(&db), listed(&db));
}

/// A removed object leaves the list; one added again goes last; one
/// replaced under its identifier keeps its place. Each new identifier and
/// each removal moves the revision by one; a replacement, a refused add and
/// a removal of nothing don't.
#[test]
fn the_list_and_revision_follow_adds_and_removes() {
    let mut db = ObjectDatabase::new();
    db.add(device(1)).unwrap();
    db.add(ai(1)).unwrap();
    db.add(ai(2)).unwrap();
    db.add(bv(1)).unwrap();
    assert_eq!(db.database_revision(), 4);

    let gone = db.remove(&oid(ObjectType::ANALOG_INPUT, 2)).unwrap();
    assert!(gone.is_some());
    assert_eq!(db.database_revision(), 5);
    assert_eq!(
        listed(&db),
        [
            oid(ObjectType::DEVICE, 1),
            oid(ObjectType::ANALOG_INPUT, 1),
            oid(ObjectType::BINARY_VALUE, 1),
        ]
    );

    // Replaced: same place, same revision.
    let mut replaced = ai(1);
    replaced.set_present_value(5.0);
    db.add(replaced).unwrap();
    assert_eq!(db.database_revision(), 5);
    assert_eq!(listed(&db)[1], oid(ObjectType::ANALOG_INPUT, 1));

    // Back again: last.
    db.add(ai(2)).unwrap();
    assert_eq!(db.database_revision(), 6);
    assert_eq!(listed(&db).last(), Some(&oid(ObjectType::ANALOG_INPUT, 2)));

    // Refused (a name in use) and absent: nothing moves.
    let clash = Box::new(AnalogInputObject::new(9, "BV-1", EngineeringUnits::NO_UNITS).unwrap());
    assert!(db.add(clash).is_err());
    assert!(
        db.remove(&oid(ObjectType::ANALOG_INPUT, 9))
            .unwrap()
            .is_none()
    );
    assert_eq!(db.database_revision(), 6);
    assert_eq!(listed(&db).len(), db.len());
}

/// An application that keeps the revision sets it once built; changes
/// count on from there, and it wraps past `u32::MAX`.
#[test]
fn a_set_revision_counts_on_and_wraps() {
    let mut db = ObjectDatabase::new();
    db.add(device(1)).unwrap();
    db.set_database_revision(u32::MAX);
    assert_eq!(
        db.read_device_property(PropertyIdentifier::DATABASE_REVISION, None)
            .unwrap(),
        PropertyValue::Unsigned(u64::from(u32::MAX))
    );
    db.add(ai(1)).unwrap();
    assert_eq!(db.database_revision(), 0);
    db.remove(&oid(ObjectType::ANALOG_INPUT, 1)).unwrap();
    assert_eq!(db.database_revision(), 1);
}

/// Index 0 is the count, 1 the first entry and the count the last; one
/// past it, and the largest index, are INVALID_ARRAY_INDEX. Database_Revision
/// takes no index, and nothing else is the database's to serve.
#[test]
fn reads_by_index_and_their_refusals() {
    let mut db = ObjectDatabase::new();
    db.add(device(1)).unwrap();
    db.add(ai(1)).unwrap();
    db.add(bv(1)).unwrap();
    let read = |index| db.read_device_property(PropertyIdentifier::OBJECT_LIST, index);
    assert_eq!(read(Some(0)).unwrap(), PropertyValue::Unsigned(3));
    assert_eq!(
        read(Some(1)).unwrap(),
        PropertyValue::ObjectIdentifier(oid(ObjectType::DEVICE, 1))
    );
    assert_eq!(
        read(Some(3)).unwrap(),
        PropertyValue::ObjectIdentifier(oid(ObjectType::BINARY_VALUE, 1))
    );
    for past in [4, u32::MAX] {
        assert_eq!(
            protocol(read(Some(past)).unwrap_err()),
            (ErrorClass::PROPERTY, ErrorCode::INVALID_ARRAY_INDEX),
            "{past}"
        );
    }
    assert_eq!(
        protocol(
            db.read_device_property(PropertyIdentifier::DATABASE_REVISION, Some(1))
                .unwrap_err()
        ),
        (ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY)
    );
    assert_eq!(
        protocol(
            db.read_device_property(PropertyIdentifier::OBJECT_NAME, None)
                .unwrap_err()
        ),
        (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)
    );
}

/// On a large database a read of the count or of any one entry builds no
/// list: only a whole read does, once.
#[test]
fn a_large_database_reads_by_index_without_building_the_list() {
    let mut db = ObjectDatabase::new();
    db.add(device(1)).unwrap();
    for instance in 1..5000 {
        db.add(ai(instance)).unwrap();
    }
    let before = whole_reads();
    assert_eq!(
        db.read_device_property(PropertyIdentifier::OBJECT_LIST, Some(0))
            .unwrap(),
        PropertyValue::Unsigned(5000)
    );
    for index in [1, 2, 2500, 4999, 5000] {
        let PropertyValue::ObjectIdentifier(entry) = db
            .read_device_property(PropertyIdentifier::OBJECT_LIST, Some(index))
            .unwrap()
        else {
            panic!("an entry reads as one identifier");
        };
        assert_eq!(entry, listed(&db)[index as usize - 1]);
    }
    assert_eq!(whole_reads(), before, "no index read built the list");
    assert_eq!(whole(&db).len(), 5000);
    assert_eq!(whole_reads(), before + 1);
}

/// The database serves the two properties of the local Device only: the
/// bound one, or else the lowest Device; another Device and any other
/// property or object answer for themselves.
#[test]
fn only_the_local_devices_two_properties_are_kept() {
    let mut db = ObjectDatabase::new();
    db.add(device(5)).unwrap();
    db.add(device(3)).unwrap();
    db.add(ai(1)).unwrap();
    let keeps = |db: &ObjectDatabase, object: ObjectIdentifier, property| {
        db.keeps_device_property(db.get(&object).unwrap(), property)
    };
    let (low, high) = (oid(ObjectType::DEVICE, 3), oid(ObjectType::DEVICE, 5));
    for property in [
        PropertyIdentifier::OBJECT_LIST,
        PropertyIdentifier::DATABASE_REVISION,
    ] {
        assert!(keeps(&db, low, property));
        assert!(!keeps(&db, high, property));
        assert!(!keeps(&db, oid(ObjectType::ANALOG_INPUT, 1), property));
    }
    assert!(!keeps(&db, low, PropertyIdentifier::OBJECT_NAME));
    db.bind_local_device_internal(Some(high));
    assert!(keeps(&db, high, PropertyIdentifier::OBJECT_LIST));
    assert!(!keeps(&db, low, PropertyIdentifier::OBJECT_LIST));
}

/// A rename moves the revision: an Object_Name written and indexed, or an
/// object replaced under another name. Writing the name it already has, or
/// replacing it under the same name, doesn't.
#[test]
fn a_rename_moves_the_revision_and_the_same_name_does_not() {
    let mut db = ObjectDatabase::new();
    db.add(device(1)).unwrap();
    db.add(ai(1)).unwrap();
    let ai1 = oid(ObjectType::ANALOG_INPUT, 1);
    let name = |db: &mut ObjectDatabase, name: &str| {
        db.get_mut(&ai1)
            .unwrap()
            .write_property(
                PropertyIdentifier::OBJECT_NAME,
                None,
                PropertyValue::CharacterString(name.into()),
                None,
            )
            .unwrap();
        db.update_name_index(&ai1);
        db.database_revision()
    };
    assert_eq!(name(&mut db, "AI-1"), 2);
    assert_eq!(name(&mut db, "Zone temperature"), 3);
    assert_eq!(name(&mut db, "Zone temperature"), 3);
    assert!(db.find_by_name("Zone temperature").is_some());
    // Replaced under its identifier: another name moves it, the same not.
    db.add(ai(1)).unwrap();
    assert_eq!(db.database_revision(), 4);
    db.add(ai(1)).unwrap();
    assert_eq!(db.database_revision(), 4);
}

/// What follows a reference to a local property reads the local Device's
/// two properties from the database, and anything else from the object.
#[test]
fn a_reference_reads_the_device_lists_from_the_database() {
    let mut db = ObjectDatabase::new();
    db.add(device(1)).unwrap();
    db.add(ai(1)).unwrap();
    let read = |db: &ObjectDatabase, object, property, index| {
        db.read_object_property(db.get(&object).unwrap(), property, index)
            .unwrap()
    };
    let device = oid(ObjectType::DEVICE, 1);
    assert_eq!(
        read(&db, device, PropertyIdentifier::DATABASE_REVISION, None),
        PropertyValue::Unsigned(2)
    );
    assert_eq!(
        read(&db, device, PropertyIdentifier::OBJECT_LIST, Some(0)),
        PropertyValue::Unsigned(2)
    );
    assert_eq!(
        read(&db, device, PropertyIdentifier::OBJECT_NAME, None),
        PropertyValue::CharacterString("device-1".into())
    );
    assert_eq!(
        read(
            &db,
            oid(ObjectType::ANALOG_INPUT, 1),
            PropertyIdentifier::OBJECT_NAME,
            None
        ),
        PropertyValue::CharacterString("AI-1".into())
    );
}

/// While the local Device is in a backup or a restore, a File added,
/// renamed or removed leaves the revision alone: the procedures' own
/// configuration Files (Clauses 12.11.35, 19.1). Any other object still
/// moves it, and once the Device is IDLE again so does a File.
#[test]
fn configuration_files_in_a_backup_or_restore_leave_the_revision() {
    use bacnet_types::enums::BackupAndRestoreState as State;
    let mut db = ObjectDatabase::new();
    let device = DeviceObject::new(DeviceConfig {
        identity: DeviceIdentity::new(1, 0).unwrap(),
        backup_and_restore: true,
        ..DeviceConfig::default()
    })
    .unwrap();
    let device_oid = device.object_identifier();
    db.add(Box::new(device)).unwrap();
    let set_state = |db: &mut ObjectDatabase, state| {
        db.get_mut(&device_oid)
            .and_then(|device| device.device_authority_internal())
            .unwrap()
            .set_backup_and_restore_state(state);
    };
    let file = |instance| {
        Box::new(
            crate::file::FileObject::new(instance, format!("cfg-{instance}"), "config").unwrap(),
        )
    };
    let file1 = oid(ObjectType::FILE, 1);
    for state in [State::PERFORMING_A_BACKUP, State::PREPARING_FOR_RESTORE] {
        set_state(&mut db, state);
        let before = db.database_revision();
        db.add(file(1)).unwrap();
        // Replaced under another name: a rename.
        db.add(Box::new(
            crate::file::FileObject::new(1, "renamed", "config").unwrap(),
        ))
        .unwrap();
        drop(db.remove(&file1).unwrap());
        assert_eq!(db.database_revision(), before, "{state:?}");
        // Anything but a File still counts.
        db.add(ai(7)).unwrap();
        assert_eq!(db.database_revision(), before.wrapping_add(1), "{state:?}");
        drop(db.remove(&oid(ObjectType::ANALOG_INPUT, 7)).unwrap());
    }
    set_state(&mut db, State::IDLE);
    let before = db.database_revision();
    db.add(file(1)).unwrap();
    drop(db.remove(&file1).unwrap());
    assert_eq!(db.database_revision(), before.wrapping_add(2));
}
