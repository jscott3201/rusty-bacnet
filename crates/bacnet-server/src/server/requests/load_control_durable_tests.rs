//! A Load Control's kept writes save through the server (#2479): a
//! WriteProperty of a request property, Enable, Description or Shed_Levels
//! (#2502) is staged, and its save runs on the object's writer thread while
//! the database guard is dropped, so the database stays readable and
//! writable meanwhile.

use super::durable_write_wire_tests::{HeldStorage, SIMPLE_ACK_WRITE, WAIT, while_saving};
use super::mutation_list_wire_tests::wire;
use super::mutation_tests::{Fixture, value};
use super::*;
use PropertyIdentifier as P;
use bacnet_objects::load_control::{
    LoadControlObject, LoadControlPersistence, LoadControlSnapshot,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::BACnetShedLevel;
use std::sync::atomic::Ordering;

type ShedStorage = HeldStorage<LoadControlSnapshot>;

impl LoadControlPersistence for ShedStorage {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<LoadControlSnapshot>, Error> {
        Ok(self.load_saved())
    }

    fn save(&self, _: ObjectIdentifier, snapshot: &LoadControlSnapshot) -> Result<(), Error> {
        self.store(snapshot)
    }
}

/// A request fixture whose database holds LC-1, kept in `storage`.
async fn served_by(storage: &Arc<ShedStorage>) -> (Arc<Fixture>, ObjectIdentifier) {
    let fixture = Fixture::new(None);
    let object = LoadControlObject::with_persistence(
        1,
        "LC-1",
        Arc::clone(storage) as Arc<dyn LoadControlPersistence>,
    )
    .unwrap();
    let oid = object.object_identifier();
    fixture.db.write().await.add(Box::new(object)).unwrap();
    (Arc::new(fixture), oid)
}

fn write_property(oid: ObjectIdentifier, property: P, property_value: Vec<u8>) -> Bytes {
    write_element(oid, property, None, property_value)
}

fn write_element(
    oid: ObjectIdentifier,
    property: P,
    property_array_index: Option<u32>,
    property_value: Vec<u8>,
) -> Bytes {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index,
        property_value,
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    request.freeze()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_kept_write_saves_off_the_lock_and_survives_a_rebuild() {
    for (property, octets, served, kept) in [
        (
            P::REQUESTED_SHED_LEVEL,
            vec![0x09, 80],
            PropertyValue::ApplicationData(vec![0x09, 80]),
            LoadControlSnapshot {
                requested_shed_level: BACnetShedLevel::Percent(80),
                ..LoadControlSnapshot::default()
            },
        ),
        (
            P::DUTY_WINDOW,
            value(PropertyValue::Unsigned(15)),
            PropertyValue::Unsigned(15),
            LoadControlSnapshot {
                duty_window: 15,
                ..LoadControlSnapshot::default()
            },
        ),
        (
            P::ENABLE,
            value(PropertyValue::Boolean(false)),
            PropertyValue::Boolean(false),
            LoadControlSnapshot {
                enable: false,
                ..LoadControlSnapshot::default()
            },
        ),
        (
            P::DESCRIPTION,
            value(PropertyValue::CharacterString("Chillers".into())),
            PropertyValue::CharacterString("Chillers".into()),
            LoadControlSnapshot {
                description: Some("Chillers".into()),
                ..LoadControlSnapshot::default()
            },
        ),
    ] {
        let storage = Arc::new(ShedStorage::default());
        let (fixture, oid) = served_by(&storage).await;
        let request = write_property(oid, property, octets);
        let service = ConfirmedServiceChoice::WRITE_PROPERTY;
        let (response, available) = while_saving(&fixture, &storage, service, request, WAIT).await;
        assert_eq!(response, SIMPLE_ACK_WRITE, "{property:?}");
        assert!(available, "the database was held while {property:?} saved");
        // The write took the staged save; it did not save again.
        assert_eq!(storage.saves.load(Ordering::SeqCst), 1, "{property:?}");
        assert_eq!(storage.load_saved(), Some(kept), "{property:?}");
        assert_eq!(fixture.read(oid, property).await, served);
        drop(fixture);
        let (rebuilt, oid) = served_by(&storage).await;
        assert_eq!(rebuilt.read(oid, property).await, served);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_whose_save_fails_is_refused_and_changes_nothing() {
    let storage = Arc::new(ShedStorage::default());
    let (fixture, oid) = served_by(&storage).await;
    storage.fail.store(true, Ordering::SeqCst);
    let request = write_property(oid, P::DUTY_WINDOW, value(PropertyValue::Unsigned(15)));
    let service = ConfirmedServiceChoice::WRITE_PROPERTY;
    let (response, available) = while_saving(&fixture, &storage, service, request, WAIT).await;
    // DEVICE (0) / OPERATIONAL_PROBLEM (25).
    assert_eq!(response, [0x50, 5, 15, 0x91, 0, 0x91, 25]);
    assert!(available);
    assert_eq!(
        fixture.read(oid, P::DUTY_WINDOW).await,
        PropertyValue::Unsigned(0)
    );
    assert_eq!(storage.load_saved(), None);
}

/// [`served_by`], the object given Shed_Levels 1 and 2 as it is built.
async fn with_levels(storage: &Arc<ShedStorage>) -> (Arc<Fixture>, ObjectIdentifier) {
    let fixture = Fixture::new(None);
    let mut object = LoadControlObject::with_persistence(
        1,
        "LC-1",
        Arc::clone(storage) as Arc<dyn LoadControlPersistence>,
    )
    .unwrap();
    object
        .set_shed_levels(vec![(1, "one".into()), (2, "two".into())])
        .unwrap();
    let oid = object.object_identifier();
    fixture.db.write().await.add(Box::new(object)).unwrap();
    (Arc::new(fixture), oid)
}

fn levels(values: &[u64]) -> PropertyValue {
    PropertyValue::List(values.iter().map(|&v| PropertyValue::Unsigned(v)).collect())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shed_levels_element_saves_off_the_lock_and_survives_a_rebuild() {
    let storage = Arc::new(ShedStorage::default());
    let (fixture, oid) = with_levels(&storage).await;
    let service = ConfirmedServiceChoice::WRITE_PROPERTY;
    // Refused by the object before anything is staged: an index past the
    // end (PROPERTY / INVALID_ARRAY_INDEX, 42) and a level of 0 (PROPERTY /
    // VALUE_OUT_OF_RANGE, 37).
    for (index, level, code) in [(3, 4, 42), (1, 0, 37)] {
        let request = write_element(
            oid,
            P::SHED_LEVELS,
            Some(index),
            value(PropertyValue::Unsigned(level)),
        );
        let response = wire(&fixture, service, request).await;
        assert_eq!(response, [0x50, 5, 15, 0x91, 2, 0x91, code], "{index}");
    }
    assert_eq!(storage.saves.load(Ordering::SeqCst), 0);
    let request = write_element(
        oid,
        P::SHED_LEVELS,
        Some(2),
        value(PropertyValue::Unsigned(9)),
    );
    let (response, available) = while_saving(&fixture, &storage, service, request, WAIT).await;
    assert_eq!(response, SIMPLE_ACK_WRITE);
    assert!(available, "the database was held while Shed_Levels saved");
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
    assert_eq!(storage.load_saved().unwrap().shed_levels, [(2, 9)]);
    assert_eq!(fixture.read(oid, P::SHED_LEVELS).await, levels(&[1, 9]));
    drop(fixture);
    let (rebuilt, oid) = with_levels(&storage).await;
    assert_eq!(rebuilt.read(oid, P::SHED_LEVELS).await, levels(&[1, 9]));
}
