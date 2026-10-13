//! An Access Rights object's rule arrays and Enable save through the server
//! (#1392): WriteProperty, WritePropertyMultiple and `write_local` stage the
//! save, which runs on the object's writer thread while the database guard
//! is dropped. A state that cannot be saved is refused, a saved one is what
//! a rebuilt server serves, and `stop()` leaves storage with what the object
//! served.
//!
//! DEVICE 0; OPERATIONAL_PROBLEM 25.

use super::durable_stop_tests::{send, stop_then_release};
use super::durable_write_wire_tests::{
    while_saving, HeldStorage, ERROR_PDU, SIMPLE_ACK_WPM, SIMPLE_ACK_WRITE, WAIT,
};
use super::mutation_list_wire_tests::wire;
use super::mutation_tests::{value, wpm, Fixture};
use super::*;
use crate::server::test_transport::TestTransport;
use bacnet_encoding::constructed::encode_access_rule;
use bacnet_objects::access_control::{
    AccessRightsObject, AccessRightsPersistence, AccessRightsSnapshot,
};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::{BACnetAccessRule, BACnetDeviceObjectPropertyReference};
use bacnet_types::enums::{AccessRuleLocationSpecifier, AccessRuleTimeRangeSpecifier};
use std::sync::atomic::Ordering;
use std::time::Duration;
use PropertyIdentifier as P;

type RightsStorage = HeldStorage<AccessRightsSnapshot>;

impl AccessRightsPersistence for RightsStorage {
    fn load(&self, _rights: ObjectIdentifier) -> Result<Option<AccessRightsSnapshot>, Error> {
        Ok(self.load_saved())
    }

    fn save(
        &self,
        _rights: ObjectIdentifier,
        snapshot: &AccessRightsSnapshot,
    ) -> Result<(), Error> {
        self.store(snapshot)
    }
}

/// The Error PDU a WriteProperty refused with DEVICE / OPERATIONAL_PROBLEM
/// gets: invoke ID 5, the service, then the class and code.
const WRITE_REFUSED: [u8; 7] = [0x50, 5, 15, 0x91, 0, 0x91, 25];

/// A rule that applies at any time in Access Zone `instance`.
fn zone_rule(instance: u32) -> BACnetAccessRule {
    let zone = ObjectIdentifier::new(ObjectType::ACCESS_ZONE, instance).unwrap();
    BACnetAccessRule::new(None, Some(zone.into()), true)
}

/// The rule an index-0 write appends: SPECIFIED with unspecified references
/// (a Schedule's Present_Value and an Access Point), disabled.
fn grown_rule() -> BACnetAccessRule {
    let unspecified =
        |object_type| ObjectIdentifier::new(object_type, ObjectIdentifier::MAX_INSTANCE).unwrap();
    BACnetAccessRule {
        time_range_specifier: AccessRuleTimeRangeSpecifier::SPECIFIED,
        time_range: Some(BACnetDeviceObjectPropertyReference::new_local(
            unspecified(ObjectType::SCHEDULE),
            P::PRESENT_VALUE.to_raw(),
        )),
        location_specifier: AccessRuleLocationSpecifier::SPECIFIED,
        location: Some(unspecified(ObjectType::ACCESS_POINT).into()),
        enable: false,
    }
}

fn encoded(rules: &[BACnetAccessRule]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    for rule in rules {
        encode_access_rule(&mut buf, rule);
    }
    buf.to_vec()
}

/// A rule array as a whole read serves it.
fn served_rules(rules: &[BACnetAccessRule]) -> PropertyValue {
    PropertyValue::List(
        rules
            .iter()
            .map(|rule| PropertyValue::ApplicationData(encoded(std::slice::from_ref(rule))))
            .collect(),
    )
}

/// Storage that already holds `snapshot`, as saved by an earlier start.
fn holding(snapshot: AccessRightsSnapshot) -> Arc<RightsStorage> {
    let storage = Arc::new(RightsStorage::default());
    *storage.saved.lock().unwrap() = Some(snapshot);
    storage
}

fn positive_only(rules: &[BACnetAccessRule]) -> AccessRightsSnapshot {
    AccessRightsSnapshot {
        positive_access_rules: Some(rules.to_vec()),
        ..AccessRightsSnapshot::default()
    }
}

/// Access Rights 1, kept in `storage`.
fn rights_object(storage: &Arc<RightsStorage>) -> AccessRightsObject {
    AccessRightsObject::with_persistence(
        1,
        "AR",
        Arc::clone(storage) as Arc<dyn AccessRightsPersistence>,
    )
    .unwrap()
}

/// A request fixture whose database holds [`rights_object`].
async fn served_by(storage: &Arc<RightsStorage>) -> (Arc<Fixture>, ObjectIdentifier) {
    let fixture = Fixture::new(None);
    let rights = rights_object(storage);
    let oid = rights.object_identifier();
    fixture.db.write().await.add(Box::new(rights)).unwrap();
    (Arc::new(fixture), oid)
}

/// The three saved properties as `fixture` serves them.
async fn reads(fixture: &Fixture, rights: ObjectIdentifier) -> Vec<PropertyValue> {
    let mut values = Vec::new();
    for property in [
        P::POSITIVE_ACCESS_RULES,
        P::NEGATIVE_ACCESS_RULES,
        P::LOG_ENABLE,
    ] {
        values.push(fixture.read(rights, property).await);
    }
    values
}

/// The reads a server serves while storage holds `snapshot`: an array no
/// write set is empty, and Enable defaults to TRUE.
fn expected_reads(snapshot: &AccessRightsSnapshot) -> Vec<PropertyValue> {
    let rules =
        |rules: &Option<Vec<BACnetAccessRule>>| served_rules(rules.as_deref().unwrap_or(&[]));
    vec![
        rules(&snapshot.positive_access_rules),
        rules(&snapshot.negative_access_rules),
        PropertyValue::Boolean(snapshot.enable.unwrap_or(true)),
    ]
}

fn write_property(
    rights: ObjectIdentifier,
    property: P,
    index: Option<u32>,
    property_value: Vec<u8>,
) -> Bytes {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: rights,
        property_identifier: property,
        property_array_index: index,
        property_value,
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    request.freeze()
}

fn attempt(property: P, index: Option<u32>, value: Vec<u8>) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: property,
        property_array_index: index,
        value,
        priority: None,
    }
}

fn write_property_multiple(rights: ObjectIdentifier, attempts: Vec<BACnetPropertyValue>) -> Bytes {
    wpm(vec![WriteAccessSpecification {
        object_identifier: rights,
        list_of_properties: attempts,
    }])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_saved_write_property_saves_off_the_lock_and_survives_a_rebuild() {
    let earlier = positive_only(&[zone_rule(1), zone_rule(2)]);
    for (property, index, octets, expected) in [
        (
            P::POSITIVE_ACCESS_RULES,
            None,
            encoded(&[zone_rule(3), zone_rule(4)]),
            positive_only(&[zone_rule(3), zone_rule(4)]),
        ),
        (
            P::POSITIVE_ACCESS_RULES,
            Some(2),
            encoded(&[zone_rule(5)]),
            positive_only(&[zone_rule(1), zone_rule(5)]),
        ),
        // An index-0 write that grows the array.
        (
            P::NEGATIVE_ACCESS_RULES,
            Some(0),
            value(PropertyValue::Unsigned(2)),
            AccessRightsSnapshot {
                negative_access_rules: Some(vec![grown_rule(), grown_rule()]),
                ..earlier.clone()
            },
        ),
        (
            P::LOG_ENABLE,
            None,
            value(PropertyValue::Boolean(false)),
            AccessRightsSnapshot {
                enable: Some(false),
                ..earlier.clone()
            },
        ),
    ] {
        let storage = holding(earlier.clone());
        let (fixture, rights) = served_by(&storage).await;
        let request = write_property(rights, property, index, octets);
        let service = ConfirmedServiceChoice::WRITE_PROPERTY;
        let (response, available) = while_saving(&fixture, &storage, service, request, WAIT).await;
        assert_eq!(response, SIMPLE_ACK_WRITE, "{property:?} {index:?}");
        assert!(available, "the database was held while {property:?} saved");
        // The write took the staged save; it did not save again.
        assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
        assert_eq!(storage.load_saved(), Some(expected.clone()));
        assert_eq!(reads(&fixture, rights).await, expected_reads(&expected));
        drop(fixture);
        let (rebuilt, rights) = served_by(&storage).await;
        assert_eq!(reads(&rebuilt, rights).await, expected_reads(&expected));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_property_multiple_with_several_rights_writes_saves_once_and_survives_a_rebuild() {
    let storage = Arc::new(RightsStorage::default());
    let (fixture, rights) = served_by(&storage).await;
    let positive = [zone_rule(1), zone_rule(2)];
    let request = write_property_multiple(
        rights,
        vec![
            attempt(P::POSITIVE_ACCESS_RULES, None, encoded(&positive)),
            attempt(
                P::NEGATIVE_ACCESS_RULES,
                Some(0),
                value(PropertyValue::Unsigned(1)),
            ),
            attempt(P::LOG_ENABLE, None, value(PropertyValue::Boolean(false))),
        ],
    );
    let (started, go) = storage.hold();
    let sending = tokio::spawn({
        let fixture = Arc::clone(&fixture);
        async move {
            wire(
                &fixture,
                ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
                request,
            )
            .await
        }
    });
    tokio::task::spawn_blocking(move || started.recv_timeout(WAIT))
        .await
        .unwrap()
        .expect("the save started");
    // The request stages its three writes to the object as one save, of the
    // state they leave together, made with the guard dropped (#1423).
    let readable = tokio::time::timeout(WAIT, fixture.db.read()).await.is_ok();
    let writable = tokio::time::timeout(WAIT, fixture.db.write()).await.is_ok();
    drop(go);
    assert_eq!(sending.await.unwrap(), SIMPLE_ACK_WPM);
    assert!(readable && writable, "the database was held while it saved");
    // Each write took its step of that save; none saved again.
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
    let expected = AccessRightsSnapshot {
        positive_access_rules: Some(positive.to_vec()),
        negative_access_rules: Some(vec![grown_rule()]),
        enable: Some(false),
        accompaniment: None,
    };
    assert_eq!(storage.load_saved(), Some(expected.clone()));
    drop(fixture);
    let (rebuilt, rights) = served_by(&storage).await;
    assert_eq!(reads(&rebuilt, rights).await, expected_reads(&expected));
}

/// A started server holding [`rights_object`] and a Device, and the channel
/// that feeds it requests.
async fn server(
    storage: &Arc<RightsStorage>,
) -> (
    BACnetServer<TestTransport>,
    tokio::sync::mpsc::Sender<bacnet_transport::port::ReceivedNpdu>,
) {
    let (transport, inbound) = TestTransport::inbound(4);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 100,
            name: "Access Rights device".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(rights_object(storage))).unwrap();
    let server = BACnetServer::generic_builder()
        .transport(transport)
        .database(db)
        .enable_event_enrollment(false)
        .build()
        .await
        .unwrap();
    (server, inbound)
}

fn rights_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ACCESS_RIGHTS, 1).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_local_saves_off_the_lock_and_survives_a_rebuild() {
    let storage = Arc::new(RightsStorage::default());
    let (server, _inbound) = server(&storage).await;
    let server = Arc::new(server);
    let negative = [zone_rule(7)];
    let (started, go) = storage.hold();
    let writing = tokio::spawn({
        let server = Arc::clone(&server);
        let value = PropertyValue::ApplicationData(encoded(&negative));
        async move {
            server
                .write_local(
                    &rights_oid(),
                    P::NEGATIVE_ACCESS_RULES,
                    None,
                    value,
                    None,
                    crate::LocalCommandSource::ServerDevice,
                )
                .await
        }
    });
    tokio::task::spawn_blocking(move || started.recv_timeout(WAIT))
        .await
        .unwrap()
        .expect("the save started");
    let db = Arc::clone(server.database());
    let readable = tokio::time::timeout(WAIT, db.read()).await.is_ok();
    let writable = tokio::time::timeout(WAIT, db.write()).await.is_ok();
    go.send(()).unwrap();
    writing.await.unwrap().unwrap();
    assert!(readable && writable, "the database was held while it saved");
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
    drop(db);
    let mut server = Arc::into_inner(server).expect("no other handle on the server");
    server.stop().await.unwrap();
    drop(server);
    let (rebuilt, rights) = served_by(&storage).await;
    assert_eq!(
        rebuilt.read(rights, P::NEGATIVE_ACCESS_RULES).await,
        served_rules(&negative)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_state_that_cannot_be_saved_is_refused_on_every_path_and_nothing_changes() {
    let kept = positive_only(&[zone_rule(1), zone_rule(2)]);
    let storage = holding(kept.clone());
    let (fixture, rights) = served_by(&storage).await;
    storage.fail.store(true, Ordering::SeqCst);
    for (property, index, octets) in [
        (P::POSITIVE_ACCESS_RULES, None, encoded(&[zone_rule(3)])),
        (P::POSITIVE_ACCESS_RULES, Some(1), encoded(&[zone_rule(3)])),
        (
            P::NEGATIVE_ACCESS_RULES,
            Some(0),
            value(PropertyValue::Unsigned(2)),
        ),
        (P::LOG_ENABLE, None, value(PropertyValue::Boolean(false))),
    ] {
        let request = write_property(rights, property, index, octets);
        assert_eq!(
            wire(&fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await,
            WRITE_REFUSED,
            "{property:?} {index:?}"
        );
    }
    let request = write_property_multiple(
        rights,
        vec![attempt(
            P::LOG_ENABLE,
            None,
            value(PropertyValue::Boolean(false)),
        )],
    );
    let response = wire(
        &fixture,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        request,
    )
    .await;
    // A WritePropertyMultiple-Error opens with the class and code.
    assert_eq!(response[..8], [0x50, 5, 16, 0x0E, 0x91, 0, 0x91, 25]);
    assert_eq!(reads(&fixture, rights).await, expected_reads(&kept));
    assert_eq!(storage.load_saved(), Some(kept.clone()));
    drop(fixture);
    storage.fail.store(false, Ordering::SeqCst);
    let (rebuilt, rights) = served_by(&storage).await;
    assert_eq!(reads(&rebuilt, rights).await, expected_reads(&kept));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_property_multiple_refused_at_its_first_rights_write_stages_nothing() {
    let storage = Arc::new(RightsStorage::default());
    let (fixture, rights) = served_by(&storage).await;
    // The first attempt fails, since Enable takes a BOOLEAN. The object
    // stages a request's writes only up to one it will refuse (#1423), so
    // the rules write after it is never saved.
    let request = write_property_multiple(
        rights,
        vec![
            attempt(P::LOG_ENABLE, None, value(PropertyValue::Unsigned(1))),
            attempt(P::POSITIVE_ACCESS_RULES, None, encoded(&[zone_rule(9)])),
        ],
    );
    let response = wire(
        &fixture,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        request,
    )
    .await;
    assert_eq!(response[0], ERROR_PDU, "the first attempt was refused");
    saves_done(&fixture, rights).await;
    assert_eq!(storage.saves.load(Ordering::SeqCst), 0);
    assert_eq!(storage.load_saved(), None);
    assert_eq!(
        reads(&fixture, rights).await,
        expected_reads(&AccessRightsSnapshot::default())
    );
}

/// Wait, without the guard, until every save `rights` has queued has run.
/// The request released what it staged, so settling drops nothing: a write
/// still staged here would be one the request never corrected.
async fn saves_done(fixture: &Fixture, rights: ObjectIdentifier) {
    let mut db = fixture.db.write().await;
    let writes = db
        .get_mut(&rights)
        .and_then(|object| object.durable_writes_internal())
        .expect("the object saves");
    assert!(!writes.has_staged_write(), "a write is still staged");
    let wait = writes.settle_forgotten_writes().expect("the object saves");
    drop(db);
    tokio::time::timeout(WAIT, wait)
        .await
        .expect("the saves ran");
}

#[tokio::test(start_paused = true)]
async fn a_paused_clock_stands_still_while_a_rights_save_runs() {
    let storage = Arc::new(RightsStorage::default());
    let (fixture, rights) = served_by(&storage).await;
    let (started, go) = storage.hold();
    // The save takes real time on the object's writer thread, as on a slow
    // disk, while a timer like a request's APDU timeout is pending.
    let releasing = std::thread::spawn(move || {
        started.recv_timeout(WAIT).expect("the save started");
        std::thread::sleep(Duration::from_millis(50));
        go.send(()).unwrap();
    });
    let timer = tokio::spawn(tokio::time::sleep(Duration::from_secs(3)));
    let start = tokio::time::Instant::now();
    let request = write_property(
        rights,
        P::LOG_ENABLE,
        None,
        value(PropertyValue::Boolean(false)),
    );
    assert_eq!(
        wire(&fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await,
        SIMPLE_ACK_WRITE
    );
    releasing.join().unwrap();
    // Waiting for the save kept the runtime busy, so the paused clock did
    // not jump to the timer.
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "virtual time jumped {:?} while the save ran",
        start.elapsed()
    );
    assert!(!timer.is_finished());
    timer.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_stopped_mid_save_leaves_storage_with_the_served_rules() {
    let served = positive_only(&[zone_rule(1)]);
    let storage = holding(served.clone());
    let (server, inbound) = server(&storage).await;
    let (started, go) = storage.hold();
    let request = write_property(
        rights_oid(),
        P::POSITIVE_ACCESS_RULES,
        Some(1),
        encoded(&[zone_rule(9)]),
    );
    send(&inbound, ConfirmedServiceChoice::WRITE_PROPERTY, request).await;
    tokio::task::spawn_blocking(move || started.recv_timeout(WAIT))
        .await
        .unwrap()
        .expect("the save started");
    // stop() aborts the request, so its staged write is never made or
    // released; the staged save lands only after the request has gone.
    let server = stop_then_release(server, go).await;
    // Storage holds the rules the object serves once stop returns, though
    // the database outlives the stop.
    assert_eq!(storage.load_saved(), Some(served.clone()));
    let db = Arc::clone(server.database());
    assert_eq!(
        db.read()
            .await
            .get(&rights_oid())
            .unwrap()
            .read_property(P::POSITIVE_ACCESS_RULES, None)
            .unwrap(),
        served_rules(&[zone_rule(1)])
    );
    // A restart serves them too.
    drop(db);
    drop(server);
    assert_eq!(
        rights_object(&storage).positive_access_rules(),
        [zone_rule(1)]
    );
}

/// Neither rule array nor Enable is commandable or holds a NULL, so a NULL
/// written to one succeeds and leaves it as it was (#1396). The object
/// stages and saves nothing for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_null_succeeds_unchanged_and_saves_nothing() {
    let kept = AccessRightsSnapshot {
        positive_access_rules: Some(vec![zone_rule(1)]),
        enable: Some(false),
        ..AccessRightsSnapshot::default()
    };
    let storage = holding(kept.clone());
    let (fixture, rights) = served_by(&storage).await;
    let null = || value(PropertyValue::Null);
    for (property, index) in [
        (P::LOG_ENABLE, None),
        (P::POSITIVE_ACCESS_RULES, None),
        (P::POSITIVE_ACCESS_RULES, Some(1)),
        (P::NEGATIVE_ACCESS_RULES, Some(0)),
    ] {
        let request = write_property(rights, property, index, null());
        assert_eq!(
            wire(&fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await,
            SIMPLE_ACK_WRITE,
            "{property:?} {index:?}"
        );
    }
    let request = write_property_multiple(rights, vec![attempt(P::LOG_ENABLE, None, null())]);
    assert_eq!(
        wire(
            &fixture,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            request
        )
        .await,
        SIMPLE_ACK_WPM
    );
    assert_eq!(reads(&fixture, rights).await, expected_reads(&kept));
    assert_eq!(storage.saves.load(Ordering::SeqCst), 0);
    assert_eq!(storage.load_saved(), Some(kept));
}

/// Accompaniment saves through the server as the rule arrays do (#1393): a
/// WriteProperty stages its save with the guard dropped, a rebuilt server
/// serves the saved reference though nothing configures one, and a NULL
/// succeeds without changing or saving anything (#1396).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_accompaniment_write_saves_off_the_lock_and_survives_a_rebuild() {
    // Access Credential 5 in Device 99: device [0], then object [1].
    const REMOTE_CREDENTIAL: [u8; 10] =
        [0x0C, 0x02, 0x00, 0x00, 0x63, 0x1C, 0x08, 0x00, 0x00, 0x05];
    let oid = |object_type, instance| ObjectIdentifier::new(object_type, instance).unwrap();
    // An earlier start saved Access User 3, so the row is served.
    let storage = holding(AccessRightsSnapshot {
        accompaniment: Some(oid(ObjectType::ACCESS_USER, 3).into()),
        ..AccessRightsSnapshot::default()
    });
    let (fixture, rights) = served_by(&storage).await;
    let request = write_property(rights, P::ACCOMPANIMENT, None, REMOTE_CREDENTIAL.to_vec());
    let service = ConfirmedServiceChoice::WRITE_PROPERTY;
    let (response, available) = while_saving(&fixture, &storage, service, request, WAIT).await;
    assert_eq!(response, SIMPLE_ACK_WRITE);
    assert!(available, "the database was held while Accompaniment saved");
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
    let credential = bacnet_types::constructed::BACnetDeviceObjectReference {
        device_identifier: Some(oid(ObjectType::DEVICE, 99)),
        object_identifier: oid(ObjectType::ACCESS_CREDENTIAL, 5),
    };
    assert_eq!(
        storage.load_saved().and_then(|saved| saved.accompaniment),
        Some(credential)
    );
    let served = PropertyValue::ApplicationData(REMOTE_CREDENTIAL.to_vec());
    assert_eq!(fixture.read(rights, P::ACCOMPANIMENT).await, served);

    let null = || value(PropertyValue::Null);
    let request = write_property(rights, P::ACCOMPANIMENT, None, null());
    assert_eq!(wire(&fixture, service, request).await, SIMPLE_ACK_WRITE);
    let request = write_property_multiple(rights, vec![attempt(P::ACCOMPANIMENT, None, null())]);
    assert_eq!(
        wire(
            &fixture,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            request
        )
        .await,
        SIMPLE_ACK_WPM
    );
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.read(rights, P::ACCOMPANIMENT).await, served);
    drop(fixture);
    let (rebuilt, rights) = served_by(&storage).await;
    assert_eq!(rebuilt.read(rights, P::ACCOMPANIMENT).await, served);
}

#[path = "access_rights_fold_wire_tests.rs"]
mod fold_wire_tests;
