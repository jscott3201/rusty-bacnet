//! A peer's write of the Device's UTC_Offset and Daylight_Savings_Status on
//! the wire (#2395). Without the opt-in both are refused as read-only, as
//! before; with it a write moves the clock as the host's zone change does,
//! keeping its synchronization, and a value of the wrong datatype or off
//! the 15-minute grid is refused and changes nothing.

use super::audit_log_buffer_wire_tests::write_error;
use super::mutation_list_wire_tests::wire;
use super::mutation_tests::{Fixture, value, wpm};
use super::*;
use crate::server::clock::{ClockConfig, ServerClock};
use bacnet_objects::clock::ClockReader;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::primitives::{Date, Time};

use PropertyIdentifier as P;

const SIMPLE_ACK_WRITE: [u8; 3] = [0x20, 5, 15];
const SIMPLE_ACK_WPM: [u8; 3] = [0x20, 5, 16];

/// A clock five hours west, synchronized to 13:15 UTC on Thursday
/// 2024-07-04 (08:15 local), taking a peer's zone writes when `writable`.
pub(super) fn synced_clock(writable: bool) -> Arc<ServerClock> {
    let clock =
        ServerClock::new(ClockConfig::new(300, false).unwrap()).taking_zone_writes(writable);
    let date = Date {
        year: 124,
        month: 7,
        day: 4,
        day_of_week: 4,
    };
    let time = Time {
        hour: 13,
        minute: 15,
        second: 0,
        hundredths: 0,
    };
    clock.synchronize(date, time, true).unwrap();
    Arc::new(clock)
}

/// A fixture holding the default Device, on `clock`.
async fn fixture(clock: &Arc<ServerClock>) -> (Arc<Fixture>, ObjectIdentifier) {
    let fixture = Fixture::new(None);
    let device = DeviceObject::new(DeviceConfig::default()).unwrap();
    let oid = device.object_identifier();
    let mut db = fixture.db.write().await;
    db.add(Box::new(device)).unwrap();
    db.set_clock_reader(Some(Arc::clone(clock) as Arc<dyn ClockReader>));
    drop(db);
    (Arc::new(fixture), oid)
}

fn write_request(device: ObjectIdentifier, property: P, written: PropertyValue) -> Bytes {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: device,
        property_identifier: property,
        property_array_index: None,
        property_value: value(written),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    request.freeze()
}

async fn write(
    fixture: &Fixture,
    device: ObjectIdentifier,
    property: P,
    written: PropertyValue,
) -> Vec<u8> {
    let request = write_request(device, property, written);
    wire(fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await
}

/// The clock's local hour and minute, UTC_Offset and daylight saving.
fn reading(clock: &ServerClock) -> (u8, u8, i16, bool) {
    let frame = clock.read_clock().unwrap();
    (
        frame.local_time.hour,
        frame.local_time.minute,
        frame.utc_offset,
        frame.daylight_savings_status,
    )
}

/// Without the opt-in both properties stay read-only: refused as before,
/// and listed so.
#[tokio::test(start_paused = true)]
async fn without_the_opt_in_a_zone_write_is_refused_as_before() {
    let clock = synced_clock(false);
    let (fixture, device) = fixture(&clock).await;
    let denied = write_error(ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED);
    for (property, written) in [
        (P::UTC_OFFSET, PropertyValue::Signed(360)),
        (P::DAYLIGHT_SAVINGS_STATUS, PropertyValue::Boolean(true)),
    ] {
        assert_eq!(write(&fixture, device, property, written).await, denied);
    }
    assert_eq!(reading(&clock), (8, 15, 300, false));
    let db = fixture.db.read().await;
    let rows = db.get(&device).unwrap().property_metadata();
    for property in [P::UTC_OFFSET, P::DAYLIGHT_SAVINGS_STATUS] {
        let row = rows.iter().find(|row| row.property_identifier == property);
        assert_eq!(
            row.unwrap().write_capability,
            bacnet_objects::property_metadata::PropertyWriteCapability::ReadOnly
        );
    }
}

/// With it, daylight saving written after a synchronization moves local
/// time an hour and the clock stays synchronized; an offset moves it by the
/// difference, and both rows are listed writable.
#[tokio::test(start_paused = true)]
async fn a_zone_write_moves_local_time_and_keeps_the_synchronization() {
    let clock = synced_clock(true);
    let (fixture, device) = fixture(&clock).await;
    let anchor = clock.synchronized_at();
    assert_eq!(
        write(
            &fixture,
            device,
            P::DAYLIGHT_SAVINGS_STATUS,
            PropertyValue::Boolean(true)
        )
        .await,
        SIMPLE_ACK_WRITE
    );
    assert_eq!(reading(&clock), (9, 15, 300, true));
    assert_eq!(clock.synchronized_at(), anchor);
    assert!(anchor.is_some());
    // Six hours west instead of five: an hour back.
    assert_eq!(
        write(&fixture, device, P::UTC_OFFSET, PropertyValue::Signed(360)).await,
        SIMPLE_ACK_WRITE
    );
    assert_eq!(reading(&clock), (8, 15, 360, true));
    // 5:45 east, as a peer may name any multiple of 15 minutes.
    assert_eq!(
        write(&fixture, device, P::UTC_OFFSET, PropertyValue::Signed(-345)).await,
        SIMPLE_ACK_WRITE
    );
    assert_eq!(reading(&clock), (20, 0, -345, true));
    assert_eq!(clock.synchronized_at(), anchor);
    assert_eq!(
        fixture.read(device, P::UTC_OFFSET).await,
        PropertyValue::Signed(-345)
    );
    let db = fixture.db.read().await;
    let rows = db.get(&device).unwrap().property_metadata();
    for property in [P::UTC_OFFSET, P::DAYLIGHT_SAVINGS_STATUS] {
        let row = rows.iter().find(|row| row.property_identifier == property);
        assert_eq!(
            row.unwrap().write_capability,
            bacnet_objects::property_metadata::PropertyWriteCapability::Always
        );
    }
}

/// A value off the 15-minute grid or past a day either way, or of another
/// datatype, is refused and changes nothing; a NULL succeeds and changes
/// nothing.
#[tokio::test(start_paused = true)]
async fn a_bad_zone_write_is_refused_and_changes_nothing() {
    let clock = synced_clock(true);
    let (fixture, device) = fixture(&clock).await;
    let range = write_error(ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE);
    let datatype = write_error(ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE);
    for (property, written, answer) in [
        (P::UTC_OFFSET, PropertyValue::Signed(7), &range),
        (P::UTC_OFFSET, PropertyValue::Signed(1455), &range),
        (P::UTC_OFFSET, PropertyValue::Signed(-1455), &range),
        (P::UTC_OFFSET, PropertyValue::Unsigned(60), &datatype),
        (P::UTC_OFFSET, PropertyValue::Boolean(true), &datatype),
        (
            P::DAYLIGHT_SAVINGS_STATUS,
            PropertyValue::Unsigned(1),
            &datatype,
        ),
        (
            P::DAYLIGHT_SAVINGS_STATUS,
            PropertyValue::Signed(0),
            &datatype,
        ),
    ] {
        assert_eq!(&write(&fixture, device, property, written).await, answer);
    }
    for property in [P::UTC_OFFSET, P::DAYLIGHT_SAVINGS_STATUS] {
        assert_eq!(
            write(&fixture, device, property, PropertyValue::Null).await,
            SIMPLE_ACK_WRITE
        );
    }
    assert_eq!(reading(&clock), (8, 15, 300, false));
    // Both ends of the range are taken.
    for minutes in [-1440, 1440] {
        assert_eq!(
            write(
                &fixture,
                device,
                P::UTC_OFFSET,
                PropertyValue::Signed(minutes)
            )
            .await,
            SIMPLE_ACK_WRITE
        );
    }
}

/// A WritePropertyMultiple of both properties leaves both, and a reader
/// after it sees the pair it wrote.
#[tokio::test(start_paused = true)]
async fn a_write_property_multiple_writes_both() {
    let clock = synced_clock(true);
    let (fixture, device) = fixture(&clock).await;
    let attempt = |property, written| BACnetPropertyValue {
        property_identifier: property,
        property_array_index: None,
        value: value(written),
        priority: None,
    };
    let request = wpm(vec![WriteAccessSpecification {
        object_identifier: device,
        list_of_properties: vec![
            attempt(P::UTC_OFFSET, PropertyValue::Signed(-60)),
            attempt(P::DAYLIGHT_SAVINGS_STATUS, PropertyValue::Boolean(true)),
        ],
    }]);
    let service = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
    assert_eq!(wire(&fixture, service, request).await, SIMPLE_ACK_WPM);
    assert_eq!(reading(&clock), (15, 15, -60, true));
}
