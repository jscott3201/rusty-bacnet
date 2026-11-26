//! A Device with Tags storage keeps each half of the clock's zone a peer
//! writes across a restart (#2395): the server puts it over the zone its
//! builder gives at start, and the host's own `set_clock_zone`, newer,
//! drops it; a save that fails refuses the write, or the host's change, and
//! leaves the clock as it was. With the opt-in off a half held is kept,
//! unapplied.
use super::*;
use crate::server::test_transport::TestTransport;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::object_profile::{TagsPersistence, TagsSnapshot, WrittenZone};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn zone(utc_offset_minutes: i16, daylight_savings: bool) -> ClockConfig {
    ClockConfig::new(utc_offset_minutes, daylight_savings).unwrap()
}

/// Tags storage in memory for a Device, its saves counted, and a switch
/// that fails them.
#[derive(Default)]
struct ZoneStore {
    saved: Mutex<Option<TagsSnapshot>>,
    saves: AtomicUsize,
    fail: AtomicBool,
}

impl ZoneStore {
    fn saved(&self) -> Option<TagsSnapshot> {
        self.saved.lock().unwrap().clone()
    }
}

impl TagsPersistence for ZoneStore {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error> {
        Ok(self.saved())
    }

    fn save(&self, _: ObjectIdentifier, snapshot: &TagsSnapshot) -> Result<(), Error> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::from(bacnet_types::error::PersistenceError::new(
                bacnet_types::error::PersistenceErrorKind::BackendFailed,
                "storage unavailable",
            )));
        }
        *self.saved.lock().unwrap() = Some(snapshot.clone());
        self.saves.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// A server built in `zone`, taking a peer's zone writes when `writable`,
/// over a database holding its Device: `device`, or one kept in `storage`.
async fn built_on(
    storage: &Arc<ZoneStore>,
    zone: ClockConfig,
    writable: bool,
    device: Option<DeviceObject>,
) -> (BACnetServer<TestTransport>, ObjectIdentifier) {
    let device = device.unwrap_or_else(|| {
        DeviceObject::with_tags_persistence(DeviceConfig::default(), storage.clone()).unwrap()
    });
    let oid = device.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(device)).unwrap();
    let server = BACnetServer::generic_builder()
        .device(bacnet_objects::DeviceIdentity::default())
        .database(db)
        .transport(TestTransport::new())
        .clock_zone(zone)
        .clock_zone_writable(writable)
        .build()
        .await
        .unwrap();
    (server, oid)
}

/// A write of `value` to `property` of the Device, as a request makes it
/// under the database guard.
async fn write(
    server: &BACnetServer<TestTransport>,
    device: ObjectIdentifier,
    property: P,
    value: PropertyValue,
) -> Result<(), Error> {
    server
        .database()
        .write()
        .await
        .get_mut(&device)
        .unwrap()
        .write_property(property, None, value, None)
}

fn kept(utc_offset: Option<i16>, daylight_savings_status: Option<bool>) -> WrittenZone {
    WrittenZone {
        utc_offset,
        daylight_savings_status,
    }
}

/// Storage already holding `zone`.
fn holding(zone: WrittenZone) -> Arc<ZoneStore> {
    let storage = Arc::new(ZoneStore::default());
    *storage.saved.lock().unwrap() = Some(TagsSnapshot {
        zone,
        ..TagsSnapshot::default()
    });
    storage
}

/// The newest change wins (#2395): a half a peer wrote is saved and wins
/// over the builder's zone at the next start; the host's own change, newer,
/// drops it from storage, so the start after runs the builder's zone; a
/// peer's write after that wins again.
#[tokio::test(start_paused = true)]
async fn the_newest_change_wins_across_restarts() {
    let storage = Arc::new(ZoneStore::default());
    let (mut server, device) = built_on(&storage, zone(300, false), true, None).await;
    write(
        &server,
        device,
        P::DAYLIGHT_SAVINGS_STATUS,
        PropertyValue::Boolean(true),
    )
    .await
    .unwrap();
    assert_eq!(server.clock_zone(), Some(zone(300, true)));
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
    assert_eq!(storage.saved().unwrap().zone, kept(None, Some(true)));
    server.stop().await.unwrap();
    drop(server);

    let (mut server, device) = built_on(&storage, zone(360, false), true, None).await;
    assert_eq!(server.clock_zone(), Some(zone(360, true)));
    server.set_clock_zone(zone(0, false)).await.unwrap();
    assert_eq!(server.clock_zone(), Some(zone(0, false)));
    assert!(storage.saved().unwrap().zone.is_empty());
    assert_eq!(storage.saves.load(Ordering::SeqCst), 2);
    server.stop().await.unwrap();
    drop(server);

    let (mut server, _) = built_on(&storage, zone(360, false), true, None).await;
    assert_eq!(server.clock_zone(), Some(zone(360, false)));
    write(&server, device, P::UTC_OFFSET, PropertyValue::Signed(-60))
        .await
        .unwrap();
    server.stop().await.unwrap();
    drop(server);

    let (mut server, _) = built_on(&storage, zone(360, false), true, None).await;
    assert_eq!(server.clock_zone(), Some(zone(-60, false)));
    server.stop().await.unwrap();
}

/// A host's change whose save fails is returned and changes nothing: the
/// clock keeps its zone and storage its half.
#[tokio::test(start_paused = true)]
async fn a_host_change_whose_save_fails_changes_nothing() {
    let storage = holding(kept(None, Some(true)));
    let (mut server, _) = built_on(&storage, zone(300, false), true, None).await;
    assert_eq!(server.clock_zone(), Some(zone(300, true)));
    storage.fail.store(true, Ordering::SeqCst);
    assert!(server.set_clock_zone(zone(0, false)).await.is_err());
    assert_eq!(server.clock_zone(), Some(zone(300, true)));
    assert_eq!(storage.saved().unwrap().zone, kept(None, Some(true)));
    storage.fail.store(false, Ordering::SeqCst);
    server.set_clock_zone(zone(0, false)).await.unwrap();
    assert!(storage.saved().unwrap().zone.is_empty());
    server.stop().await.unwrap();
}

/// With the opt-in off a half held is left unapplied, named in a warning,
/// and kept: the host's change and a Description write leave the file's
/// half alone. Turning the opt-in on applies it.
#[tokio::test(start_paused = true)]
async fn with_the_opt_in_off_a_held_half_is_kept_unapplied() {
    let storage = holding(kept(Some(-60), None));
    let (spy, _guard) = super::super::warn_spy::WarnSpy::install();
    let (mut server, device) = built_on(&storage, zone(300, false), false, None).await;
    assert_eq!(server.clock_zone(), Some(zone(300, false)));
    assert!(
        spy.warnings()
            .iter()
            .any(|warning| warning.message.contains("left unapplied")),
        "{:?}",
        spy.warnings()
    );
    server.set_clock_zone(zone(0, true)).await.unwrap();
    write(
        &server,
        device,
        P::DESCRIPTION,
        PropertyValue::CharacterString("lobby".into()),
    )
    .await
    .unwrap();
    let saved = storage.saved().unwrap();
    assert_eq!(saved.zone, kept(Some(-60), None));
    assert_eq!(saved.description.as_deref(), Some("lobby"));
    server.stop().await.unwrap();
    drop(server);

    let (mut server, _) = built_on(&storage, zone(300, false), true, None).await;
    assert_eq!(server.clock_zone(), Some(zone(-60, false)));
    server.stop().await.unwrap();
}

/// A save that fails refuses the write with DEVICE / OPERATIONAL_PROBLEM,
/// and the clock keeps the zone it had.
#[tokio::test(start_paused = true)]
async fn a_zone_write_whose_save_fails_is_refused_and_changes_nothing() {
    let storage = Arc::new(ZoneStore::default());
    let (mut server, device) = built_on(&storage, zone(300, false), true, None).await;
    storage.fail.store(true, Ordering::SeqCst);
    let refused = write(&server, device, P::UTC_OFFSET, PropertyValue::Signed(360)).await;
    assert!(
        matches!(&refused, Err(Error::Remote(RemoteError::Error { class, code }))
            if *class == ErrorClass::DEVICE && *code == ErrorCode::OPERATIONAL_PROBLEM),
        "{refused:?}"
    );
    assert_eq!(server.clock_zone(), Some(zone(300, false)));
    assert_eq!(storage.saved(), None);
    server.stop().await.unwrap();
}
