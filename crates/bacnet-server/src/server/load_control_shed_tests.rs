//! A Load Control shed request over the wire (#2464): a client writes the
//! request, the server runs the Clause 12.28 state machine on the Device
//! clock (paused time, a clock set by hand), the application's
//! `LoadShedder` answers it, and each shed state reaches a SubscribeCOV
//! subscriber, the ones time alone brings included.

use super::*;
use bacnet_objects::clock::ClockFrame;
use bacnet_objects::load_control::{
    LoadControlPersistence, LoadControlSnapshot, LoadShedder, ShedProgress, ShedRequest, ShedStatus,
};
use bacnet_types::primitives::{Date, Time};
use std::sync::Mutex as StdMutex;

/// 15:mm:00 on the harness's day, as the Device clock reads it.
fn clock_at(minute: u8) -> ClockFrame {
    ClockFrame {
        local_time: Time {
            minute,
            ..at(0).local_time
        },
        ..at(0)
    }
}

/// 15:mm:00 on the harness's day, as a client writes Start_Time.
fn start_at(minute: u8) -> Vec<u8> {
    let frame = clock_at(minute);
    encode_date_time(frame.local_date, frame.local_time)
}

fn encode_date_time(date: Date, time: Time) -> Vec<u8> {
    let mut bytes = encode(PropertyValue::Date(date));
    bytes.extend(encode(PropertyValue::Time(time)));
    bytes
}

fn set_clock(h: &Harness, minute: u8) {
    *h.clock.0.lock().unwrap() = clock_at(minute);
}

/// An application whose answer the test sets, counting its stops.
struct App {
    answer: StdMutex<ShedStatus>,
    asked: StdMutex<Vec<ShedRequest>>,
    stops: StdMutex<usize>,
}

impl App {
    fn answer(&self, progress: ShedProgress, actual: u64) {
        *self.answer.lock().unwrap() = ShedStatus::new(
            progress,
            BACnetShedLevel::Level(2),
            BACnetShedLevel::Level(actual),
        );
    }
}

impl LoadShedder for App {
    fn shed(&self, request: &ShedRequest) -> ShedStatus {
        self.asked.lock().unwrap().push(request.clone());
        self.answer.lock().unwrap().clone()
    }

    fn stop(&self) {
        *self.stops.lock().unwrap() += 1;
    }
}

/// A harness serving LC-1 with three shed levels and `app` installed.
async fn harness(app: &Arc<App>) -> Harness {
    let mut object = LoadControlObject::new(1, "LC-1").unwrap();
    object
        .set_shed_levels(vec![
            (1, "Dim the lights".into()),
            (2, "Raise cooling setpoints".into()),
            (3, "Stop the chillers".into()),
        ])
        .unwrap();
    object.set_shedder(app.clone());
    Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(object)).unwrap()
    })
    .await
}

fn app() -> Arc<App> {
    Arc::new(App {
        answer: StdMutex::new(ShedStatus::new(
            ShedProgress::Waiting,
            BACnetShedLevel::Level(2),
            BACnetShedLevel::Level(0),
        )),
        asked: StdMutex::new(Vec::new()),
        stops: StdMutex::new(0),
    })
}

fn lc1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LOAD_CONTROL, 1).unwrap()
}

/// A SubscribeCOV report of LC-1: its state, then the request properties.
fn report(state: u32, requested: u64, start: Vec<u8>, duration: u64, window: u64) -> Values {
    vec![
        (PV, enumerated(state)),
        (SF, normal()),
        (
            PropertyIdentifier::REQUESTED_SHED_LEVEL,
            vec![0x19, u8::try_from(requested).unwrap()],
        ),
        (PropertyIdentifier::START_TIME, start),
        (PropertyIdentifier::SHED_DURATION, unsigned(duration)),
        (PropertyIdentifier::DUTY_WINDOW, unsigned(window)),
    ]
}

fn unspecified() -> Vec<u8> {
    vec![0xA4, 0xFF, 0xFF, 0xFF, 0xFF, 0xB4, 0xFF, 0xFF, 0xFF, 0xFF]
}

/// Read `property` of LC-1 from the server's database, encoded.
async fn read(h: &Harness, property: PropertyIdentifier) -> Vec<u8> {
    let db = h.server.database().read().await;
    encode(
        db.get(&lc1())
            .unwrap()
            .read_property(property, None)
            .unwrap(),
    )
}

#[tokio::test(start_paused = true)]
async fn a_shed_request_runs_on_the_device_clock_and_reports_each_state() {
    let app = app();
    let mut h = harness(&app).await;
    let oid = lc1();
    set_clock(&h, 0);
    assert_eq!(
        subscribed(&mut h, oid).await,
        report(0, 0, unspecified(), 0, 0)
    );

    // The request: level 2 for 30 minutes over a 10-minute window, from
    // 15:05. Each parameter write reports, and nothing starts until the
    // Start_Time write, which before #2464 was refused.
    let parameters = [
        (PropertyIdentifier::REQUESTED_SHED_LEVEL, vec![0x19, 0x02]),
        (PropertyIdentifier::SHED_DURATION, unsigned(30)),
        (PropertyIdentifier::DUTY_WINDOW, unsigned(10)),
    ];
    for (property, value) in parameters {
        write(&mut h, oid, property, value).await;
        h.cov_notification().await;
    }
    write(&mut h, oid, PropertyIdentifier::START_TIME, start_at(5)).await;
    // SHED_REQUEST_PENDING: the application expects level 2 by 15:05.
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(1, 2, start_at(5), 30, 10)
    );
    assert_eq!(
        read(&h, PropertyIdentifier::EXPECTED_SHED_LEVEL).await,
        vec![0x19, 2]
    );
    {
        let asked = app.asked.lock().unwrap();
        let request = asked.last().unwrap();
        assert!(!request.started);
        assert_eq!(request.shed_level, Some(1));
    }

    // 15:05 comes, and the application complies: SHED_COMPLIANT without
    // any write, from the server's once-a-second pass.
    app.answer(ShedProgress::Compliant, 2);
    set_clock(&h, 5);
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(2, 2, start_at(5), 30, 10)
    );
    assert_eq!(
        read(&h, PropertyIdentifier::ACTUAL_SHED_LEVEL).await,
        vec![0x19, 2]
    );

    // Actual_Shed_Level isn't in the row, so its change alone reports
    // nothing, though the pass serves it.
    app.answer(ShedProgress::Compliant, 3);
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        read(&h, PropertyIdentifier::ACTUAL_SHED_LEVEL).await,
        vec![0x19, 3]
    );
    h.no_notification().await;

    // The loads fall short: SHED_NON_COMPLIANT.
    app.answer(ShedProgress::Shedding, 1);
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(3, 2, start_at(5), 30, 10)
    );

    // 15:35, Start_Time plus Shed_Duration: the request is over and its
    // properties go back to their idle values.
    set_clock(&h, 35);
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(0, 0, unspecified(), 0, 0)
    );
    assert_eq!(*app.stops.lock().unwrap(), 1);
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn writes_cancel_and_disable_a_request_over_the_wire() {
    let app = app();
    app.answer(ShedProgress::Compliant, 2);
    let mut h = harness(&app).await;
    let oid = lc1();
    set_clock(&h, 0);
    assert_eq!(
        subscribed(&mut h, oid).await,
        report(0, 0, unspecified(), 0, 0)
    );

    // One WritePropertyMultiple carries a whole request starting now.
    write_multiple(
        &mut h,
        oid,
        vec![
            (PropertyIdentifier::REQUESTED_SHED_LEVEL, vec![0x19, 0x02]),
            (PropertyIdentifier::SHED_DURATION, unsigned(30)),
            (PropertyIdentifier::START_TIME, start_at(0)),
        ],
    )
    .await;
    let mut last = None;
    while let Ok(notification) =
        tokio::time::timeout(Duration::from_millis(100), h.cov_notification()).await
    {
        last = Some(values(&notification, oid));
    }
    assert_eq!(last, Some(report(2, 2, start_at(0), 30, 0)));

    // The unspecified Start_Time cancels it.
    write(&mut h, oid, PropertyIdentifier::START_TIME, unspecified()).await;
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(0, 0, unspecified(), 0, 0)
    );
    assert_eq!(*app.stops.lock().unwrap(), 1);

    // A request again, then Enable FALSE ends it. While disabled, the
    // request writes are all acknowledged, but only the level and the duty
    // window are kept: Shed_Duration and Start_Time keep their idle values.
    write(
        &mut h,
        oid,
        PropertyIdentifier::REQUESTED_SHED_LEVEL,
        vec![0x19, 0x03],
    )
    .await;
    write(&mut h, oid, PropertyIdentifier::SHED_DURATION, unsigned(20)).await;
    write(&mut h, oid, PropertyIdentifier::START_TIME, start_at(1)).await;
    let mut state = None;
    while let Ok(notification) =
        tokio::time::timeout(Duration::from_millis(100), h.cov_notification()).await
    {
        state = Some(values(&notification, oid)[0].clone());
    }
    assert_eq!(state, Some((PV, enumerated(2))));
    write(
        &mut h,
        oid,
        PropertyIdentifier::ENABLE,
        encode(PropertyValue::Boolean(false)),
    )
    .await;
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(0, 0, unspecified(), 0, 0)
    );
    assert_eq!(*app.stops.lock().unwrap(), 2);
    write(
        &mut h,
        oid,
        PropertyIdentifier::REQUESTED_SHED_LEVEL,
        vec![0x19, 0x01],
    )
    .await;
    write(&mut h, oid, PropertyIdentifier::SHED_DURATION, unsigned(20)).await;
    write(&mut h, oid, PropertyIdentifier::DUTY_WINDOW, unsigned(5)).await;
    write(&mut h, oid, PropertyIdentifier::START_TIME, start_at(2)).await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    for (property, value) in [
        (PropertyIdentifier::PRESENT_VALUE, enumerated(0)),
        (PropertyIdentifier::REQUESTED_SHED_LEVEL, vec![0x19, 0x01]),
        (PropertyIdentifier::SHED_DURATION, unsigned(0)),
        (PropertyIdentifier::DUTY_WINDOW, unsigned(5)),
        (PropertyIdentifier::START_TIME, unspecified()),
    ] {
        assert_eq!(read(&h, property).await, value, "{property:?}");
    }
    // Enable TRUE starts nothing; a Shed_Duration and a Start_Time written
    // after it start a request with the level and window held.
    write(
        &mut h,
        oid,
        PropertyIdentifier::ENABLE,
        encode(PropertyValue::Boolean(true)),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        read(&h, PropertyIdentifier::PRESENT_VALUE).await,
        enumerated(0)
    );
    write(&mut h, oid, PropertyIdentifier::SHED_DURATION, unsigned(20)).await;
    write(&mut h, oid, PropertyIdentifier::START_TIME, start_at(0)).await;
    assert_eq!(
        read(&h, PropertyIdentifier::PRESENT_VALUE).await,
        enumerated(2)
    );
    {
        let asked = app.asked.lock().unwrap();
        let request = asked.last().unwrap();
        assert_eq!(request.requested, BACnetShedLevel::Level(1));
        assert_eq!((request.shed_duration, request.duty_window), (20, 5));
    }
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn the_new_properties_take_their_datatypes_over_the_wire() {
    let app = app();
    let mut h = harness(&app).await;
    let oid = lc1();
    // Element 2 of Shed_Levels; its size and Shed_Level_Descriptions are the
    // device's.
    let write_at = |property, index, value| WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: Some(index),
        property_value: value,
        priority: None,
    };
    let mut answers = Vec::new();
    for request in [
        write_at(PropertyIdentifier::SHED_LEVELS, 2, unsigned(5)),
        write_at(PropertyIdentifier::SHED_LEVELS, 0, unsigned(4)),
        write_at(PropertyIdentifier::SHED_LEVELS, 4, unsigned(4)),
        // Level 0 is no shed, which Shed_Levels doesn't hold.
        write_at(PropertyIdentifier::SHED_LEVELS, 1, unsigned(0)),
        write_at(
            PropertyIdentifier::SHED_LEVEL_DESCRIPTIONS,
            1,
            encode(PropertyValue::CharacterString("other".into())),
        ),
    ] {
        let mut body = BytesMut::new();
        request.encode(&mut body).unwrap();
        h.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
            .await;
        answers.push(response(&h).await);
    }
    assert_eq!(
        answers,
        [
            Ok(()),
            Err(ErrorCode::WRITE_ACCESS_DENIED),
            Err(ErrorCode::INVALID_ARRAY_INDEX),
            Err(ErrorCode::VALUE_OUT_OF_RANGE),
            Err(ErrorCode::WRITE_ACCESS_DENIED),
        ]
    );
    let levels = [1, 5, 3].map(unsigned).concat();
    assert_eq!(read(&h, PropertyIdentifier::SHED_LEVELS).await, levels);
    for (property, value, code) in [
        (
            PropertyIdentifier::ENABLE,
            unsigned(1),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            PropertyIdentifier::DUTY_WINDOW,
            encode(PropertyValue::Real(1.0)),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        // A Start_Time with an unspecified hour names no moment.
        (
            PropertyIdentifier::START_TIME,
            encode_date_time(
                clock_at(0).local_date,
                Time {
                    hour: 0xFF,
                    ..clock_at(0).local_time
                },
            ),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
    ] {
        assert_eq!(
            try_write(&mut h, oid, property, value).await,
            Err(code),
            "{property:?}"
        );
    }
    write(&mut h, oid, PropertyIdentifier::DUTY_WINDOW, unsigned(15)).await;
    assert_eq!(
        read(&h, PropertyIdentifier::DUTY_WINDOW).await,
        unsigned(15)
    );
    write(
        &mut h,
        oid,
        PropertyIdentifier::ENABLE,
        encode(PropertyValue::Boolean(false)),
    )
    .await;
    assert_eq!(
        read(&h, PropertyIdentifier::ENABLE).await,
        encode(PropertyValue::Boolean(false))
    );
    h.server.stop().await.unwrap();
}

/// Storage in memory for a Load Control's request, refusing saves while
/// `fail`.
#[derive(Default)]
struct Store {
    saved: StdMutex<Option<LoadControlSnapshot>>,
    fail: std::sync::atomic::AtomicBool,
}

impl LoadControlPersistence for Store {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<LoadControlSnapshot>, Error> {
        Ok(self.saved.lock().unwrap().clone())
    }

    fn save(&self, _: ObjectIdentifier, snapshot: &LoadControlSnapshot) -> Result<(), Error> {
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(Error::Encoding("the disk is full".into()));
        }
        *self.saved.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

/// A harness serving LC-1 kept in `store`, with `app` installed.
async fn kept_harness(app: &Arc<App>, store: &Arc<Store>) -> Harness {
    let mut object = LoadControlObject::with_persistence(1, "LC-1", store.clone()).unwrap();
    object
        .set_shed_levels(vec![(1, "a".into()), (2, "b".into())])
        .unwrap();
    object.set_shedder(app.clone());
    Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(object)).unwrap()
    })
    .await
}

#[tokio::test(start_paused = true)]
async fn a_request_written_over_the_wire_survives_a_restart() {
    // A century: the server starts on the system clock before the harness
    // sets its own, so the request is under way whatever the date.
    const CENTURY: u64 = 100 * 365 * 24 * 60;
    let app = app();
    app.answer(ShedProgress::Compliant, 2);
    let store = Arc::new(Store::default());
    let mut h = kept_harness(&app, &store).await;
    let oid = lc1();
    set_clock(&h, 0);
    for (property, value) in [
        (PropertyIdentifier::REQUESTED_SHED_LEVEL, vec![0x19, 0x02]),
        (PropertyIdentifier::SHED_DURATION, unsigned(CENTURY)),
        (PropertyIdentifier::START_TIME, start_at(0)),
    ] {
        write(&mut h, oid, property, value).await;
    }
    // Each write was saved before it was answered (#2479).
    {
        let saved = store.saved.lock().unwrap().clone().unwrap();
        assert_eq!(saved.requested_shed_level, BACnetShedLevel::Level(2));
        assert_eq!(saved.shed_duration, CENTURY);
        let frame = clock_at(0);
        assert_eq!(saved.start_time, (frame.local_date, frame.local_time));
    }
    assert_eq!(
        read(&h, PropertyIdentifier::PRESENT_VALUE).await,
        enumerated(2)
    );
    h.server.stop().await.unwrap();
    drop(h);
    // The device comes back: the server's start looks at the request again,
    // and the loads are shed for it.
    app.asked.lock().unwrap().clear();
    let h = kept_harness(&app, &store).await;
    assert_eq!(
        read(&h, PropertyIdentifier::PRESENT_VALUE).await,
        enumerated(2)
    );
    assert_eq!(read(&h, PropertyIdentifier::START_TIME).await, start_at(0));
    assert_eq!(
        read(&h, PropertyIdentifier::SHED_DURATION).await,
        unsigned(CENTURY)
    );
    assert!(!app.asked.lock().unwrap().is_empty());
    // A write that can't be saved is refused, and changes nothing.
    let mut h = h;
    store.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        try_write(&mut h, oid, PropertyIdentifier::DUTY_WINDOW, unsigned(5)).await,
        Err(ErrorCode::OPERATIONAL_PROBLEM)
    );
    assert_eq!(read(&h, PropertyIdentifier::DUTY_WINDOW).await, unsigned(0));
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_delete_object_during_a_shed_stops_the_loads() {
    let app = app();
    app.answer(ShedProgress::Compliant, 2);
    let mut h = harness(&app).await;
    let oid = lc1();
    set_clock(&h, 0);
    for (property, value) in [
        (PropertyIdentifier::REQUESTED_SHED_LEVEL, vec![0x19, 0x02]),
        (PropertyIdentifier::SHED_DURATION, unsigned(30)),
        (PropertyIdentifier::START_TIME, start_at(0)),
    ] {
        write(&mut h, oid, property, value).await;
    }
    assert_eq!(
        read(&h, PropertyIdentifier::PRESENT_VALUE).await,
        enumerated(2)
    );
    assert_eq!(*app.stops.lock().unwrap(), 0);
    // The object goes with its request, and the application is told (#2479).
    let mut body = BytesMut::new();
    bacnet_services::object_mgmt::DeleteObjectRequest {
        object_identifier: oid,
    }
    .encode(&mut body);
    h.request(ConfirmedServiceChoice::DELETE_OBJECT, body).await;
    assert_eq!(response(&h).await, Ok(()));
    assert!(h.server.database().read().await.get(&oid).is_none());
    assert_eq!(*app.stops.lock().unwrap(), 1);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_clock_set_back_before_start_time_reports_the_request_pending_again() {
    let app = app();
    app.answer(ShedProgress::Compliant, 2);
    let mut h = harness(&app).await;
    let oid = lc1();
    set_clock(&h, 0);
    assert_eq!(
        subscribed(&mut h, oid).await,
        report(0, 0, unspecified(), 0, 0)
    );
    for (property, value) in [
        (PropertyIdentifier::REQUESTED_SHED_LEVEL, vec![0x19, 0x02]),
        (PropertyIdentifier::SHED_DURATION, unsigned(30)),
        (PropertyIdentifier::START_TIME, start_at(5)),
    ] {
        write(&mut h, oid, property, value).await;
        h.cov_notification().await;
    }
    // Compliant early, and on at 15:05.
    set_clock(&h, 5);
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        read(&h, PropertyIdentifier::PRESENT_VALUE).await,
        enumerated(2)
    );
    h.no_notification().await;
    // The Device clock goes back to 15:02 and the loads wait for Start_Time
    // again: the request is pending once more, with no write.
    app.answer(ShedProgress::Waiting, 0);
    set_clock(&h, 2);
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(1, 2, start_at(5), 30, 0)
    );
    // At 15:05 again the loads, still waiting, don't comply.
    set_clock(&h, 5);
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(3, 2, start_at(5), 30, 0)
    );
    h.server.stop().await.unwrap();
}
