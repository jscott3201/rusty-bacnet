//! TimeSynchronization and UTCTimeSynchronization through the `Responder`
//! and the `Reporter`: which it runs, what each sets, and what the Device
//! then serves.
use super::{date, time};
use crate::clock::{self, Basis, DeviceClock, Zone};
use crate::cov::{CovPolicy, SubscriberEndpoint};
use crate::report::{ReportConfig, Reporter};
use crate::responder::{self, Access, Inbound, Limits, Outcome, Responder};
use bacnet_encoding::apdu::{Apdu, UnconfirmedRequest, encode_apdu};
use bacnet_objects::DeviceIdentity;
use bacnet_objects::database::ObjectDatabase;
use bacnet_services::device_mgmt::TimeSynchronizationRequest;
use bacnet_time::{Duration, Instant};
use bacnet_types::bitstring::ServicesSupported;
use bacnet_types::enums::{
    ObjectType, PropertyIdentifier as P, ServiceSupported, UnconfirmedServiceChoice,
};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};
use bytes::{Bytes, BytesMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const DEVICE: u32 = 77;
const LOCAL: UnconfirmedServiceChoice = UnconfirmedServiceChoice::TIME_SYNCHRONIZATION;
const UTC: UnconfirmedServiceChoice = UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION;

/// The host's timer, which the test moves by hand.
#[derive(Clone, Default)]
struct Timer(Arc<AtomicU64>);

impl Timer {
    fn clock(&self, zone: Zone) -> DeviceClock {
        let nanos = Arc::clone(&self.0);
        DeviceClock::new(
            zone,
            Arc::new(move || Instant::from_nanos(nanos.load(Ordering::Relaxed))),
        )
    }

    fn advance(&self, by: Duration) {
        self.0.fetch_add(by.as_nanos(), Ordering::Relaxed);
    }

    fn now(&self) -> Instant {
        Instant::from_nanos(self.0.load(Ordering::Relaxed))
    }
}

/// A Device listing the responder's services and `clock_services`, and a
/// responder for it, with a clock in UTC_Offset 300 when `clocked`.
fn device(
    clock_services: &[ServiceSupported],
    clocked: Option<&Timer>,
) -> (Responder, ObjectDatabase) {
    let services: Vec<_> = responder::SERVICES
        .iter()
        .chain(clock_services)
        .copied()
        .collect();
    let identity = DeviceIdentity::new(DEVICE, 0x105)
        .unwrap()
        .with_services(&services);
    let mut db = identity.build_database().unwrap();
    let responder = Responder::new(identity, Limits::default()).unwrap();
    let responder = match clocked {
        Some(timer) => {
            let zone = Zone::new(300, false).unwrap();
            responder.with_clock(timer.clock(zone), &mut db)
        }
        None => responder,
    };
    (responder, db)
}

fn sync(service: UnconfirmedServiceChoice, date: Date, time: Time) -> Bytes {
    let mut parameters = BytesMut::new();
    TimeSynchronizationRequest { date, time }.encode(&mut parameters);
    unconfirmed(service, parameters.freeze())
}

fn unconfirmed(service: UnconfirmedServiceChoice, parameters: Bytes) -> Bytes {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: service,
            service_request: parameters,
        }),
    )
    .unwrap();
    apdu.freeze()
}

/// `apdu` through `handle`: its outcome, after checking it wrote nothing.
fn handle(responder: &mut Responder, db: &mut ObjectDatabase, apdu: Bytes) -> Outcome {
    let mut out = BytesMut::new();
    let written = responder.handle(db, Inbound::new(apdu, 1476), &mut out);
    assert!(out.is_empty(), "{out:02X?}");
    assert!(written.objects.is_empty());
    written.outcome
}

fn read(db: &ObjectDatabase, property: P) -> Option<PropertyValue> {
    let device = ObjectIdentifier::new(ObjectType::DEVICE, DEVICE).unwrap();
    db.get(&device).unwrap().read_property(property, None).ok()
}

/// The Device's Local_Date and Local_Time.
fn local(db: &ObjectDatabase) -> (Date, Time) {
    match (read(db, P::LOCAL_DATE), read(db, P::LOCAL_TIME)) {
        (Some(PropertyValue::Date(date)), Some(PropertyValue::Time(time))) => (date, time),
        other => panic!("no clock: {other:?}"),
    }
}

fn lists(db: &ObjectDatabase, service: ServiceSupported) -> bool {
    match read(db, P::PROTOCOL_SERVICES_SUPPORTED) {
        Some(PropertyValue::BitString { data, .. }) => {
            ServicesSupported::from_bacnet(&data).contains(service)
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn time_synchronizations_take_the_write_guard() {
    for service in [LOCAL, UTC] {
        let apdu = sync(service, date(2024, 7, 4, 4), time(9, 0, 0, 0));
        assert_eq!(Responder::classify(&apdu), Access::Write);
    }
}

/// With a clock the Device serves the four clock properties and lists both
/// services. Unsynchronized it reads 1990-01-01 from when the clock was
/// built; a UTCTimeSynchronization sets it, less UTC_Offset, and it runs on
/// with the host's timer.
#[test]
fn a_utc_synchronization_sets_local_date_and_time() {
    let timer = Timer::default();
    let (mut responder, mut db) = device(clock::SERVICES, Some(&timer));
    assert!(lists(&db, ServiceSupported::TIME_SYNCHRONIZATION));
    assert!(lists(&db, ServiceSupported::UTC_TIME_SYNCHRONIZATION));
    assert_eq!(read(&db, P::UTC_OFFSET), Some(PropertyValue::Signed(300)));
    assert_eq!(
        read(&db, P::DAYLIGHT_SAVINGS_STATUS),
        Some(PropertyValue::Boolean(false))
    );
    timer.advance(Duration::from_secs(61));
    assert_eq!(local(&db), (date(1990, 1, 1, 1), time(0, 1, 1, 0)));

    let apdu = sync(UTC, date(2024, 7, 4, 4), time(13, 15, 0, 0));
    assert_eq!(handle(&mut responder, &mut db, apdu), Outcome::Silent);
    assert_eq!(local(&db), (date(2024, 7, 4, 4), time(8, 15, 0, 0)));
    let clock = responder.clock().unwrap().clock();
    assert!(clock.is_synced());
    timer.advance(Duration::from_millis(1_500));
    assert_eq!(local(&db), (date(2024, 7, 4, 4), time(8, 15, 1, 50)));

    // A local one reads back as given.
    let apdu = sync(LOCAL, date(2024, 7, 5, 5), time(0, 0, 0, 0));
    assert_eq!(handle(&mut responder, &mut db, apdu), Outcome::Silent);
    assert_eq!(local(&db), (date(2024, 7, 5, 5), time(0, 0, 0, 0)));
}

/// A Device that doesn't list a service doesn't act on it, and a value that
/// isn't specific, or parameters that don't decode, change nothing.
#[test]
fn only_a_listed_service_with_a_specific_value_sets_the_clock() {
    let timer = Timer::default();
    let only_local = [ServiceSupported::TIME_SYNCHRONIZATION];
    let (mut responder, mut db) = device(&only_local, Some(&timer));
    assert!(!lists(&db, ServiceSupported::UTC_TIME_SYNCHRONIZATION));
    let restart = (date(1990, 1, 1, 1), time(0, 0, 0, 0));
    let ignored = [
        sync(UTC, date(2024, 7, 4, 4), time(13, 15, 0, 0)),
        sync(LOCAL, date(2024, 7, 4, 5), time(13, 15, 0, 0)),
        sync(
            LOCAL,
            date(2024, 7, 4, 4),
            time(Time::UNSPECIFIED, 15, 0, 0),
        ),
        unconfirmed(LOCAL, Bytes::from_static(&[0xA4, 0x7C, 0x07])),
    ];
    for apdu in ignored {
        assert_eq!(
            handle(&mut responder, &mut db, apdu.clone()),
            Outcome::Silent
        );
        assert_eq!(local(&db), restart, "{apdu:02X?}");
        assert!(!responder.clock().unwrap().clock().is_synced());
    }
    let apdu = sync(LOCAL, date(2024, 7, 4, 4), time(13, 15, 0, 0));
    assert_eq!(handle(&mut responder, &mut db, apdu), Outcome::Silent);
    assert_eq!(local(&db), (date(2024, 7, 4, 4), time(13, 15, 0, 0)));

    // Listed by the identity but with no clock: the Device has none to
    // list them by, and the responder leaves them to the host.
    let (mut clockless, mut db) = device(clock::SERVICES, None);
    assert!(!lists(&db, ServiceSupported::TIME_SYNCHRONIZATION));
    assert_eq!(read(&db, P::LOCAL_TIME), None);
    let apdu = sync(LOCAL, date(2024, 7, 4, 4), time(13, 15, 0, 0));
    let mut out = BytesMut::new();
    let written = clockless.write(&mut db, None, Inbound::new(apdu.clone(), 1476), &mut out);
    assert_eq!(written.outcome, Outcome::Unhandled);
    assert_eq!(handle(&mut clockless, &mut db, apdu), Outcome::Silent);
    assert_eq!(read(&db, P::LOCAL_TIME), None);
}

/// A real-time clock on the board sets the time directly; it counts as
/// synchronized. A responder with no clock refuses it.
#[test]
fn a_host_sets_the_clock_from_its_own_rtc() {
    let timer = Timer::default();
    let (mut responder, mut db) = device(clock::SERVICES, Some(&timer));
    responder
        .set_time(
            &mut db,
            Basis::Local,
            date(2026, 11, 4, 3),
            time(7, 30, 0, 0),
        )
        .unwrap();
    assert_eq!(local(&db), (date(2026, 11, 4, 3), time(7, 30, 0, 0)));
    assert!(responder.clock().unwrap().clock().is_synced());
    assert!(
        responder
            .set_time(
                &mut db,
                Basis::Local,
                date(2026, 11, 4, 4),
                time(7, 30, 0, 0)
            )
            .is_err()
    );
    assert_eq!(local(&db), (date(2026, 11, 4, 3), time(7, 30, 0, 0)));

    let (mut clockless, mut db) = device(&[], None);
    let refused = clockless.set_time(&mut db, Basis::Utc, date(2026, 11, 4, 3), time(7, 0, 0, 0));
    assert!(refused.is_err());
}

/// The `Reporter` hands a synchronization to its responder, which sets the
/// clock as it does alone.
#[test]
fn the_reporter_runs_them_through_its_responder() {
    let timer = Timer::default();
    let (responder, mut db) = device(clock::SERVICES, Some(&timer));
    let mut reporter = Reporter::new(responder, ReportConfig::new(CovPolicy::default())).unwrap();
    let peer = SubscriberEndpoint::new(&[3], None);
    let apdu = sync(UTC, date(2024, 7, 4, 4), time(13, 15, 0, 0));
    let mut out = BytesMut::new();
    let written = reporter.handle(
        &mut db,
        &peer,
        Inbound::new(apdu, 1476),
        Instant::ZERO,
        &mut out,
    );
    assert_eq!(written.outcome, Outcome::Silent);
    assert!(out.is_empty());
    assert_eq!(local(&db), (date(2024, 7, 4, 4), time(8, 15, 0, 0)));
}

/// A host sets the clock of a running `Reporter` from its real-time clock,
/// and a later synchronization through `handle` sets the same clock: the
/// reporter, its responder and the database read one clock throughout.
#[test]
fn a_reporter_takes_the_hosts_rtc_and_later_syncs() {
    let timer = Timer::default();
    let (responder, mut db) = device(clock::SERVICES, Some(&timer));
    let mut reporter = Reporter::new(responder, ReportConfig::new(CovPolicy::default())).unwrap();
    let clock = |reporter: &Reporter| *reporter.clock().unwrap().clock();
    assert!(!clock(&reporter).is_synced());
    assert_eq!(clock(&reporter).window(Instant::ZERO), None);

    // Wednesday 2026-11-04, 07:30 local.
    reporter
        .set_time(
            &mut db,
            Basis::Local,
            date(2026, 11, 4, 3),
            time(7, 30, 0, 0),
        )
        .unwrap();
    assert!(clock(&reporter).is_synced());
    assert_eq!(local(&db), (date(2026, 11, 4, 3), time(7, 30, 0, 0)));
    let window = clock(&reporter).window(Instant::ZERO);
    let wednesday = bacnet_types::bitstring::DaysOfWeek::WEDNESDAY;
    assert_eq!(window, Some((wednesday, time(7, 30, 0, 0))));
    // A value that isn't specific is refused and changes nothing.
    let refused = reporter.set_time(&mut db, Basis::Utc, date(2026, 11, 4, 4), time(7, 0, 0, 0));
    assert!(refused.is_err());
    assert_eq!(local(&db), (date(2026, 11, 4, 3), time(7, 30, 0, 0)));

    timer.advance(Duration::from_secs(60));
    let apdu = sync(UTC, date(2026, 11, 4, 3), time(13, 0, 0, 0));
    let peer = SubscriberEndpoint::new(&[3], None);
    let mut out = BytesMut::new();
    let request = Inbound::new(apdu, 1476);
    let now = Instant::from_secs(60);
    let written = reporter.handle(&mut db, &peer, request, now, &mut out);
    assert_eq!(written.outcome, Outcome::Silent);
    assert_eq!(local(&db), (date(2026, 11, 4, 3), time(8, 0, 0, 0)));
    let window = clock(&reporter).window(timer.now());
    assert_eq!(window, Some((wednesday, time(8, 0, 0, 0))));

    let (clockless, mut db) = device(&[], None);
    let mut clockless = Reporter::new(clockless, ReportConfig::new(CovPolicy::default())).unwrap();
    assert!(clockless.clock().is_none());
    let refused = clockless.set_time(&mut db, Basis::Utc, date(2026, 11, 4, 3), time(7, 0, 0, 0));
    assert!(refused.is_err());
}

/// A host moves the zone of a running responder's clock: the Device reads
/// the new UTC_Offset and Daylight_Savings_Status and the moved Local_Date
/// and Local_Time at once, the clock stays synchronized, and a later
/// synchronization is read in the new zone. A responder with no clock
/// refuses.
#[test]
fn a_host_changes_the_zone_and_the_device_reads_it() {
    let timer = Timer::default();
    let (mut responder, mut db) = device(clock::SERVICES, Some(&timer));
    let zone = |offset, summer| Zone::new(offset, summer).unwrap();
    let pair = |db: &ObjectDatabase| {
        (
            read(db, P::UTC_OFFSET).unwrap(),
            read(db, P::DAYLIGHT_SAVINGS_STATUS).unwrap(),
        )
    };
    let signed = PropertyValue::Signed;
    let summer = PropertyValue::Boolean;

    // Before any synchronization the count from 1990 goes on in the new
    // zone, unsynchronized.
    timer.advance(Duration::from_secs(61));
    responder.set_zone(&mut db, zone(240, false)).unwrap();
    assert_eq!(pair(&db), (signed(240), summer(false)));
    assert_eq!(local(&db), (date(1990, 1, 1, 1), time(0, 1, 1, 0)));
    assert!(!responder.clock().unwrap().clock().is_synced());

    // 13:15 UTC, four hours west: 09:15.
    let apdu = sync(UTC, date(2024, 7, 4, 4), time(13, 15, 0, 0));
    assert_eq!(handle(&mut responder, &mut db, apdu), Outcome::Silent);
    assert_eq!(local(&db), (date(2024, 7, 4, 4), time(9, 15, 0, 0)));

    responder.set_zone(&mut db, zone(240, true)).unwrap();
    assert_eq!(pair(&db), (signed(240), summer(true)));
    assert_eq!(local(&db), (date(2024, 7, 4, 4), time(10, 15, 0, 0)));
    let clock = *responder.clock().unwrap().clock();
    assert!(clock.is_synced());
    assert_eq!(clock.zone(), zone(240, true));

    // 12:15 east, in daylight saving: 02:30 on Friday.
    responder.set_zone(&mut db, zone(-735, true)).unwrap();
    assert_eq!(pair(&db), (signed(-735), summer(true)));
    assert_eq!(local(&db), (date(2024, 7, 5, 5), time(2, 30, 0, 0)));
    timer.advance(Duration::from_millis(1_500));
    assert_eq!(local(&db), (date(2024, 7, 5, 5), time(2, 30, 1, 50)));

    // A local synchronization reads back as given, and a UTC one in the
    // zone the clock is in now.
    let apdu = sync(LOCAL, date(2024, 7, 5, 5), time(6, 0, 0, 0));
    assert_eq!(handle(&mut responder, &mut db, apdu), Outcome::Silent);
    assert_eq!(local(&db), (date(2024, 7, 5, 5), time(6, 0, 0, 0)));
    let apdu = sync(UTC, date(2024, 7, 4, 4), time(13, 15, 0, 0));
    assert_eq!(handle(&mut responder, &mut db, apdu), Outcome::Silent);
    assert_eq!(local(&db), (date(2024, 7, 5, 5), time(2, 30, 0, 0)));

    let (mut clockless, mut db) = device(&[], None);
    assert!(clockless.set_zone(&mut db, zone(240, true)).is_err());
    assert_eq!(read(&db, P::UTC_OFFSET), None);
}

/// A running `Reporter` takes the zone change through its responder: its
/// clock's window, which recipient windows are judged by, and the Device's
/// reads move together, and a zone the clock refuses changes neither.
#[test]
fn a_reporter_takes_a_zone_change() {
    let timer = Timer::default();
    let (responder, mut db) = device(clock::SERVICES, Some(&timer));
    let mut reporter = Reporter::new(responder, ReportConfig::new(CovPolicy::default())).unwrap();
    let zone = |offset, summer| Zone::new(offset, summer).unwrap();
    let window = |reporter: &Reporter, timer: &Timer| {
        let clock = reporter.clock().unwrap().clock();
        clock.window(timer.now())
    };
    let thursday = bacnet_types::bitstring::DaysOfWeek::THURSDAY;

    // Thursday 2024-07-04, 08:15 local, five hours west.
    reporter
        .set_time(&mut db, Basis::Utc, date(2024, 7, 4, 4), time(13, 15, 0, 0))
        .unwrap();
    assert_eq!(
        window(&reporter, &timer),
        Some((thursday, time(8, 15, 0, 0)))
    );
    reporter.set_zone(&mut db, zone(300, true)).unwrap();
    assert_eq!(
        window(&reporter, &timer),
        Some((thursday, time(9, 15, 0, 0)))
    );
    assert_eq!(local(&db), (date(2024, 7, 4, 4), time(9, 15, 0, 0)));
    assert_eq!(
        read(&db, P::DAYLIGHT_SAVINGS_STATUS),
        Some(PropertyValue::Boolean(true))
    );

    // Monday 1900-01-01, 04:30 UTC, is 00:30 local; standard time there
    // would be 1899.
    reporter
        .set_time(&mut db, Basis::Utc, date(1900, 1, 1, 1), time(4, 30, 0, 0))
        .unwrap();
    assert!(reporter.set_zone(&mut db, zone(300, false)).is_err());
    assert_eq!(local(&db), (date(1900, 1, 1, 1), time(0, 30, 0, 0)));
    assert_eq!(read(&db, P::UTC_OFFSET), Some(PropertyValue::Signed(300)));
    assert_eq!(
        read(&db, P::DAYLIGHT_SAVINGS_STATUS),
        Some(PropertyValue::Boolean(true))
    );

    let (clockless, mut db) = device(&[], None);
    let mut clockless = Reporter::new(clockless, ReportConfig::new(CovPolicy::default())).unwrap();
    assert!(clockless.set_zone(&mut db, zone(300, true)).is_err());
}
