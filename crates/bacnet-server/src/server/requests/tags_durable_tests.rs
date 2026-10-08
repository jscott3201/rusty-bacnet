//! Durable Tags through actual confirmed dispatch for all four profile objects.

use super::durable_stop_tests::{send, stop_then_release};
use super::durable_write_wire_tests::{
    while_saving, HeldStorage, SIMPLE_ACK_WPM, SIMPLE_ACK_WRITE, WAIT,
};
use super::mutation_list_wire_tests::wire;
use super::mutation_tests::{apdu, wpm, Fixture};
use super::*;
use crate::server::test_transport::TestTransport;
use bacnet_objects::color::{ColorObject, ColorTemperatureObject};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::lighting::{BinaryLightingOutputObject, LightingOutputObject};
use bacnet_objects::object_profile::{ObjectProfile, TagsPersistence, TagsSnapshot};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::{BACnetNameValue, TagValue};
use std::sync::atomic::Ordering;

type Storage = HeldStorage<TagsSnapshot>;
const TAGS: PropertyIdentifier = PropertyIdentifier::TAGS;
const KINDS: [ObjectType; 4] = [
    ObjectType::COLOR,
    ObjectType::COLOR_TEMPERATURE,
    ObjectType::LIGHTING_OUTPUT,
    ObjectType::BINARY_LIGHTING_OUTPUT,
];
// Independently authored BACnetNameValue vectors: semantic exhaust and floor=3.
const EXHAUST: &[u8] = &[0x0d, 8, 0, b'e', b'x', b'h', b'a', b'u', b's', b't'];
const FLOOR: &[u8] = &[0x0d, 6, 0, b'f', b'l', b'o', b'o', b'r', 0x21, 3];
const EMPTY_TAG: &[u8] = &[0x09, 0];

impl TagsPersistence for Storage {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error> {
        Ok(self.load_saved())
    }
    fn save(&self, _: ObjectIdentifier, snapshot: &TagsSnapshot) -> Result<(), Error> {
        self.store(snapshot)
    }
}

fn object(kind: ObjectType, storage: &Arc<Storage>) -> Box<dyn BACnetObject> {
    let profile = ObjectProfile {
        tags: Some(vec![BACnetNameValue::semantic("exhaust")]),
        ..ObjectProfile::default()
    };
    macro_rules! build {
        ($ty:ty) => {{
            let mut object = <$ty>::with_tags_persistence(1, "tags", storage.clone()).unwrap();
            object.set_profile(profile).unwrap();
            Box::new(object)
        }};
    }
    match kind {
        ObjectType::COLOR => build!(ColorObject),
        ObjectType::COLOR_TEMPERATURE => build!(ColorTemperatureObject),
        ObjectType::LIGHTING_OUTPUT => build!(LightingOutputObject),
        ObjectType::BINARY_LIGHTING_OUTPUT => build!(BinaryLightingOutputObject),
        _ => unreachable!(),
    }
}

async fn served_by(kind: ObjectType, storage: &Arc<Storage>) -> (Arc<Fixture>, ObjectIdentifier) {
    let fixture = Arc::new(Fixture::new(None));
    let object = object(kind, storage);
    let oid = object.object_identifier();
    fixture.db.write().await.add(object).unwrap();
    (fixture, oid)
}

fn request(oid: ObjectIdentifier, index: Option<u32>, octets: &[u8]) -> Bytes {
    let mut bytes = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: TAGS,
        property_array_index: index,
        property_value: octets.to_vec(),
        priority: None,
    }
    .encode(&mut bytes)
    .unwrap();
    bytes.freeze()
}

fn attempt(index: Option<u32>, bytes: &[u8]) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: TAGS,
        property_array_index: index,
        value: bytes.to_vec(),
        priority: None,
    }
}

fn multiple(oid: ObjectIdentifier, writes: Vec<BACnetPropertyValue>) -> Bytes {
    wpm(vec![WriteAccessSpecification {
        object_identifier: oid,
        list_of_properties: writes,
    }])
}

async fn read_wire(fixture: &Fixture, oid: ObjectIdentifier, index: Option<u32>) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: TAGS,
        property_array_index: index,
    }
    .encode(&mut bytes);
    let response = fixture
        .dispatch(ConfirmedServiceChoice::READ_PROPERTY, bytes.freeze(), 6)
        .await
        .unwrap();
    let Apdu::ComplexAck(response) = apdu(response) else {
        panic!("expected ReadProperty ACK")
    };
    assert_eq!(response.invoke_id, 6);
    assert_eq!(
        response.service_choice,
        ConfirmedServiceChoice::READ_PROPERTY
    );
    ReadPropertyACK::decode(&response.service_ack)
        .unwrap()
        .property_value
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_four_whole_resize_and_element_writes_survive_reconstruction() {
    for kind in KINDS {
        for (index, input, expected) in [
            (None, FLOOR.to_vec(), FLOOR.to_vec()),
            (Some(0), vec![0x21, 2], [EXHAUST, EMPTY_TAG].concat()),
            (Some(1), FLOOR.to_vec(), FLOOR.to_vec()),
            (None, vec![], vec![]),
        ] {
            let storage = Arc::new(Storage::default());
            let (fixture, oid) = served_by(kind, &storage).await;
            let (response, available) = while_saving(
                &fixture,
                &storage,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                request(oid, index, &input),
                WAIT,
            )
            .await;
            assert_eq!(response, SIMPLE_ACK_WRITE);
            assert!(available, "database held during {kind:?} save");
            assert_eq!(read_wire(&fixture, oid, None).await, expected);
            assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
            drop(fixture);
            let (rebuilt, oid) = served_by(kind, &storage).await;
            assert_eq!(read_wire(&rebuilt, oid, None).await, expected);
            assert_eq!(
                read_wire(&rebuilt, oid, Some(0)).await,
                [
                    0x21,
                    if input.is_empty() {
                        0
                    } else if index == Some(0) {
                        2
                    } else {
                        1
                    }
                ]
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_four_wpm_folds_ordered_indices_into_one_final_save() {
    for kind in KINDS {
        let storage = Arc::new(Storage::default());
        let (fixture, oid) = served_by(kind, &storage).await;
        let writes = vec![
            attempt(None, FLOOR),
            attempt(Some(0), &[0x21, 2]),
            attempt(Some(2), EXHAUST),
            attempt(Some(1), EXHAUST),
            attempt(Some(2), FLOOR),
        ];
        assert_eq!(
            wire(
                &fixture,
                ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
                multiple(oid, writes)
            )
            .await,
            SIMPLE_ACK_WPM
        );
        assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
        assert_eq!(
            read_wire(&fixture, oid, None).await,
            [EXHAUST, FLOOR].concat()
        );
        drop(fixture);
        let (rebuilt, oid) = served_by(kind, &storage).await;
        assert_eq!(
            read_wire(&rebuilt, oid, None).await,
            [EXHAUST, FLOOR].concat()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wpm_invalid_middle_and_unrelated_failure_preserve_successful_prefix() {
    for unrelated in [false, true] {
        let storage = Arc::new(Storage::default());
        let (fixture, oid) = served_by(ObjectType::COLOR, &storage).await;
        let mut refused = attempt(Some(9), FLOOR);
        if unrelated {
            refused.property_identifier = PropertyIdentifier::PROFILE_NAME;
            refused.property_array_index = None;
        }
        let response = wire(
            &fixture,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            multiple(
                oid,
                vec![attempt(None, FLOOR), refused, attempt(None, EXHAUST)],
            ),
        )
        .await;
        assert_eq!(response[0], 0x50);
        let wait = fixture
            .db
            .write()
            .await
            .get_mut(&oid)
            .unwrap()
            .durable_writes_internal()
            .unwrap()
            .settle_forgotten_writes()
            .unwrap();
        tokio::task::spawn_blocking(move || wait.block())
            .await
            .unwrap();
        assert_eq!(read_wire(&fixture, oid, None).await, FLOOR);
        drop(fixture);
        let (rebuilt, oid) = served_by(ObjectType::COLOR, &storage).await;
        assert_eq!(read_wire(&rebuilt, oid, None).await, FLOOR);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_save_refuses_wire_write_without_publishing_or_replacing_storage() {
    for kind in KINDS {
        let storage = Arc::new(Storage::default());
        let (fixture, oid) = served_by(kind, &storage).await;
        storage.fail.store(true, Ordering::SeqCst);
        let response = wire(
            &fixture,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            request(oid, None, FLOOR),
        )
        .await;
        assert_eq!(response, [0x50, 5, 15, 0x91, 0, 0x91, 25]);
        assert_eq!(read_wire(&fixture, oid, None).await, EXHAUST);
        assert_eq!(storage.load_saved(), None);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unrelated_confirmed_read_completes_while_tags_save_and_ack_are_held() {
    let storage = Arc::new(Storage::default());
    let (fixture, oid) = served_by(ObjectType::COLOR, &storage).await;
    let (started, go) = storage.hold();
    let writing = tokio::spawn({
        let fixture = fixture.clone();
        async move {
            wire(
                &fixture,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                request(oid, None, FLOOR),
            )
            .await
        }
    });
    tokio::task::spawn_blocking(move || started.recv_timeout(WAIT))
        .await
        .unwrap()
        .unwrap();
    assert!(!writing.is_finished());
    let before = tokio::time::timeout(WAIT, read_wire(&fixture, oid, None))
        .await
        .unwrap();
    assert_eq!(before, EXHAUST);
    assert!(!writing.is_finished());
    drop(go);
    assert_eq!(writing.await.unwrap(), SIMPLE_ACK_WRITE);
    assert_eq!(read_wire(&fixture, oid, None).await, FLOOR);
}

async fn server(
    kind: ObjectType,
    storage: &Arc<Storage>,
) -> (
    BACnetServer<TestTransport>,
    tokio::sync::mpsc::Sender<bacnet_transport::port::ReceivedNpdu>,
) {
    let (transport, inbound) = TestTransport::inbound(4);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 100,
            name: "Tags device".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(object(kind, storage)).unwrap();
    let server = BACnetServer::generic_builder()
        .transport(transport)
        .database(db)
        .enable_event_enrollment(false)
        .build()
        .await
        .unwrap();
    (server, inbound)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_four_stop_cancels_held_write_and_waits_for_successful_correction() {
    for kind in KINDS {
        let storage = Arc::new(Storage::default());
        let served = TagsSnapshot {
            tags: Some(vec![BACnetNameValue::valued(
                "floor",
                TagValue::Primitive(PropertyValue::Unsigned(3)),
            )]),
        };
        *storage.saved.lock().unwrap() = Some(served.clone());
        let (server, inbound) = server(kind, &storage).await;
        let oid = ObjectIdentifier::new(kind, 1).unwrap();
        let (started, go) = storage.hold();
        send(
            &inbound,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            request(oid, None, EXHAUST),
        )
        .await;
        tokio::task::spawn_blocking(move || started.recv_timeout(WAIT))
            .await
            .unwrap()
            .unwrap();
        let server = stop_then_release(server, go).await;
        assert_eq!(storage.load_saved(), Some(served));
        assert_eq!(storage.saves.load(Ordering::SeqCst), 2);
        drop(server);
        let (rebuilt, oid) = served_by(kind, &storage).await;
        assert_eq!(read_wire(&rebuilt, oid, None).await, FLOOR);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_local_waits_off_the_database_lock_and_survives_rebuild() {
    let storage = Arc::new(Storage::default());
    let (server, _inbound) = server(ObjectType::COLOR, &storage).await;
    let server = Arc::new(server);
    let oid = ObjectIdentifier::new(ObjectType::COLOR, 1).unwrap();
    let (started, go) = storage.hold();
    let writing = tokio::spawn({
        let server = server.clone();
        async move {
            server
                .write_local(
                    &oid,
                    TAGS,
                    None,
                    PropertyValue::ApplicationData(FLOOR.to_vec()),
                    None,
                    crate::LocalCommandSource::ServerDevice,
                )
                .await
        }
    });
    tokio::task::spawn_blocking(move || started.recv_timeout(WAIT))
        .await
        .unwrap()
        .unwrap();
    assert!(tokio::time::timeout(WAIT, server.database().write())
        .await
        .is_ok());
    drop(go);
    writing.await.unwrap().unwrap();
    let mut server = Arc::into_inner(server).expect("writer released server");
    server.stop().await.unwrap();
    drop(server);
    let (rebuilt, oid) = served_by(ObjectType::COLOR, &storage).await;
    assert_eq!(read_wire(&rebuilt, oid, None).await, FLOOR);
}
