//! Schedule and Calendar evaluation in the server's schedule pass (#1028,
//! #1029), observed over the ReadProperty and WriteProperty handlers with an
//! injected clock.

use std::sync::{Arc, Mutex};

use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::schedule::{CalendarObject, ScheduleObject};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::{
    BACnetDateRange, BACnetObjectPropertyReference, BACnetSpecialEvent, BACnetTimeValue,
    SpecialEventPeriod,
};
use bytes::BytesMut;

use super::*;
use crate::handlers::{handle_read_property, handle_write_property};
use bacnet_types::enums::EngineeringUnits;

/// A Device clock a test moves by hand.
pub(crate) struct SettableClock(Mutex<ClockFrame>);

impl SettableClock {
    /// A clock reading `year`-`month`-`day` `hour`:`minute`:00.00 local time.
    pub(crate) fn at(year: u16, month: u8, day: u8, hour: u8, minute: u8) -> Arc<Self> {
        Arc::new(Self(Mutex::new(Self::frame(
            year, month, day, hour, minute,
        ))))
    }

    /// Move the clock.
    pub(crate) fn set(&self, year: u16, month: u8, day: u8, hour: u8, minute: u8) {
        *self.0.lock().unwrap() = Self::frame(year, month, day, hour, minute);
    }

    fn frame(year: u16, month: u8, day: u8, hour: u8, minute: u8) -> ClockFrame {
        ClockFrame {
            local_date: SpecificDate::new(year, month, day).unwrap().to_date(),
            local_time: Time {
                hour,
                minute,
                second: 0,
                hundredths: 0,
            },
            utc_offset: 0,
            daylight_savings_status: false,
        }
    }
}

impl ClockReader for SettableClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(*self.0.lock().unwrap())
    }
}

const SCHEDULE: u32 = 1;
const CALENDAR: u32 = 3;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn target() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_OUTPUT, 2)
}

fn at(hour: u8, minute: u8) -> Time {
    Time {
        hour,
        minute,
        second: 0,
        hundredths: 0,
    }
}

/// A database holding a Device (the command source), Analog Output 2
/// (relinquish default 0.0) and `schedule`, which commands its Present_Value.
fn database(clock: &Arc<SettableClock>, mut schedule: ScheduleObject) -> ObjectDatabase {
    schedule
        .add_object_property_reference(BACnetObjectPropertyReference::new(
            target(),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ))
        .unwrap();
    let mut db = ObjectDatabase::new();
    db.set_clock_reader(Some(clock.clone()));
    db.add(Box::new(
        DeviceObject::new(DeviceConfig::default()).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogOutputObject::new(2, "AO-2", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(schedule)).unwrap();
    db
}

/// A property as a ReadProperty ACK carries it.
fn read_wire(
    db: &ObjectDatabase,
    object_identifier: ObjectIdentifier,
    property_identifier: PropertyIdentifier,
    property_array_index: Option<u32>,
) -> Vec<u8> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier,
        property_identifier,
        property_array_index,
    }
    .encode(&mut request);
    let mut response = BytesMut::new();
    handle_read_property(db, &request, &mut response).unwrap();
    ReadPropertyACK::decode(&response).unwrap().property_value
}

fn write_wire(
    db: &mut ObjectDatabase,
    object_identifier: ObjectIdentifier,
    property_identifier: PropertyIdentifier,
    property_value: &[u8],
) {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier,
        property_identifier,
        property_array_index: None,
        property_value: property_value.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).unwrap();
}

/// Analog Output 2's Present_Value and Priority_Array slot `slot` on the wire.
fn target_wire(db: &ObjectDatabase, slot: u32) -> (Vec<u8>, Vec<u8>) {
    (
        read_wire(db, target(), PropertyIdentifier::PRESENT_VALUE, None),
        read_wire(db, target(), PropertyIdentifier::PRIORITY_ARRAY, Some(slot)),
    )
}

fn schedule_present_value(db: &ObjectDatabase) -> Vec<u8> {
    read_wire(
        db,
        oid(ObjectType::SCHEDULE, SCHEDULE),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    )
}

/// Application Real (tag 4, length 4: 0x44) and NULL (0x00).
const REAL_21_5: &[u8] = &[0x44, 0x41, 0xAC, 0x00, 0x00];
const REAL_5: &[u8] = &[0x44, 0x40, 0xA0, 0x00, 0x00];
const REAL_10: &[u8] = &[0x44, 0x41, 0x20, 0x00, 0x00];
const REAL_0: &[u8] = &[0x44, 0x00, 0x00, 0x00, 0x00];
const NULL: &[u8] = &[0x00];

#[tokio::test]
async fn schedule_commands_its_typed_value_at_priority_for_writing_and_relinquishes_with_null() {
    // #1028: Present_Value read as an Octet String of the time-value's
    // encoding, and the targets were written with it at priority 16.
    let clock = SettableClock::at(2026, 9, 14, 7, 0); // a Monday
    let mut schedule = ScheduleObject::new(SCHEDULE, "SCH-1", PropertyValue::Null).unwrap();
    schedule
        .set_weekly_schedule(
            0,
            vec![
                BACnetTimeValue {
                    time: at(8, 0),
                    value: PropertyValue::Real(21.5),
                },
                BACnetTimeValue {
                    time: at(17, 0),
                    value: PropertyValue::Null,
                },
            ],
        )
        .unwrap();
    schedule.set_priority_for_writing(9).unwrap();
    let db = Arc::new(RwLock::new(database(&clock, schedule)));

    // 07:00: Schedule_Default (NULL) applies; writing it relinquishes slot 9.
    tick_schedules(&db).await;
    let guard = db.read().await;
    assert_eq!(schedule_present_value(&guard), NULL);
    assert_eq!(target_wire(&guard, 9), (REAL_0.to_vec(), NULL.to_vec()));
    drop(guard);

    // 08:00: the weekly Real is commanded at slot 9 as a Real.
    clock.set(2026, 9, 14, 8, 0);
    tick_schedules(&db).await;
    let guard = db.read().await;
    assert_eq!(schedule_present_value(&guard), REAL_21_5);
    assert_eq!(
        target_wire(&guard, 9),
        (REAL_21_5.to_vec(), REAL_21_5.to_vec())
    );
    // Slot 16, where every write used to land, stays empty.
    assert_eq!(target_wire(&guard, 16).1, NULL);
    drop(guard);

    // 17:00: the weekly NULL hands back to Schedule_Default, NULL again, and
    // writing it relinquishes slot 9.
    clock.set(2026, 9, 14, 17, 0);
    tick_schedules(&db).await;
    let guard = db.read().await;
    assert_eq!(schedule_present_value(&guard), NULL);
    assert_eq!(target_wire(&guard, 9), (REAL_0.to_vec(), NULL.to_vec()));
}

#[tokio::test]
async fn schedule_follows_a_calendar_whose_date_list_is_written_over_the_wire() {
    let clock = SettableClock::at(2026, 9, 14, 9, 0); // a Monday
    let calendar = oid(ObjectType::CALENDAR, CALENDAR);
    let mut schedule = ScheduleObject::new(SCHEDULE, "SCH-1", PropertyValue::Real(10.0)).unwrap();
    schedule
        .set_weekly_schedule(
            0,
            vec![BACnetTimeValue {
                time: at(8, 0),
                value: PropertyValue::Real(21.5),
            }],
        )
        .unwrap();
    schedule
        .add_exception(BACnetSpecialEvent {
            period: SpecialEventPeriod::CalendarReference(calendar),
            list_of_time_values: vec![BACnetTimeValue {
                time: at(0, 0),
                value: PropertyValue::Real(5.0),
            }],
            event_priority: 1,
        })
        .unwrap();
    let mut db = database(&clock, schedule);
    db.add(Box::new(CalendarObject::new(CALENDAR, "Holidays").unwrap()))
        .unwrap();
    let db = Arc::new(RwLock::new(db));
    let calendar_pv =
        |db: &ObjectDatabase| read_wire(db, calendar, PropertyIdentifier::PRESENT_VALUE, None);

    // The Calendar is FALSE: the weekly value applies.
    tick_schedules(&db).await;
    assert_eq!(calendar_pv(&*db.read().await), [0x10]);
    assert_eq!(target_wire(&*db.read().await, 16).0, REAL_21_5);

    // Writing today into Date_List (date [0], 0x0C) makes the Calendar TRUE
    // at once, and the next pass switches to the exception.
    write_wire(
        &mut *db.write().await,
        calendar,
        PropertyIdentifier::DATE_LIST,
        &[0x0C, 126, 9, 14, 0xFF],
    );
    assert_eq!(calendar_pv(&*db.read().await), [0x11]);
    tick_schedules(&db).await;
    assert_eq!(schedule_present_value(&*db.read().await), REAL_5);
    assert_eq!(target_wire(&*db.read().await, 16).0, REAL_5);

    // The next day the Calendar is FALSE again, and Tuesday has no weekly
    // entries: Schedule_Default.
    clock.set(2026, 9, 15, 9, 0);
    assert_eq!(calendar_pv(&*db.read().await), [0x10]);
    tick_schedules(&db).await;
    assert_eq!(target_wire(&*db.read().await, 16).0, REAL_10);
}

#[tokio::test]
async fn schedule_writes_only_within_its_effective_period() {
    let clock = SettableClock::at(2026, 9, 13, 12, 0);
    let mut schedule = ScheduleObject::new(SCHEDULE, "SCH-1", PropertyValue::Real(10.0)).unwrap();
    schedule
        .set_effective_period(BACnetDateRange {
            start_date: SpecificDate::new(2026, 9, 14).unwrap().to_date(),
            end_date: SpecificDate::new(2026, 9, 15).unwrap().to_date(),
        })
        .unwrap();
    // Mondays and Wednesdays at 08:00.
    for day in [0, 2] {
        schedule
            .set_weekly_schedule(
                day,
                vec![BACnetTimeValue {
                    time: at(8, 0),
                    value: PropertyValue::Real(21.5),
                }],
            )
            .unwrap();
    }
    let db = Arc::new(RwLock::new(database(&clock, schedule)));

    // The day before the period: nothing is written.
    tick_schedules(&db).await;
    assert_eq!(target_wire(&*db.read().await, 16).1, NULL);
    // Its first day: the weekly value.
    clock.set(2026, 9, 14, 12, 0);
    tick_schedules(&db).await;
    assert_eq!(target_wire(&*db.read().await, 16).1, REAL_21_5);
    // Its last day, Tuesday: Schedule_Default.
    clock.set(2026, 9, 15, 12, 0);
    tick_schedules(&db).await;
    assert_eq!(target_wire(&*db.read().await, 16).1, REAL_10);
    // The day after, Wednesday would schedule 21.5, but the period is over:
    // nothing is written and Present_Value keeps its last value.
    clock.set(2026, 9, 16, 12, 0);
    tick_schedules(&db).await;
    assert_eq!(target_wire(&*db.read().await, 16).1, REAL_10);
    assert_eq!(schedule_present_value(&*db.read().await), REAL_10);
}

#[tokio::test]
async fn a_present_value_written_out_of_service_reaches_the_target_without_a_clock() {
    // #1055: a value written through the handler alone, with no server pass
    // after it, goes out at the next tick; it needs no valid Device clock.
    let clock = SettableClock::at(2026, 9, 14, 9, 0);
    let mut schedule = ScheduleObject::new(SCHEDULE, "SCH-1", PropertyValue::Real(10.0)).unwrap();
    schedule.set_priority_for_writing(9).unwrap();
    let mut db = database(&clock, schedule);
    db.set_clock_reader(None);
    let sch = oid(ObjectType::SCHEDULE, SCHEDULE);
    write_wire(&mut db, sch, PropertyIdentifier::OUT_OF_SERVICE, &[0x11]);
    write_wire(&mut db, sch, PropertyIdentifier::PRESENT_VALUE, REAL_5);
    assert_eq!(target_wire(&db, 9), (REAL_0.to_vec(), NULL.to_vec()));

    let db = Arc::new(RwLock::new(db));
    tick_schedules(&db).await;
    let guard = db.read().await;
    assert_eq!(schedule_present_value(&guard), REAL_5);
    assert_eq!(target_wire(&guard, 9), (REAL_5.to_vec(), REAL_5.to_vec()));
}
