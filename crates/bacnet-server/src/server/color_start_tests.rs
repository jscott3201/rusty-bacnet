//! Color and Color Temperature outputs at the server's start (#1526), in
//! paused time: In_Progress reads NOT_CONTROLLED until the start drives the
//! output, a non-zero default is driven at once, and a zero default keeps
//! the Present_Value storage brought back. `stop()` waits for the saves, so
//! a second server on the same storage starts from the last value.

use super::*;
use crate::server::cov_notifications_tests::recording_transport;
use crate::server::test_transport::TestTransport;
use bacnet_objects::color::{
    ColorObject, ColorOutputPersistence, ColorOutputSnapshot, ColorOutputValue,
    ColorTemperatureObject,
};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::BACnetXyColor;
use bacnet_types::enums::ColorOperationInProgress as InProgress;
use std::sync::Mutex as StdMutex;

const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;
const IN_PROGRESS: PropertyIdentifier = PropertyIdentifier::IN_PROGRESS;

/// Output storage in memory, shared across server runs, refusing saves
/// while `refuse` is set.
#[derive(Default)]
struct Memory {
    held: StdMutex<Option<ColorOutputSnapshot>>,
    refuse: std::sync::atomic::AtomicBool,
}

impl Memory {
    fn held(&self) -> Option<ColorOutputValue> {
        self.held
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|saved| saved.present_value)
    }
}

impl ColorOutputPersistence for Memory {
    fn load(&self, _object: ObjectIdentifier) -> Result<Option<ColorOutputSnapshot>, Error> {
        Ok(self.held.lock().unwrap().clone())
    }

    fn save(&self, _object: ObjectIdentifier, snapshot: &ColorOutputSnapshot) -> Result<(), Error> {
        if self.refuse.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(Error::Encoding("refused".into()));
        }
        *self.held.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

fn database(objects: Vec<Box<dyn BACnetObject>>) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(1526, 0)
                .expect("valid Device identity")
                .with_name("Color-start-device"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    for object in objects {
        db.add(object).unwrap();
    }
    db
}

async fn start(db: ObjectDatabase) -> BACnetServer<TestTransport> {
    let (transport, _sent) = recording_transport();
    let config = ServerConfig {
        device: bacnet_objects::DeviceIdentity::new(1526, 0).unwrap(),
        enable_event_enrollment: false,
        ..ServerConfig::default()
    };
    BACnetServer::start(config, db, transport).await.unwrap()
}

fn read_in(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    db.get(&oid).unwrap().read_property(property, None).unwrap()
}

async fn read(
    server: &BACnetServer<TestTransport>,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    read_in(&*server.database().read().await, oid, property)
}

fn state(state: InProgress) -> PropertyValue {
    PropertyValue::Enumerated(state.to_raw())
}

fn xy(color: BACnetXyColor) -> PropertyValue {
    PropertyValue::List(vec![
        PropertyValue::Real(color.x),
        PropertyValue::Real(color.y),
    ])
}

const RED: BACnetXyColor = BACnetXyColor::new(0.64, 0.33);
const GREEN: BACnetXyColor = BACnetXyColor::new(0.3, 0.6);
const ZERO: BACnetXyColor = BACnetXyColor::new(0.0, 0.0);

#[tokio::test(start_paused = true)]
async fn a_non_zero_default_is_driven_at_start() {
    let mut color = ColorObject::new(1, "color").unwrap();
    color.set_default_color(RED).unwrap();
    let mut ct = ColorTemperatureObject::new(1, "ct").unwrap();
    ct.set_default_color_temperature(2700).unwrap();
    let (color_oid, ct_oid) = (color.object_identifier(), ct.object_identifier());
    let db = database(vec![Box::new(color), Box::new(ct)]);
    // Before the start nothing has driven either output.
    for oid in [color_oid, ct_oid] {
        assert_eq!(
            read_in(&db, oid, IN_PROGRESS),
            state(InProgress::NOT_CONTROLLED)
        );
    }
    assert_eq!(
        read_in(&db, color_oid, PV),
        xy(BACnetXyColor::new(0.3127, 0.3290))
    );

    let mut server = start(db).await;
    assert_eq!(
        read(&server, color_oid, IN_PROGRESS).await,
        state(InProgress::IDLE)
    );
    assert_eq!(read(&server, color_oid, PV).await, xy(RED));
    assert_eq!(
        read(&server, ct_oid, IN_PROGRESS).await,
        state(InProgress::IDLE)
    );
    assert_eq!(
        read(&server, ct_oid, PV).await,
        PropertyValue::Unsigned(2700)
    );
    // Instant: nothing for the monotonic task to finish.
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(
        read(&server, color_oid, IN_PROGRESS).await,
        state(InProgress::IDLE)
    );
    server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_zero_default_without_a_stored_value_stays_not_controlled_until_written() {
    let mut color = ColorObject::new(1, "color").unwrap();
    color.set_default_color(ZERO).unwrap();
    let oid = color.object_identifier();
    let mut server = start(database(vec![Box::new(color)])).await;
    assert_eq!(
        read(&server, oid, IN_PROGRESS).await,
        state(InProgress::NOT_CONTROLLED)
    );
    server
        .write_local(
            &oid,
            PV,
            None,
            xy(GREEN),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    assert_eq!(
        read(&server, oid, IN_PROGRESS).await,
        state(InProgress::IDLE)
    );
    server.stop().await.unwrap();
}

/// A first run writes Present_Value and stops; a second on the same storage
/// with a zero default starts from that value, and one with a non-zero
/// default starts from the default.
#[tokio::test(start_paused = true)]
async fn a_zero_default_keeps_the_stored_value_across_a_restart() {
    let storage = Arc::new(Memory::default());
    let build = |default: BACnetXyColor| {
        let mut color = ColorObject::new(1, "color")
            .unwrap()
            .with_output_persistence(storage.clone())
            .unwrap();
        color.set_default_color(default).unwrap();
        color
    };

    let first = build(ZERO);
    let oid = first.object_identifier();
    let mut server = start(database(vec![Box::new(first)])).await;
    assert_eq!(
        read(&server, oid, IN_PROGRESS).await,
        state(InProgress::NOT_CONTROLLED)
    );
    server
        .write_local(
            &oid,
            PV,
            None,
            xy(GREEN),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    server.stop().await.unwrap();
    assert_eq!(storage.held(), Some(ColorOutputValue::Xy(GREEN)));
    drop(server);

    let second = build(ZERO);
    assert!(second.output_restored());
    let db = database(vec![Box::new(second)]);
    assert_eq!(
        read_in(&db, oid, IN_PROGRESS),
        state(InProgress::NOT_CONTROLLED)
    );
    let mut server = start(db).await;
    assert_eq!(
        read(&server, oid, IN_PROGRESS).await,
        state(InProgress::IDLE)
    );
    assert_eq!(read(&server, oid, PV).await, xy(GREEN));
    server.stop().await.unwrap();
    drop(server);

    let mut server = start(database(vec![Box::new(build(RED))])).await;
    assert_eq!(read(&server, oid, PV).await, xy(RED));
    server.stop().await.unwrap();
    assert_eq!(storage.held(), Some(ColorOutputValue::Xy(RED)));
}

/// A default written through the server is kept with the output (#1956): a
/// zero one written in a first run keeps the last value at the next start,
/// over the non-zero default the application configures, for both kinds.
#[tokio::test(start_paused = true)]
async fn a_written_zero_default_keeps_the_last_value_at_the_next_start() {
    let color_storage = Arc::new(Memory::default());
    let ct_storage = Arc::new(Memory::default());
    let build = || -> Vec<Box<dyn BACnetObject>> {
        let mut color = ColorObject::new(1, "color")
            .unwrap()
            .with_output_persistence(color_storage.clone())
            .unwrap();
        color.set_default_color(RED).unwrap();
        let mut ct = ColorTemperatureObject::new(1, "ct")
            .unwrap()
            .with_output_persistence(ct_storage.clone())
            .unwrap();
        ct.set_default_color_temperature(4000).unwrap();
        vec![Box::new(color), Box::new(ct)]
    };
    let color = ObjectIdentifier::new(ObjectType::COLOR, 1).unwrap();
    let ct = ObjectIdentifier::new(ObjectType::COLOR_TEMPERATURE, 1).unwrap();
    let mut server = start(database(build())).await;
    let source = || crate::LocalCommandSource::ServerDevice;
    for (oid, property, value) in [
        (color, PV, xy(GREEN)),
        (color, PropertyIdentifier::DEFAULT_COLOR, xy(ZERO)),
        (ct, PV, PropertyValue::Unsigned(2700)),
        (
            ct,
            PropertyIdentifier::DEFAULT_COLOR_TEMPERATURE,
            PropertyValue::Unsigned(0),
        ),
    ] {
        server
            .write_local(&oid, property, None, value, None, source())
            .await
            .unwrap();
    }
    server.stop().await.unwrap();
    drop(server);
    assert_eq!(color_storage.held(), Some(ColorOutputValue::Xy(GREEN)));

    let mut server = start(database(build())).await;
    assert_eq!(read(&server, color, PV).await, xy(GREEN));
    assert_eq!(
        read(&server, color, PropertyIdentifier::DEFAULT_COLOR).await,
        xy(ZERO)
    );
    assert_eq!(read(&server, ct, PV).await, PropertyValue::Unsigned(2700));
    assert_eq!(
        read(&server, ct, PropertyIdentifier::DEFAULT_COLOR_TEMPERATURE).await,
        PropertyValue::Unsigned(0)
    );
    for oid in [color, ct] {
        assert_eq!(
            read(&server, oid, IN_PROGRESS).await,
            state(InProgress::IDLE)
        );
    }
    server.stop().await.unwrap();
}

/// A default whose save fails is refused through the server, and the old
/// default stays.
#[tokio::test(start_paused = true)]
async fn a_default_that_cannot_be_saved_is_refused() {
    let storage = Arc::new(Memory::default());
    let mut color = ColorObject::new(1, "color")
        .unwrap()
        .with_output_persistence(storage.clone())
        .unwrap();
    color.set_default_color(RED).unwrap();
    let oid = color.object_identifier();
    let mut server = start(database(vec![Box::new(color)])).await;
    storage
        .refuse
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let refused = server
        .write_local(
            &oid,
            PropertyIdentifier::DEFAULT_COLOR,
            None,
            xy(ZERO),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await;
    assert!(refused.is_err());
    assert_eq!(
        read(&server, oid, PropertyIdentifier::DEFAULT_COLOR).await,
        xy(RED)
    );
    storage
        .refuse
        .store(false, std::sync::atomic::Ordering::SeqCst);
    server.stop().await.unwrap();
}
