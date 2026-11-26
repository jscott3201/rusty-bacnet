//! A peer's zone write on a Device with Tags storage, on the wire (#2395):
//! the request stages its save and makes it with the database guard
//! dropped, a request writing both halves saves once, and a save that
//! fails is refused with DEVICE / OPERATIONAL_PROBLEM, the clock as it was.

use super::audit_log_buffer_wire_tests::write_error;
use super::clock_zone_wire_tests::synced_clock;
use super::mutation_list_wire_tests::wire;
use super::mutation_tests::{Fixture, value, wpm};
use super::*;
use crate::server::clock::ServerClock;
use bacnet_objects::clock::ClockReader;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::object_profile::{TagsPersistence, TagsSnapshot};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_services::write_property::WritePropertyRequest;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use PropertyIdentifier as P;

/// Tags storage in memory, its saves counted, and a switch that fails them.
#[derive(Default)]
struct Store {
    saved: StdMutex<Option<TagsSnapshot>>,
    saves: AtomicUsize,
    fail: AtomicBool,
}

impl TagsPersistence for Store {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error> {
        Ok(self.saved.lock().unwrap().clone())
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

/// A fixture holding the default Device kept in `store`, on `clock`.
async fn fixture(clock: &Arc<ServerClock>, store: &Arc<Store>) -> (Arc<Fixture>, ObjectIdentifier) {
    let fixture = Fixture::new(None);
    let device =
        DeviceObject::with_tags_persistence(DeviceConfig::default(), store.clone()).unwrap();
    let oid = device.object_identifier();
    let mut db = fixture.db.write().await;
    db.add(Box::new(device)).unwrap();
    db.set_clock_reader(Some(Arc::clone(clock) as Arc<dyn ClockReader>));
    drop(db);
    (Arc::new(fixture), oid)
}

fn attempt(property: P, written: PropertyValue) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: property,
        property_array_index: None,
        value: value(written),
        priority: None,
    }
}

fn utc_offset(minutes: i32, device: ObjectIdentifier) -> Bytes {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: device,
        property_identifier: P::UTC_OFFSET,
        property_array_index: None,
        property_value: value(PropertyValue::Signed(minutes)),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    request.freeze()
}

/// Both halves in one request: one save, holding both, and the clock in
/// the pair written.
#[tokio::test(start_paused = true)]
async fn a_request_writing_both_halves_saves_them_once() {
    let clock = synced_clock(true);
    let store = Arc::new(Store::default());
    let (fixture, device) = fixture(&clock, &store).await;
    let request = wpm(vec![WriteAccessSpecification {
        object_identifier: device,
        list_of_properties: vec![
            attempt(P::UTC_OFFSET, PropertyValue::Signed(-60)),
            attempt(P::DAYLIGHT_SAVINGS_STATUS, PropertyValue::Boolean(true)),
        ],
    }]);
    let service = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
    assert_eq!(wire(&fixture, service, request).await, [0x20, 5, 16]);
    assert_eq!(store.saves.load(Ordering::SeqCst), 1);
    let saved = store.saved.lock().unwrap().clone().unwrap();
    assert_eq!(saved.zone.over((0, false)), (-60, true));
    let frame = clock.read_clock().unwrap();
    assert_eq!(
        (frame.utc_offset, frame.daylight_savings_status),
        (-60, true)
    );
}

/// A save that fails is refused on the wire, and the clock stays.
#[tokio::test(start_paused = true)]
async fn a_zone_write_that_cannot_be_saved_is_refused_on_the_wire() {
    let clock = synced_clock(true);
    let store = Arc::new(Store::default());
    let (fixture, device) = fixture(&clock, &store).await;
    store.fail.store(true, Ordering::SeqCst);
    let service = ConfirmedServiceChoice::WRITE_PROPERTY;
    assert_eq!(
        wire(&fixture, service, utc_offset(360, device)).await,
        write_error(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM)
    );
    let frame = clock.read_clock().unwrap();
    assert_eq!(
        (frame.utc_offset, frame.daylight_savings_status),
        (300, false)
    );
    assert_eq!(
        fixture.read(device, P::UTC_OFFSET).await,
        PropertyValue::Signed(300)
    );
}

/// Both halves and then an attempt the Device refuses: the request stops
/// there, the two halves stay made, on the clock and in storage, saved
/// once, and the request answers with an Error.
#[tokio::test(start_paused = true)]
async fn both_halves_then_a_refused_attempt_keep_the_halves() {
    let clock = synced_clock(true);
    let store = Arc::new(Store::default());
    let (fixture, device) = fixture(&clock, &store).await;
    let request = wpm(vec![WriteAccessSpecification {
        object_identifier: device,
        list_of_properties: vec![
            attempt(P::UTC_OFFSET, PropertyValue::Signed(-60)),
            attempt(P::DAYLIGHT_SAVINGS_STATUS, PropertyValue::Boolean(true)),
            attempt(P::DESCRIPTION, PropertyValue::Unsigned(1)),
        ],
    }]);
    let service = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
    let answer = wire(&fixture, service, request).await;
    // An Error PDU for WritePropertyMultiple, invoke ID 5.
    assert_eq!(answer[..3], [0x50, 5, 16]);
    let frame = clock.read_clock().unwrap();
    assert_eq!(
        (frame.utc_offset, frame.daylight_savings_status),
        (-60, true)
    );
    assert_eq!(store.saves.load(Ordering::SeqCst), 1);
    let saved = store.saved.lock().unwrap().clone().unwrap();
    assert_eq!(saved.zone.over((0, false)), (-60, true));
    assert_eq!(saved.description, None);
}
