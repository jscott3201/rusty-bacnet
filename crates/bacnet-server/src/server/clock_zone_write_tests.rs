//! The opt-in that lets a peer write the Device's zone (#2395), on a
//! started server: the builders and `ServerConfig` turn it on, it is off
//! by default, and a written zone is the one everything that judges local
//! time reads, a Notification Class recipient window among them.
use super::*;
use crate::server::event_notifications_tests::local_broadcast_destination;
use crate::server::event_recipient_routing_tests::distribute_from_database_with_bindings;
use crate::server::test_transport::TestTransport;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use std::sync::Arc;
use tokio::sync::RwLock;

fn zone(utc_offset_minutes: i16, daylight_savings: bool) -> ClockConfig {
    ClockConfig::new(utc_offset_minutes, daylight_savings).unwrap()
}

/// A server built with the generic builder in `zone`, taking a peer's zone
/// writes when `writable`, over a database holding only its Device.
async fn built(writable: bool) -> (BACnetServer<TestTransport>, ObjectIdentifier) {
    let device = DeviceObject::new(DeviceConfig::default()).unwrap();
    let oid = device.object_identifier();
    let identity = bacnet_objects::DeviceIdentity::default();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(device)).unwrap();
    let server = BACnetServer::generic_builder()
        .device(identity)
        .database(db)
        .transport(TestTransport::new())
        .clock_zone(zone(300, false))
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

#[tokio::test(start_paused = true)]
async fn the_builders_opt_in_lets_a_peer_write_the_zone() {
    let (mut server, device) = built(false).await;
    assert!(!server.config.clock_zone_writable);
    let refused = write(&server, device, P::UTC_OFFSET, PropertyValue::Signed(360)).await;
    assert!(
        matches!(&refused, Err(Error::Remote(bacnet_types::error::RemoteError::Error { class, code }))
            if *class == ErrorClass::PROPERTY && *code == ErrorCode::WRITE_ACCESS_DENIED),
        "{refused:?}"
    );
    assert_eq!(server.clock_zone(), Some(zone(300, false)));
    server.stop().await.unwrap();

    let (mut server, device) = built(true).await;
    write(&server, device, P::UTC_OFFSET, PropertyValue::Signed(360))
        .await
        .unwrap();
    write(
        &server,
        device,
        P::DAYLIGHT_SAVINGS_STATUS,
        PropertyValue::Boolean(true),
    )
    .await
    .unwrap();
    assert_eq!(server.clock_zone(), Some(zone(360, true)));
    // The host's own change still applies.
    server.set_clock_zone(zone(300, false)).await.unwrap();
    assert_eq!(server.clock_zone(), Some(zone(300, false)));
    server.stop().await.unwrap();
}

/// The B/IP builder takes the opt-in into its configuration too.
#[test]
fn the_bip_builder_sets_the_opt_in() {
    let builder = BACnetServer::bip_builder();
    assert!(!builder.config.clock_zone_writable);
    assert!(builder.clock_zone_writable(true).config.clock_zone_writable);
    assert!(!ServerConfig::default().clock_zone_writable);
}

/// A Notification Class destination open from 08:00 to 08:59:59.99 takes a
/// notification at 08:30 local, not once a peer writes daylight saving
/// (09:30), and again once it writes an offset an hour further west.
#[tokio::test(start_paused = true)]
async fn a_recipient_window_follows_a_peer_written_zone() {
    let clock = Arc::new(ServerClock::new(zone(300, false)).taking_zone_writes(true));
    let thursday = Date {
        year: 124,
        month: 7,
        day: 4,
        day_of_week: 4,
    };
    let half_past = Time {
        hour: 13,
        minute: 30,
        second: 0,
        hundredths: 0,
    };
    clock.synchronize(thursday, half_past, true).unwrap();
    let reader = || Some(Arc::clone(&clock) as Arc<dyn ClockReader>);
    // The Device a peer writes, on the clock.
    let device = DeviceObject::new(DeviceConfig::default()).unwrap();
    let device_oid = device.object_identifier();
    let mut device_db = ObjectDatabase::new();
    device_db.add(Box::new(device)).unwrap();
    device_db.set_clock_reader(reader());
    let mut peer_writes = |property, value| {
        device_db
            .get_mut(&device_oid)
            .unwrap()
            .write_property(property, None, value, None)
            .unwrap();
    };
    let notified = || {
        let mut class = NotificationClass::new(0, "NC-0").unwrap();
        let mut morning = local_broadcast_destination();
        morning.from_time = Time {
            hour: 8,
            minute: 0,
            second: 0,
            hundredths: 0,
        };
        morning.to_time = Time {
            hour: 8,
            minute: 59,
            second: 59,
            hundredths: 99,
        };
        class.add_destination(morning).unwrap();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(class)).unwrap();
        db.set_clock_reader(reader());
        let bindings = Arc::new(RwLock::new(
            crate::server::device_bindings::DeviceBindingTable::new(),
        ));
        async move {
            let (broadcasts, unicasts) = distribute_from_database_with_bindings(db, bindings).await;
            broadcasts.len() + unicasts.len()
        }
    };
    assert_eq!(notified().await, 1);
    peer_writes(P::DAYLIGHT_SAVINGS_STATUS, PropertyValue::Boolean(true));
    assert_eq!(clock.read_clock().unwrap().local_time.hour, 9);
    assert_eq!(notified().await, 0);
    peer_writes(P::UTC_OFFSET, PropertyValue::Signed(360));
    assert_eq!(clock.read_clock().unwrap().local_time.hour, 8);
    assert_eq!(notified().await, 1);
}
