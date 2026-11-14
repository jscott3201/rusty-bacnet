//! A Time_Delay runs from the instant its condition began (#2303): a write
//! between the one-second sweeps seeds a transition due its delay after the
//! write, and the sweep task wakes for it then, not at a whole second.

use super::event_notifications_tests::{
    decode_broadcast_notification, notification_class_0_broadcasting, recording_transport,
};
use super::*;
use crate::server::test_transport::{SendLog, TestTransport};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{EngineeringUnits, EventState};

/// Analog Inputs AI-1, AI-2, ... reading 50, inside their limits, each
/// reporting every transition of OUT_OF_RANGE through a broadcasting
/// Notification Class 0 after its own Time_Delay.
fn database(delays: &[u32]) -> (ObjectDatabase, Vec<ObjectIdentifier>) {
    let mut db = clocked_test_database();
    let mut oids = Vec::new();
    for (instance, time_delay) in (1..).zip(delays) {
        let name = format!("AI-{instance}");
        let mut ai =
            AnalogInputObject::new(instance, &name, EngineeringUnits::DEGREES_CELSIUS).unwrap();
        for (property, value) in [
            (PropertyIdentifier::HIGH_LIMIT, 80.0),
            (PropertyIdentifier::LOW_LIMIT, 20.0),
            (PropertyIdentifier::DEADBAND, 2.0),
        ] {
            ai.write_property(property, None, PropertyValue::Real(value), None)
                .unwrap();
        }
        for (property, unused_bits, bits) in [
            (PropertyIdentifier::LIMIT_ENABLE, 6, 0xC0),
            (PropertyIdentifier::EVENT_ENABLE, 5, 0xE0),
        ] {
            ai.write_property(
                property,
                None,
                PropertyValue::BitString {
                    unused_bits,
                    data: vec![bits],
                },
                None,
            )
            .unwrap();
        }
        ai.write_property(
            PropertyIdentifier::TIME_DELAY,
            None,
            PropertyValue::Unsigned((*time_delay).into()),
            None,
        )
        .unwrap();
        ai.set_present_value(50.0);
        oids.push(ai.object_identifier());
        db.add(Box::new(ai)).unwrap();
    }
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::default().with_name("Dev"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(notification_class_0_broadcasting()))
        .unwrap();
    (db, oids)
}

/// Lower `oid`'s High_Limit below its value through the server's write
/// path, so its condition begins now and its delay is seeded.
async fn begin_condition(server: &BACnetServer<TestTransport>, oid: &ObjectIdentifier) {
    server
        .write_local(
            oid,
            PropertyIdentifier::HIGH_LIMIT,
            None,
            PropertyValue::Real(40.0),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .expect("the local write seeds the delayed transition");
}

/// The instances whose EventNotifications have gone out, sorted: one sweep
/// sends in the database's order.
fn notified(sent: &SendLog) -> Vec<u32> {
    use bacnet_encoding::apdu::decode_apdu;
    use bacnet_encoding::constructed::decode_event_notification;
    use bacnet_encoding::npdu::decode_npdu;

    let mut instances: Vec<u32> = sent
        .npdus()
        .into_iter()
        .filter_map(|frame| {
            let npdu = decode_npdu(frame).ok()?;
            let Apdu::UnconfirmedRequest(request) = decode_apdu(npdu.payload).ok()? else {
                return None;
            };
            (request.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION)
                .then(|| decode_event_notification(&request.service_request).unwrap())
        })
        .map(|notification| notification.event_object_identifier.instance_number())
        .collect();
    instances.sort_unstable();
    instances
}

/// Written 0.9 s after the sweeps began, with Time_Delay 2, the alarm goes
/// out at 2.9 s. The one-second countdown sent it at the 2 s sweep, 1.1 s
/// after the write.
#[tokio::test(start_paused = true)]
async fn a_write_seeded_delay_ends_two_seconds_after_the_write() {
    let (transport, sent) = recording_transport();
    let (db, oids) = database(&[2]);
    let oid = oids[0];
    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .expect("server should start");
    let start = tokio::time::Instant::now();

    tokio::time::sleep_until(start + Duration::from_millis(900)).await;
    // Lowering High_Limit below the value begins the condition now.
    server
        .write_local(
            &oid,
            PropertyIdentifier::HIGH_LIMIT,
            None,
            PropertyValue::Real(40.0),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .expect("the local write seeds the delayed transition");

    tokio::time::sleep_until(start + Duration::from_millis(2_890)).await;
    assert!(
        sent.npdus().is_empty(),
        "the alarm went out before its delay ended"
    );

    tokio::time::sleep_until(start + Duration::from_millis(2_910)).await;
    let notification = decode_broadcast_notification(&sent.npdus());
    assert_eq!(notification.from_state, EventState::NORMAL);
    assert_eq!(notification.to_state, EventState::HIGH_LIMIT);
}

/// Two writes' delays end in turn, each at its own instant, and a
/// Time_Delay of 0 the 1 s sweep noticed, pending with its due instant
/// already reached, delays neither of them: it goes out at the next sweep,
/// the one woken for the first write's delay.
#[tokio::test(start_paused = true)]
async fn each_delay_ends_at_its_own_instant() {
    let (transport, sent) = recording_transport();
    let (db, oids) = database(&[1, 1, 0]);
    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .expect("server should start");
    let start = tokio::time::Instant::now();
    let at = |millis| start + Duration::from_millis(millis);

    // AI-3's condition begins with no write the server sees.
    tokio::time::sleep_until(at(100)).await;
    server
        .database()
        .write()
        .await
        .get_mut(&oids[2])
        .unwrap()
        .write_property(
            PropertyIdentifier::HIGH_LIMIT,
            None,
            PropertyValue::Real(40.0),
            None,
        )
        .unwrap();
    tokio::time::sleep_until(at(200)).await;
    begin_condition(&server, &oids[0]).await; // due at 1.2 s
    tokio::time::sleep_until(at(500)).await;
    begin_condition(&server, &oids[1]).await; // due at 1.5 s

    for (millis, expected) in [
        (1_190, &[][..]),
        (1_210, &[1, 3]),
        (1_490, &[1, 3]),
        (1_510, &[1, 2, 3]),
    ] {
        tokio::time::sleep_until(at(millis)).await;
        assert_eq!(notified(&sent), expected, "at {millis} ms");
    }
}

/// A sweep woken for a due instant runs no sooner than the floor after the
/// last one: delays ending 50 ms apart go out 100 ms apart.
#[tokio::test(start_paused = true)]
async fn a_due_wake_waits_out_the_floor() {
    let (transport, sent) = recording_transport();
    let (db, oids) = database(&[1, 1]);
    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .expect("server should start");
    let start = tokio::time::Instant::now();
    let at = |millis| start + Duration::from_millis(millis);

    tokio::time::sleep_until(at(200)).await;
    begin_condition(&server, &oids[0]).await; // due at 1.2 s
    tokio::time::sleep_until(at(250)).await;
    begin_condition(&server, &oids[1]).await; // due at 1.25 s

    for (millis, expected) in [(1_210, &[1][..]), (1_290, &[1]), (1_310, &[1, 2])] {
        tokio::time::sleep_until(at(millis)).await;
        assert_eq!(notified(&sent), expected, "at {millis} ms");
    }
}

/// An object that counts the sweeps and always reports the same past due
/// instant, as a transition whose commit keeps failing would.
struct SweepCounter {
    oid: ObjectIdentifier,
    sweeps: Arc<std::sync::atomic::AtomicUsize>,
    /// A due instant before the first sweep, close to the clock so it maps
    /// to the host's timeline on every platform.
    past: bacnet_objects::event::Instant,
}

impl BACnetObject for SweepCounter {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }
    fn object_name(&self) -> &str {
        "SWEEP-COUNTER"
    }
    fn read_property(
        &self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ))
    }
    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }
    fn property_metadata(
        &self,
    ) -> std::borrow::Cow<'_, [bacnet_objects::property_metadata::PropertyMetadata]> {
        crate::test_rows::read_only(&[])
    }
    fn event_reporting(&self) -> Option<&dyn bacnet_objects::traits::EventReporting> {
        Some(self)
    }
    fn event_reporting_mut(&mut self) -> Option<&mut dyn bacnet_objects::traits::EventReporting> {
        Some(self)
    }
}

impl bacnet_objects::traits::EventReporting for SweepCounter {
    fn tick_intrinsic_reporting(
        &mut self,
        _now: bacnet_objects::event::Instant,
    ) -> Option<bacnet_objects::event::TransitionOutcome> {
        self.sweeps.fetch_add(1, Ordering::SeqCst);
        None
    }
    fn next_due(&self) -> Option<bacnet_objects::event::Instant> {
        Some(self.past)
    }
}

/// A due instant already past adds no sweep: the sweep that saw it has
/// run, so the task keeps its one-second cadence rather than waking again
/// at every floor.
#[tokio::test(start_paused = true)]
async fn a_past_due_instant_adds_no_sweep() {
    let (transport, _sent) = recording_transport();
    let (mut db, _) = database(&[]);
    let sweeps = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    db.add(Box::new(SweepCounter {
        oid: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 99).unwrap(),
        sweeps: Arc::clone(&sweeps),
        past: crate::runtime_clock::core_now()
            .saturating_sub(bacnet_objects::event::Duration::from_secs(2)),
    }))
    .unwrap();
    let _server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .expect("server should start");
    let start = tokio::time::Instant::now();

    tokio::time::sleep_until(start + Duration::from_millis(3_500)).await;
    // The sweeps at 0, 1, 2 and 3 s.
    assert_eq!(sweeps.load(Ordering::SeqCst), 4);
}

/// The floor never delays the periodic sweep: after a sweep woken at 1.95 s
/// for one delay, a second delay ending at 1.98 s goes out with the
/// periodic sweep at 2 s, not at 2.05 s.
#[tokio::test(start_paused = true)]
async fn the_floor_never_delays_the_periodic_sweep() {
    let (transport, sent) = recording_transport();
    let (db, oids) = database(&[1, 1]);
    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .expect("server should start");
    let start = tokio::time::Instant::now();
    let at = |millis| start + Duration::from_millis(millis);

    tokio::time::sleep_until(at(950)).await;
    begin_condition(&server, &oids[0]).await; // due at 1.95 s
    tokio::time::sleep_until(at(980)).await;
    begin_condition(&server, &oids[1]).await; // due at 1.98 s

    for (millis, expected) in [
        (1_940, &[][..]),
        (1_960, &[1]),
        (1_990, &[1]),
        (2_010, &[1, 2]),
    ] {
        tokio::time::sleep_until(at(millis)).await;
        assert_eq!(notified(&sent), expected, "at {millis} ms");
    }
}

/// A database whose AI-1 alarms through Notification Class 1 to Device 9,
/// which the server holds no binding for and which never answers, and whose
/// AI-2 alarms through the broadcasting class 0; both after 1 s.
fn database_with_a_silent_device() -> (ObjectDatabase, Vec<ObjectIdentifier>) {
    let (mut db, oids) = database(&[1, 1]);
    let mut nc1 = bacnet_objects::notification_class::NotificationClass::new(1, "NC-1").unwrap();
    let device_9 = ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap();
    let recipient = bacnet_types::constructed::BACnetRecipient::Device(device_9);
    let to_device = super::event_recipient_routing_tests::destination_for(recipient, false);
    nc1.add_destination(to_device).unwrap();
    db.add(Box::new(nc1)).unwrap();
    let class = PropertyValue::Unsigned(1);
    let ai1 = db.get_mut(&oids[0]).unwrap();
    ai1.write_property(PropertyIdentifier::NOTIFICATION_CLASS, None, class, None)
        .unwrap();
    (db, oids)
}

/// A sweep pass whose notification waits in a lane (here for a Device
/// recipient's I-Am, #2537) holds up no later pass: AI-2's alarm, due at
/// 2.5 s, goes out then, while AI-1's lane still waits for Device 9.
#[tokio::test(start_paused = true)]
async fn a_pass_waiting_on_a_lane_holds_up_no_later_sweep() {
    let (transport, sent) = recording_transport();
    let (db, oids) = database_with_a_silent_device();
    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .expect("server should start");
    let start = tokio::time::Instant::now();
    let at = |millis| start + Duration::from_millis(millis);
    tokio::time::sleep_until(at(100)).await;
    begin_condition(&server, &oids[0]).await; // due at 1.1 s, to Device 9
    tokio::time::sleep_until(at(1_500)).await;
    begin_condition(&server, &oids[1]).await; // due at 2.5 s, broadcast
    for (millis, expected) in [(2_490, &[][..]), (2_610, &[2])] {
        tokio::time::sleep_until(at(millis)).await;
        assert_eq!(notified(&sent), expected, "at {millis} ms");
    }
}

/// The COV subscriber of the sweep fanout tests.
const SUBSCRIBER: [u8; 6] = [10, 0, 0, 5, 0xBA, 0xC0];

/// A transport that records unicasts too; with `hold`, each event
/// notification's send waits for a permit of the semaphore returned.
fn sweep_transport(hold: bool) -> (TestTransport, SendLog, Arc<Semaphore>) {
    let permits = Arc::new(Semaphore::new(0));
    let gate = Arc::clone(&permits);
    let transport = TestTransport::builder()
        .local_mac(&crate::server::test_transport::BIP_LOCAL_MAC)
        .on_send(move |frame| {
            let gate = Arc::clone(&gate);
            async move {
                let event = matches!(frame.apdu(), Apdu::UnconfirmedRequest(request)
                    if request.service_choice
                        == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION);
                if hold && event {
                    gate.acquire().await.unwrap().forget();
                }
                Ok(())
            }
        })
        .build();
    let sent = transport.sent();
    (transport, sent, permits)
}

/// [`SUBSCRIBER`] subscribes to the COV of each of `oids`.
async fn subscribe(server: &BACnetServer<TestTransport>, oids: &[ObjectIdentifier]) {
    for (process, oid) in (1..).zip(oids) {
        let subscription = bacnet_device_core::cov::CovSubscription {
            subscriber_mac: MacAddr::from_slice(&SUBSCRIBER),
            subscriber_network: None,
            subscriber_process_identifier: process,
            monitored_object_identifier: *oid,
            issue_confirmed_notifications: false,
            expires_at: None,
            last_notified_observation: None,
            monitored_property: None,
            monitored_property_array_index: None,
            cov_increment: None,
            notification_kind: CovNotificationKind::Single,
            timestamped: false,
        };
        server
            .cov_table
            .write()
            .await
            .subscribe(subscription)
            .unwrap();
    }
}

/// The COV reports among `frames`, in order: the object's instance, and
/// whether its Status_Flags say IN_ALARM.
fn reports(frames: &[crate::server::test_transport::SentFrame]) -> Vec<(u32, bool)> {
    frames
        .iter()
        .filter_map(|frame| match frame.apdu() {
            Apdu::UnconfirmedRequest(request)
                if request.service_choice
                    == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION =>
            {
                let n = COVNotificationRequest::decode(&request.service_request).unwrap();
                let in_alarm = n
                    .list_of_values
                    .iter()
                    .find(|value| value.property_identifier == PropertyIdentifier::STATUS_FLAGS)
                    .and_then(|flags| flags.value.last().copied())
                    .is_some_and(|bits| bits & 0x80 != 0);
                Some((n.monitored_object_identifier.instance_number(), in_alarm))
            }
            _ => None,
        })
        .collect()
}

/// The COV fanouts of sweep passes go in the order the passes ran, each
/// behind its own pass's event notifications: AI-2's alarm is out at 2.5 s,
/// but its COV report waits behind AI-1's, which waits for Device 9's lane.
#[tokio::test(start_paused = true)]
async fn sweep_cov_fanouts_keep_sweep_order_behind_a_waiting_lane() {
    let (transport, sent, _) = sweep_transport(false);
    let (db, oids) = database_with_a_silent_device();
    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .expect("server should start");
    subscribe(&server, &oids).await;
    let start = tokio::time::Instant::now();
    let at = |millis| start + Duration::from_millis(millis);
    tokio::time::sleep_until(at(100)).await;
    begin_condition(&server, &oids[0]).await;
    tokio::time::sleep_until(at(1_500)).await;
    begin_condition(&server, &oids[1]).await;
    let before = sent.len();
    let reported = || -> Vec<u32> {
        let frames = sent.frames();
        reports(&frames[before..])
            .into_iter()
            .map(|(instance, _)| instance)
            .collect()
    };
    tokio::time::sleep_until(at(2_610)).await;
    assert_eq!(notified(&sent), [2], "AI-2's alarm is out");
    assert_eq!(reported(), [], "its COV report waits behind AI-1's");
    tokio::time::sleep_until(at(4_500)).await;
    assert_eq!(reported(), [1, 2]);
}

/// Each pass's COV report waits for its notifications at most the APDU
/// timeout from that pass's commit, not from the end of the wait before it:
/// passes committing at 1.1, 2.5 and 3.9 s behind notifications that never
/// go out report at 4.1, 5.5 and 6.9 s, each wait counted.
#[tokio::test(start_paused = true)]
async fn each_pass_reports_within_the_cap_of_its_own_commit() {
    let (transport, sent, _held) = sweep_transport(true);
    let (db, oids) = database(&[1, 1, 1]);
    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .expect("server should start");
    subscribe(&server, &oids).await;
    let start = tokio::time::Instant::now();
    let at = |millis| start + Duration::from_millis(millis);
    let begins = [100, 1_500, 2_900];
    let mut alarmed = Vec::new();
    let mut seen = 0;
    for millis in (0..12_000).step_by(50) {
        tokio::time::sleep_until(at(millis)).await;
        if let Some(object) = begins.iter().position(|begin| *begin == millis) {
            begin_condition(&server, &oids[object]).await;
        }
        let frames = sent.frames();
        for (instance, in_alarm) in reports(&frames[seen..]) {
            if in_alarm {
                alarmed.push((instance, millis));
            }
        }
        seen = frames.len();
    }
    assert_eq!(alarmed.len(), 3, "{alarmed:?}");
    for ((instance, reported), begin) in alarmed.into_iter().zip(begins) {
        let committed = begin + 1_000;
        let within = committed + 3_000..=committed + 3_100;
        assert!(within.contains(&reported), "AI-{instance} at {reported} ms");
    }
    let counters = server.event_notification_counters();
    assert_eq!(counters.cov_wait_expired, 3);
}

/// An object two passes behind a held lane fan out for is reported once,
/// with the later pass, after that pass's notification: no report gives the
/// later state while its notification is still held, and the alarm the
/// earlier pass's notification told of never shows in a COV report.
#[tokio::test(start_paused = true)]
async fn passes_behind_a_held_lane_merge_their_reports_of_one_object() {
    let (transport, sent, held) = sweep_transport(true);
    let (db, oids) = database(&[1]);
    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .expect("server should start");
    let start = tokio::time::Instant::now();
    let at = |millis| start + Duration::from_millis(millis);
    tokio::time::sleep_until(at(100)).await;
    begin_condition(&server, &oids[0]).await; // in alarm at 1.1 s
    // Subscribed after the write's own fanout, so the sweep's is the first.
    subscribe(&server, &oids).await;
    tokio::time::sleep_until(at(1_200)).await;
    // The condition ends unseen by the write path: a sweep sees it, and
    // AI-1 is back to NORMAL a second later.
    let high_limit = PropertyValue::Real(80.0);
    let mut db = server.database().write().await;
    let ai1 = db.get_mut(&oids[0]).unwrap();
    ai1.write_property(PropertyIdentifier::HIGH_LIMIT, None, high_limit, None)
        .unwrap();
    drop(db);
    let before = sent.len();
    let reported = || reports(&sent.frames()[before..]);
    tokio::time::sleep_until(at(3_500)).await;
    let db = server.database().read().await;
    let state = db
        .get(&oids[0])
        .unwrap()
        .read_property(PropertyIdentifier::EVENT_STATE, None);
    drop(db);
    assert!(
        matches!(state, Ok(PropertyValue::Enumerated(0))),
        "both passes made"
    );
    assert_eq!(reported(), []);
    held.add_permits(1); // the alarm's notification
    tokio::time::sleep_until(at(3_800)).await;
    assert_eq!(reported(), [], "the return's notification is still held");
    held.add_permits(1);
    tokio::time::sleep_until(at(4_000)).await;
    assert_eq!(reported(), [(1, false)], "one report, merged");
}

/// Past 64 passes waiting behind a held lane, a pass fans out at its
/// commit, its wait counted since its own notification is held; `stop()`
/// with the queue full returns, and nothing goes out after it.
#[tokio::test(start_paused = true)]
async fn past_the_bound_a_pass_fans_out_at_once_and_counts() {
    let (transport, sent, _held) = sweep_transport(true);
    let (mut db, oids) = database(&[1; 66]);
    // AI-2 to AI-65 commit with no notification to wait for.
    let off = PropertyValue::BitString {
        unused_bits: 5,
        data: vec![0],
    };
    for oid in &oids[1..65] {
        let object = db.get_mut(oid).unwrap();
        object
            .write_property(PropertyIdentifier::EVENT_ENABLE, None, off.clone(), None)
            .unwrap();
    }
    let config = ServerConfig {
        cov_retry_timeout_ms: 120_000,
        ..ServerConfig::default()
    };
    let mut server = BACnetServer::start(config, db, transport)
        .await
        .expect("server should start");
    subscribe(&server, &[oids[0], oids[1], oids[64], oids[65]]).await;
    let start = tokio::time::Instant::now();
    let at = |millis| start + Duration::from_millis(millis);
    for (k, oid) in (0..).zip(&oids) {
        tokio::time::sleep_until(at(100 + 200 * k)).await;
        begin_condition(&server, oid).await;
    }
    // AI-66's pass commits at 14.1 s, with AI-1's waiting and 64 behind it.
    let before = sent.len();
    tokio::time::sleep_until(at(14_300)).await;
    assert_eq!(reports(&sent.frames()[before..]), [(66, true)]);
    assert_eq!(server.event_notification_counters().cov_wait_expired, 1);
    tokio::time::timeout(Duration::from_secs(30), server.stop())
        .await
        .expect("stop returns")
        .unwrap();
    let stopped = sent.len();
    tokio::time::sleep(Duration::from_secs(300)).await;
    assert_eq!(sent.len(), stopped, "nothing after stop() returns");
}
