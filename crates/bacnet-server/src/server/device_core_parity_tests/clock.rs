//! The core's Device clock against the server's (#2321): the same
//! synchronizations draw the same Local_Date and Local_Time octets, and the
//! same date and time open the same Notification Class recipient windows.
use super::*;
use crate::server::clock::{ClockConfig, ServerClock};
use bacnet_device_core::clock::{
    self as core_clock, Basis, DeviceClock, MonotonicNow, SyncedClock, Zone, destination_open,
};
use bacnet_objects::event::EventTransition;
use bacnet_objects::notification_class::{
    NotificationClass, RecipientLookupOutcome, lookup_notification_recipients,
};
use bacnet_services::device_mgmt::TimeSynchronizationRequest;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{BACnetDestination, BACnetRecipient};
use std::sync::atomic::{AtomicU64, Ordering};

const LOCAL: UnconfirmedServiceChoice = UnconfirmedServiceChoice::TIME_SYNCHRONIZATION;
const UTC: UnconfirmedServiceChoice = UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION;

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

/// Synchronizations at the edges: a day's last hundredth in UTC and in
/// local time, a leap day reached across an offset, a year end and a
/// Sunday-to-Monday turn.
fn syncs() -> [(Basis, Date, Time); 6] {
    [
        (Basis::Utc, date(2024, 7, 6, 6), time(4, 59, 59, 99)),
        (Basis::Local, date(2024, 7, 6, 6), time(23, 59, 59, 99)),
        (Basis::Local, date(2024, 12, 31, 2), time(23, 59, 59, 99)),
        (Basis::Utc, date(2024, 3, 1, 5), time(0, 30, 0, 25)),
        (Basis::Local, date(2025, 3, 2, 7), time(23, 59, 59, 99)),
        (Basis::Utc, date(2024, 7, 8, 1), time(0, 0, 0, 0)),
    ]
}

const ZONES: [(i16, bool); 5] = [
    (0, false),
    (300, true),
    (-570, false),
    (-1440, false),
    (1440, true),
];

/// The core's timer, pinned by the test to the instant the server serves a
/// request: the server answers before the paused clock moves on, the core
/// after `deliver`'s sleep.
#[derive(Clone, Default)]
struct Timer(Arc<AtomicU64>);

impl Timer {
    fn pin(&self) {
        let now = crate::runtime_clock::core_now();
        self.0.store(now.as_nanos(), Ordering::Relaxed);
    }

    fn reader(&self) -> Arc<MonotonicNow> {
        let nanos = Arc::clone(&self.0);
        Arc::new(move || bacnet_time::Instant::from_nanos(nanos.load(Ordering::Relaxed)))
    }
}

impl Parity {
    /// Both sides with a Device clock in UTC_Offset `offset` and
    /// daylight-saving state `summer`: the server's own, and a
    /// `DeviceClock` on `timer` lent by the core's `Responder`.
    async fn start_clocked(offset: i16, summer: bool, timer: &Timer) -> Self {
        let identity = identity();
        let (transport, tx) = TestTransport::inbound(16);
        let sent = transport.sent();
        let config = ServerConfig {
            device: identity.clone(),
            ..ServerConfig::default()
        };
        let clock = ClockConfig::new(offset, summer).unwrap();
        let server = BACnetServer::start_with_clock(config, database(&identity), transport, clock)
            .await
            .unwrap();
        let budget = server.config.read_property_multiple_budget;
        let limits = Limits::new(budget.max_result_elements).with_rpm(budget);
        let core_identity = server
            .config
            .device
            .clone()
            .with_segmentation(Segmentation::NONE);
        let mut db = database(&identity);
        timer.pin();
        let clock = DeviceClock::new(Zone::new(offset, summer).unwrap(), timer.reader());
        let responder = Responder::new(core_identity, limits)
            .unwrap()
            .with_clock(clock, &mut db);
        let reporter = reporter(responder, &server.config);
        tokio::time::sleep(Duration::from_millis(50)).await;
        sent.clear();
        Self {
            server,
            tx,
            sent,
            reporter,
            db,
            requests: 0,
        }
    }
}

fn sync(service: UnconfirmedServiceChoice, date: Date, time: Time) -> Bytes {
    let mut body = BytesMut::new();
    TimeSynchronizationRequest { date, time }.encode(&mut body);
    unconfirmed(service, body)
}

/// Each synchronization, local and UTC, to both sides, in five zones; then
/// the Device's clock properties from both, as served and a second and a
/// half later. Protocol_Services_Supported lists the two services on both.
#[tokio::test(start_paused = true)]
async fn time_synchronization_rows_match() {
    let device = oid(ObjectType::DEVICE, DEVICE);
    let properties = [
        P::LOCAL_DATE,
        P::LOCAL_TIME,
        P::UTC_OFFSET,
        P::DAYLIGHT_SAVINGS_STATUS,
    ];
    for (offset, summer) in ZONES {
        let timer = Timer::default();
        let mut parity = Parity::start_clocked(offset, summer, &timer).await;
        let services = read_property(1, device, P::PROTOCOL_SERVICES_SUPPORTED, None);
        parity.same("services", services, 1476).await;
        for (basis, date, time) in syncs() {
            let service = match basis {
                Basis::Local => LOCAL,
                Basis::Utc => UTC,
            };
            timer.pin();
            let synced = parity.same("sync", sync(service, date, time), 1476).await;
            assert_eq!(synced, None);
            for _ in 0..2 {
                for property in properties {
                    timer.pin();
                    let read = read_property(2, device, property, None);
                    let what =
                        format!("{property:?} at {offset}, {summer} after {date:?} {time:?}");
                    assert!(parity.same(&what, read, 1476).await.is_some());
                }
                tokio::time::sleep(Duration::from_millis(1_500)).await;
            }
        }
    }
}

/// The destinations the window rows judge, process identifiers 1 to 9.
fn destinations() -> Vec<BACnetDestination> {
    let weekdays = DaysOfWeek::all() - DaysOfWeek::SATURDAY - DaysOfWeek::SUNDAY;
    let unspecified = time(Time::UNSPECIFIED, 0, 0, 0);
    let rows = [
        (DaysOfWeek::all(), time(0, 0, 0, 0), time(23, 59, 59, 99)),
        (weekdays, time(8, 0, 0, 0), time(17, 0, 0, 0)),
        (DaysOfWeek::SATURDAY, time(0, 0, 0, 0), time(23, 59, 59, 99)),
        (DaysOfWeek::SUNDAY, time(0, 0, 0, 0), time(0, 0, 0, 0)),
        (DaysOfWeek::SATURDAY, time(22, 0, 0, 0), time(2, 0, 0, 0)),
        (DaysOfWeek::all(), time(23, 0, 0, 0), time(1, 0, 0, 0)),
        (DaysOfWeek::MONDAY, time(0, 0, 0, 0), time(0, 0, 0, 99)),
        (
            DaysOfWeek::TUESDAY | DaysOfWeek::THURSDAY,
            unspecified,
            unspecified,
        ),
        (DaysOfWeek::FRIDAY, time(2, 30, 0, 0), time(2, 30, 0, 25)),
    ];
    rows.into_iter()
        .zip(1..)
        .map(
            |((valid_days, from_time, to_time), process)| BACnetDestination {
                valid_days,
                from_time,
                to_time,
                recipient: BACnetRecipient::Device(oid(ObjectType::DEVICE, 100 + process)),
                process_identifier: process,
                issue_confirmed_notifications: false,
                transitions: if process == 9 {
                    EventTransitionBits::TO_FAULT
                } else {
                    EventTransitionBits::all()
                },
            },
        )
        .collect()
}

/// The process identifiers the server selects for `transition` at `frame`'s
/// day and time, as its distribution does with a valid Device clock.
fn server_selects(
    db: &ObjectDatabase,
    frame: bacnet_objects::clock::ClockFrame,
    transition: EventTransition,
) -> Vec<u32> {
    let today = frame.day_of_week().unwrap();
    match lookup_notification_recipients(db, 5, transition, today, &frame.local_time) {
        RecipientLookupOutcome::Matched(matched) => matched.iter().map(|row| row.1).collect(),
        RecipientLookupOutcome::NoMatchingDestinations => Vec::new(),
        other => panic!("{other:?}"),
    }
}

/// The server's clock and the core's, synchronized alike in each zone, read
/// the same local day and time as they run on, so the same destinations
/// open for each transition: across midnight, a weekday turn, daylight
/// saving and offsets of a whole day either way.
#[tokio::test(start_paused = true)]
async fn recipient_windows_match_the_servers() {
    let destinations = destinations();
    let mut class = NotificationClass::new(5, "NC-5").unwrap();
    for destination in &destinations {
        class.add_destination(destination.clone()).unwrap();
    }
    let mut db = ObjectDatabase::new();
    db.add(Box::new(class)).unwrap();
    let steps = [0, 10, 1_990, 7_200_000, 90_000_000];
    let anchor = bacnet_time::Instant::from_secs(1_000);
    let mut opened = BTreeSet::new();
    for (offset, summer) in ZONES {
        for (basis, date, time) in syncs() {
            let server = ServerClock::new(ClockConfig::new(offset, summer).unwrap());
            server.synchronize(date, time, basis == Basis::Utc).unwrap();
            let mut core = SyncedClock::new(Zone::new(offset, summer).unwrap(), anchor);
            core.set(basis, date, time, anchor).unwrap();
            let mut elapsed = 0;
            for step in steps {
                tokio::time::advance(Duration::from_millis(step - elapsed)).await;
                elapsed = step;
                let now = anchor + bacnet_time::Duration::from_millis(step);
                let frame = server.read_clock().unwrap();
                let window = core.window(now).unwrap();
                assert_eq!(window, (frame.day_of_week().unwrap(), frame.local_time));
                assert_eq!(core.frame(now), Some(frame));
                for transition in [
                    EventTransition::ToOffnormal,
                    EventTransition::ToFault,
                    EventTransition::ToNormal,
                ] {
                    let core_selects: Vec<u32> = destinations
                        .iter()
                        .filter(|dest| destination_open(dest, Some(window)))
                        .filter(|dest| dest.transitions.intersects(transition.bit_mask()))
                        .map(|dest| dest.process_identifier)
                        .collect();
                    let server_selects = server_selects(&db, frame, transition);
                    assert_eq!(core_selects, server_selects, "{frame:?} {transition:?}");
                    opened.extend(core_selects);
                }
            }
        }
    }
    // Every destination opened somewhere, so each row judged both ways.
    assert_eq!(opened, (1..=9).collect());
    // The services the core's clock runs are the server's.
    for service in core_clock::SERVICES {
        assert!(EXECUTED_SERVICES.contains(service));
    }
}
