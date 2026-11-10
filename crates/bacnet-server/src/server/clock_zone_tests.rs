//! A zone change on the running server's clock (#2395): the UTC instant and
//! the synchronization are kept, every reader follows the new local time,
//! and no reader sees one zone's offset with the other's daylight-saving
//! status.
use super::*;
use crate::server::event_notifications_tests::local_broadcast_destination;
use crate::server::event_recipient_routing_tests::distribute_from_database_with_bindings;
use crate::server::test_transport::TestTransport;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::schedule::{CalendarObject, ScheduleObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{BACnetCalendarEntry, BACnetTimeValue};
use bacnet_types::enums::{ObjectType, PropertyIdentifier as P};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::RwLock;

fn date(year: u16, month: u8, day: u8, day_of_week: u8) -> Date {
    Date {
        year: (year - 1900) as u8,
        month,
        day,
        day_of_week,
    }
}

fn time(hour: u8, minute: u8, second: u8, hundredths: u8) -> Time {
    Time {
        hour,
        minute,
        second,
        hundredths,
    }
}

fn zone(utc_offset_minutes: i16, daylight_savings: bool) -> ClockConfig {
    ClockConfig::new(utc_offset_minutes, daylight_savings).unwrap()
}

/// A clock in `zone`, set by a UTCTimeSynchronization of Thursday
/// 2024-07-04 at `utc`.
fn synced(zone: ClockConfig, utc: Time) -> Arc<ServerClock> {
    let clock = Arc::new(ServerClock::new(zone));
    clock.synchronize(date(2024, 7, 4, 4), utc, true).unwrap();
    clock
}

fn local(clock: &ServerClock) -> (Date, Time) {
    let frame = clock.read_clock().unwrap();
    (frame.local_date, frame.local_time)
}

fn reader(clock: &Arc<ServerClock>) -> Option<Arc<dyn ClockReader>> {
    Some(Arc::clone(clock) as Arc<dyn ClockReader>)
}

/// A started server over a database holding only its Device, in `zone`, or
/// with no clock.
async fn server(zone: Option<ClockConfig>) -> (BACnetServer<TestTransport>, ObjectIdentifier) {
    let device = DeviceObject::new(DeviceConfig::default()).unwrap();
    let oid = device.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(device)).unwrap();
    let server = BACnetServer::start_with_clock_mode(
        ServerConfig::default(),
        db,
        TestTransport::new(),
        zone,
    )
    .await
    .unwrap();
    (server, oid)
}

/// The Device's Local_Date, Local_Time, UTC_Offset and
/// Daylight_Savings_Status, read under `db`.
fn device_clock(db: &ObjectDatabase, device: ObjectIdentifier) -> (Date, Time, i32, bool) {
    let read = |property| db.get(&device).unwrap().read_property(property, None);
    match (
        read(P::LOCAL_DATE),
        read(P::LOCAL_TIME),
        read(P::UTC_OFFSET),
        read(P::DAYLIGHT_SAVINGS_STATUS),
    ) {
        (
            Ok(PropertyValue::Date(date)),
            Ok(PropertyValue::Time(time)),
            Ok(PropertyValue::Signed(offset)),
            Ok(PropertyValue::Boolean(summer)),
        ) => (date, time, offset, summer),
        other => panic!("no clock: {other:?}"),
    }
}

/// After a synchronization, daylight saving moves local time an hour and an
/// offset change moves it by the difference, across midnight too; the UTC
/// anchor the synchronization stored is untouched, and the clock runs on.
#[tokio::test(start_paused = true)]
async fn a_zone_change_keeps_the_utc_instant_and_the_synchronization() {
    let clock = synced(zone(300, false), time(13, 15, 0, 0));
    let anchor = clock.state.lock().unwrap().synchronized_utc;
    assert!(anchor.is_some());
    assert_eq!(local(&clock), (date(2024, 7, 4, 4), time(8, 15, 0, 0)));

    clock.set_zone(zone(300, true)).unwrap();
    assert_eq!(local(&clock), (date(2024, 7, 4, 4), time(9, 15, 0, 0)));
    let frame = clock.read_clock().unwrap();
    assert_eq!(
        (frame.utc_offset, frame.daylight_savings_status),
        (300, true)
    );
    assert_eq!(clock.zone(), Some(zone(300, true)));

    // Six hours west instead of five, still in daylight saving.
    clock.set_zone(zone(360, true)).unwrap();
    assert_eq!(local(&clock), (date(2024, 7, 4, 4), time(8, 15, 0, 0)));
    // 10:45 east: 24:00 local, which is midnight on Friday; daylight saving
    // then reads 01:00, and standard time puts Thursday back.
    clock.set_zone(zone(-645, false)).unwrap();
    assert_eq!(local(&clock), (date(2024, 7, 5, 5), time(0, 0, 0, 0)));
    clock.set_zone(zone(-645, true)).unwrap();
    assert_eq!(local(&clock), (date(2024, 7, 5, 5), time(1, 0, 0, 0)));
    clock.set_zone(zone(-630, false)).unwrap();
    assert_eq!(local(&clock), (date(2024, 7, 4, 4), time(23, 45, 0, 0)));
    assert_eq!(clock.state.lock().unwrap().synchronized_utc, anchor);

    tokio::time::advance(Duration::from_millis(61_500)).await;
    assert_eq!(local(&clock), (date(2024, 7, 4, 4), time(23, 46, 1, 50)));
    // A later UTC synchronization is read in the zone the clock is in now.
    clock
        .synchronize(date(2024, 7, 4, 4), time(13, 15, 0, 0), true)
        .unwrap();
    assert_eq!(local(&clock), (date(2024, 7, 4, 4), time(23, 45, 0, 0)));
}

/// A zone whose local date would leave the BACnet Date range is refused and
/// changes nothing; so is an offset `ClockConfig` doesn't take, which never
/// reaches the clock.
#[tokio::test(start_paused = true)]
async fn a_refused_zone_changes_nothing() {
    for refused in [14, -1455, 1441] {
        assert!(ClockConfig::new(refused, true).is_err(), "{refused}");
    }
    // 1900-01-01 was a Monday: an hour west of it is 1899.
    let first = Arc::new(ServerClock::new(zone(0, false)));
    first
        .synchronize(date(1900, 1, 1, 1), time(0, 30, 0, 0), true)
        .unwrap();
    let before = *first.state.lock().unwrap();
    assert!(first.set_zone(zone(60, false)).is_err());
    let after = *first.state.lock().unwrap();
    assert_eq!(after.config, before.config);
    assert_eq!(after.synchronized_utc, before.synchronized_utc);
    assert_eq!(local(&first), (date(1900, 1, 1, 1), time(0, 30, 0, 0)));
    // An hour west in daylight saving is still 00:30.
    first.set_zone(zone(60, true)).unwrap();
    assert_eq!(local(&first), (date(1900, 1, 1, 1), time(0, 30, 0, 0)));

    // 2154-12-31 was a Tuesday: daylight saving at 23:30 would be 2155.
    let last = Arc::new(ServerClock::new(zone(0, false)));
    last.synchronize(date(2154, 12, 31, 2), time(23, 30, 0, 0), true)
        .unwrap();
    let before = *last.state.lock().unwrap();
    assert!(last.set_zone(zone(0, true)).is_err());
    // The range is judged at the instant of the change: a quarter-hour east
    // is fine at 23:30, and refused once the clock has run 20 minutes on.
    tokio::time::advance(Duration::from_secs(1_200)).await;
    assert!(last.set_zone(zone(-15, false)).is_err());
    let after = *last.state.lock().unwrap();
    assert_eq!(after.config, before.config);
    assert_eq!(after.synchronized_utc, before.synchronized_utc);
    assert_eq!(local(&last), (date(2154, 12, 31, 2), time(23, 50, 0, 0)));
    last.set_zone(zone(60, false)).unwrap();
    assert_eq!(local(&last), (date(2154, 12, 31, 2), time(22, 50, 0, 0)));
}

/// The step a synchronization makes is judged against `max_step` in the
/// zone its value is stored under, under one hold of the clock's lock: a
/// local value 90 minutes ahead is refused, leaving the clock as it was,
/// and the same value is taken once daylight saving has made it 30 minutes
/// ahead. A UTC value's step is the same in any zone.
#[tokio::test(start_paused = true)]
async fn a_step_is_judged_in_the_zone_its_value_is_stored_under() {
    let hour = Some(Duration::from_secs(3_600));
    let thursday = date(2024, 7, 4, 4);
    // 08:15 local, five hours west.
    let clock = synced(zone(300, false), time(13, 15, 0, 0));
    let holds = || clock.holds.load(Ordering::Relaxed);
    let anchor = clock.state.lock().unwrap().synchronized_utc;
    let mut step = None;

    let before = holds();
    let ahead = clock.synchronize_within(thursday, time(9, 45, 0, 0), false, hour, &mut step);
    assert_eq!(holds() - before, 1);
    assert!(ahead.unwrap_err().to_string().contains("step cap exceeded"));
    assert_eq!(step, Some(90 * 6_000));
    assert_eq!(clock.state.lock().unwrap().synchronized_utc, anchor);
    assert_eq!(local(&clock), (thursday, time(8, 15, 0, 0)));

    clock.set_zone(zone(300, true)).unwrap();
    let before = holds();
    clock
        .synchronize_within(thursday, time(9, 45, 0, 0), false, hour, &mut step)
        .unwrap();
    assert_eq!(holds() - before, 1);
    assert_eq!(step, Some(30 * 6_000));
    assert_eq!(local(&clock), (thursday, time(9, 45, 0, 0)));
    assert_eq!(clock.zone(), Some(zone(300, true)));

    // 09:45 in daylight saving, five hours west, is 13:45 UTC. A UTC value
    // of 14:45 is an hour ahead, the cap exactly, whatever the zone.
    for (offset, summer) in [(300, true), (-645, false)] {
        clock.set_zone(zone(offset, summer)).unwrap();
        let mut step = None;
        let late = clock.synchronize_within(thursday, time(14, 45, 0, 1), true, hour, &mut step);
        assert!(late.is_err(), "{offset}, {summer}");
        assert_eq!(step, Some(60 * 6_000 + 1));
    }
    clock
        .synchronize_within(thursday, time(14, 45, 0, 0), true, hour, &mut step)
        .unwrap();
    assert_eq!(step, Some(60 * 6_000));
    // With no cap any step is taken, and still reported.
    clock
        .synchronize_within(
            date(2024, 7, 5, 5),
            time(14, 45, 0, 0),
            true,
            None,
            &mut step,
        )
        .unwrap();
    assert_eq!(step, Some(24 * 60 * 6_000));
}

/// A builder starts the clock in the zone it is given: the Device's first
/// reads are in it, and so is the Schedule pass that runs as the server
/// starts. The Schedule holds the hour as its value, so its Present_Value
/// after that pass is the local hour, five hours behind the system's UTC.
#[tokio::test(start_paused = true)]
async fn a_builder_starts_the_clock_in_its_zone() {
    let west = zone(300, false);
    let identity = bacnet_objects::DeviceIdentity::new(2395, 0x105).unwrap();
    let mut db = identity
        .clone()
        .with_services(bacnet_objects::device::EXECUTED_SERVICES)
        .build_database()
        .unwrap();
    let mut schedule = ScheduleObject::new(1, "Hour", PropertyValue::Real(-1.0)).unwrap();
    let hours: Vec<_> = (0..24_u8)
        .map(|hour| BACnetTimeValue {
            time: time(hour, 0, 0, 0),
            value: PropertyValue::Real(f32::from(hour)),
        })
        .collect();
    for day in 0..7 {
        schedule.set_weekly_schedule(day, hours.clone()).unwrap();
    }
    let schedule_oid = schedule.object_identifier();
    db.add(Box::new(schedule)).unwrap();

    // The local hour five hours west of the system's UTC, before the start
    // and after the reads: the same, unless an hour turned in between.
    let local_hour = || {
        let utc = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        ((utc.as_secs() / 3_600 + 24 - 5) % 24) as u8
    };
    let earliest = local_hour();
    let server = BACnetServer::generic_builder()
        .device(identity.clone())
        .database(db)
        .transport(TestTransport::new())
        .clock_zone(west)
        .build()
        .await
        .unwrap();
    assert_eq!(server.clock_zone(), Some(west));
    let (_, now, offset, summer) =
        device_clock(&*server.database().read().await, identity.device_oid());
    // The Schedule task's first pass runs once the test lets it.
    tokio::time::sleep(Duration::from_millis(1)).await;
    let scheduled = server
        .database()
        .read()
        .await
        .get(&schedule_oid)
        .unwrap()
        .read_property(P::PRESENT_VALUE, None)
        .unwrap();
    let hours = [earliest, local_hour()];
    assert_eq!((offset, summer), (300, false));
    assert!(hours.contains(&now.hour), "{now:?} isn't in {hours:?}");
    let expected = hours.map(|hour| PropertyValue::Real(f32::from(hour)));
    assert!(expected.contains(&scheduled), "{scheduled:?}, {hours:?}");
}

/// The running server's Device reads the new pair and the moved local time,
/// before any synchronization and after one, which the change keeps. A
/// clockless server refuses.
#[tokio::test(start_paused = true)]
async fn a_running_servers_device_follows_its_zone() {
    let (server, device) = server(Some(zone(0, false))).await;
    assert_eq!(server.clock_zone(), Some(zone(0, false)));
    // Before any synchronization the clock reads the system's UTC.
    server.set_clock_zone(zone(300, false)).await.unwrap();
    assert_eq!(server.clock_zone(), Some(zone(300, false)));
    let (_, _, offset, summer) = device_clock(&*server.database().read().await, device);
    assert_eq!((offset, summer), (300, false));
    let clock = server.clock.clone().unwrap();
    assert!(clock.state.lock().unwrap().synchronized_utc.is_none());

    clock
        .synchronize(date(2024, 7, 4, 4), time(13, 15, 0, 0), true)
        .unwrap();
    assert_eq!(
        device_clock(&*server.database().read().await, device),
        (date(2024, 7, 4, 4), time(8, 15, 0, 0), 300, false)
    );
    let anchor = clock.state.lock().unwrap().synchronized_utc;
    server.set_clock_zone(zone(300, true)).await.unwrap();
    assert_eq!(
        device_clock(&*server.database().read().await, device),
        (date(2024, 7, 4, 4), time(9, 15, 0, 0), 300, true)
    );
    assert_eq!(clock.state.lock().unwrap().synchronized_utc, anchor);
    server.set_clock_zone(zone(-60, true)).await.unwrap();
    assert_eq!(
        device_clock(&*server.database().read().await, device),
        (date(2024, 7, 4, 4), time(15, 15, 0, 0), -60, true)
    );

    let (clockless, _) = self::server(None).await;
    assert_eq!(clockless.clock_zone(), None);
    assert!(clockless.set_clock_zone(zone(300, true)).await.is_err());
}

/// A request reads under a database guard, and the change waits for the
/// write guard: a reader that has the old offset reads the old
/// daylight-saving status too, however long it takes, and the new pair
/// only once it lets go.
#[tokio::test(start_paused = true)]
async fn a_request_reads_one_pair_across_a_change() {
    let (server, device) = server(Some(zone(300, false))).await;
    let request = server.database().read().await;
    let read = |db: &ObjectDatabase, property| {
        db.get(&device)
            .unwrap()
            .read_property(property, None)
            .unwrap()
    };
    assert_eq!(read(&request, P::UTC_OFFSET), PropertyValue::Signed(300));
    let mut change = std::pin::pin!(server.set_clock_zone(zone(-60, true)));
    let waited = tokio::time::timeout(Duration::from_secs(5), &mut change).await;
    assert!(waited.is_err(), "the change ran under a request's guard");
    assert_eq!(
        read(&request, P::DAYLIGHT_SAVINGS_STATUS),
        PropertyValue::Boolean(false)
    );
    assert_eq!(server.clock_zone(), Some(zone(300, false)));
    drop(request);
    change.await.unwrap();
    let (_, _, offset, summer) = device_clock(&*server.database().read().await, device);
    assert_eq!((offset, summer), (-60, true));
}

/// Readers on other threads, while the zone flips between two zones that
/// differ in both fields, only ever see one zone's pair, with that zone's
/// local time: 07:xx five hours west, 14:xx an hour east in daylight saving.
/// The zone keeps flipping until every reader has seen both pairs, so each
/// read through at least one change.
#[test]
fn concurrent_readers_see_the_old_pair_or_the_new() {
    const READERS: usize = 4;
    const FLIPS: usize = 20_000;
    let (west, east) = (zone(300, false), zone(-60, true));
    let clock = synced(west, time(12, 0, 0, 0));
    let done = AtomicBool::new(false);
    let saw_both = AtomicUsize::new(0);
    let start = std::sync::Barrier::new(READERS + 1);
    let seen = std::thread::scope(|scope| {
        let readers: Vec<_> = (0..READERS)
            .map(|_| {
                scope.spawn(|| {
                    start.wait();
                    let mut seen = [0_usize; 2];
                    while !done.load(Ordering::Relaxed) {
                        let frame = clock.read_clock().unwrap();
                        let pair = (frame.utc_offset, frame.daylight_savings_status);
                        let which = match (pair, frame.local_time.hour) {
                            ((300, false), 7) => 0,
                            ((-60, true), 14) => 1,
                            torn => panic!("a torn frame: {torn:?}"),
                        };
                        seen[which] += 1;
                        if seen[which] == 1 && seen[1 - which] > 0 {
                            saw_both.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    seen
                })
            })
            .collect();
        start.wait();
        // Bounded by the readers' progress, never by time. A reader that
        // panicked on a torn frame never reports, so stop for it too.
        let mut flips = 0;
        while flips < FLIPS
            || (saw_both.load(Ordering::Relaxed) < READERS
                && !readers.iter().any(|reader| reader.is_finished()))
        {
            for next in [east, west] {
                clock.set_zone(next).unwrap();
            }
            flips += 2;
            std::thread::yield_now();
        }
        done.store(true, Ordering::Relaxed);
        readers
            .into_iter()
            .map(|reader| reader.join().unwrap())
            .collect::<Vec<_>>()
    });
    // Every read was one of the two pairs (a reader panics otherwise), and
    // every reader saw each of them.
    for (reader, seen) in seen.iter().enumerate() {
        assert!(seen[0] > 0 && seen[1] > 0, "reader {reader}: {seen:?}");
    }
    assert_eq!(clock.zone(), Some(west));
}

/// The next Schedule pass and a Calendar's Present_Value follow the new
/// local time. Nothing is recalculated at the change itself: the Schedule
/// keeps its value until the pass, as it does after a TimeSynchronization
/// that moves local time by the same hour, and then both hold the same
/// value.
#[tokio::test(start_paused = true)]
async fn schedules_and_calendars_judge_by_the_new_local_time() {
    let schedule_oid = ObjectIdentifier::new(ObjectType::SCHEDULE, 1).unwrap();
    let calendar_oid = ObjectIdentifier::new(ObjectType::CALENDAR, 1).unwrap();
    // Thursday from 09:00 on: 21.0; a Calendar for Friday 2024-07-05.
    let build = |clock: &Arc<ServerClock>| {
        let mut schedule = ScheduleObject::new(1, "S", PropertyValue::Real(10.0)).unwrap();
        let from_nine = BACnetTimeValue {
            time: time(9, 0, 0, 0),
            value: PropertyValue::Real(21.0),
        };
        schedule.set_weekly_schedule(3, vec![from_nine]).unwrap();
        let mut calendar = CalendarObject::new(1, "C").unwrap();
        let friday = BACnetCalendarEntry::Date(date(2024, 7, 5, 5));
        calendar.add_date_entry(friday).unwrap();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(schedule)).unwrap();
        db.add(Box::new(calendar)).unwrap();
        db.set_clock_reader(reader(clock));
        Arc::new(RwLock::new(db))
    };
    let value = |db: &ObjectDatabase, oid| {
        let object = db.get(&oid).unwrap();
        object.read_property(P::PRESENT_VALUE, None).unwrap()
    };

    // 08:30 local, five hours west.
    let moved = synced(zone(300, false), time(13, 30, 0, 0));
    let by_zone = build(&moved);
    crate::schedule::tick_schedules(&by_zone).await;
    assert_eq!(
        value(&*by_zone.read().await, schedule_oid),
        PropertyValue::Real(10.0)
    );
    moved.set_zone(zone(300, true)).unwrap();
    assert_eq!(local(&moved).1, time(9, 30, 0, 0));
    assert_eq!(
        value(&*by_zone.read().await, schedule_oid),
        PropertyValue::Real(10.0)
    );
    crate::schedule::tick_schedules(&by_zone).await;
    assert_eq!(
        value(&*by_zone.read().await, schedule_oid),
        PropertyValue::Real(21.0)
    );

    // The same hour by a TimeSynchronization of 09:30 local.
    let resynced = synced(zone(300, false), time(13, 30, 0, 0));
    let by_sync = build(&resynced);
    crate::schedule::tick_schedules(&by_sync).await;
    resynced
        .synchronize(date(2024, 7, 4, 4), time(9, 30, 0, 0), false)
        .unwrap();
    assert_eq!(
        value(&*by_sync.read().await, schedule_oid),
        PropertyValue::Real(10.0)
    );
    crate::schedule::tick_schedules(&by_sync).await;
    assert_eq!(
        value(&*by_sync.read().await, schedule_oid),
        PropertyValue::Real(21.0)
    );

    // Thursday 23:30 an hour west of UTC; daylight saving makes it Friday.
    let late = synced(zone(60, false), time(0, 30, 0, 0));
    late.synchronize(date(2024, 7, 5, 5), time(0, 30, 0, 0), true)
        .unwrap();
    let db = build(&late);
    assert_eq!(local(&late), (date(2024, 7, 4, 4), time(23, 30, 0, 0)));
    assert_eq!(
        value(&*db.read().await, calendar_oid),
        PropertyValue::Boolean(false)
    );
    late.set_zone(zone(60, true)).unwrap();
    assert_eq!(
        value(&*db.read().await, calendar_oid),
        PropertyValue::Boolean(true)
    );
}

/// A Notification Class destination open from 08:00 to 08:59:59.99 takes a
/// notification at 08:30 local, not once daylight saving makes that 09:30,
/// and again when an offset an hour further west makes it 08:30.
#[tokio::test(start_paused = true)]
async fn a_recipient_window_opens_and_closes_by_the_new_local_time() {
    let clock = synced(zone(300, false), time(13, 30, 0, 0));
    let notified = |clock: &Arc<ServerClock>| {
        let mut class = NotificationClass::new(0, "NC-0").unwrap();
        let mut morning = local_broadcast_destination();
        morning.from_time = time(8, 0, 0, 0);
        morning.to_time = time(8, 59, 59, 99);
        class.add_destination(morning).unwrap();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(class)).unwrap();
        db.set_clock_reader(reader(clock));
        let bindings = Arc::new(RwLock::new(
            crate::server::device_bindings::DeviceBindingTable::new(),
        ));
        async move {
            let (broadcasts, unicasts) = distribute_from_database_with_bindings(db, bindings).await;
            broadcasts.len() + unicasts.len()
        }
    };
    assert_eq!(notified(&clock).await, 1);
    clock.set_zone(zone(300, true)).unwrap();
    assert_eq!(local(&clock).1.hour, 9);
    assert_eq!(notified(&clock).await, 0);
    clock.set_zone(zone(360, true)).unwrap();
    assert_eq!(local(&clock).1.hour, 8);
    assert_eq!(notified(&clock).await, 1);
}
