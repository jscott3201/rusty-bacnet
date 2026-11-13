//! The lanes on the wire, through a running server (#2537): COV after its
//! own held event notification and the capped wait, an acknowledgment
//! behind its transition, `stop()` with a held lane, a Device recipient's
//! order, the overflow counters, DCC, a write with no transition, and
//! recipient windows judged at commit.

use super::*;
use crate::server::DccState;
use crate::server::device_bindings::DeviceBinding;
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_services::alarm_event::AcknowledgeAlarmRequest;
use bacnet_types::primitives::{Date, Time};

/// A Device clock the test sets.
struct SetClock(StdMutex<ClockFrame>);

impl ClockReader for SetClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(*self.0.lock().unwrap())
    }
}

fn at_hour(hour: u8) -> ClockFrame {
    ClockFrame {
        local_date: Date {
            year: 126,
            month: 9,
            day: 29,
            day_of_week: 2,
        },
        local_time: Time {
            hour,
            minute: 30,
            second: 0,
            hundredths: 0,
        },
        utc_offset: 0,
        daylight_savings_status: false,
    }
}

/// Notification Class 0 sending to `destinations` alone.
pub(super) fn class_0(db: &mut ObjectDatabase, destinations: Vec<BACnetDestination>) {
    let mut class = NotificationClass::new(0, "NC-0").unwrap();
    for destination in destinations {
        class.add_destination(destination).unwrap();
    }
    db.remove(&class.object_identifier()).unwrap();
    db.add(Box::new(class)).unwrap();
}

fn to_other(other: BACnetRecipient) -> BACnetDestination {
    BACnetDestination {
        recipient: other,
        ..local_broadcast_destination()
    }
}

fn unicast(to: u32) -> Sent {
    Sent::Event {
        broadcast: false,
        object: ai1(),
        to,
        ack: false,
    }
}

/// A SubscribeCOV of AI-1 for [`SUBSCRIBER`], unconfirmed.
pub(super) async fn subscribe_cov(rig: &Rig) {
    let mut table = rig.server.cov_table.write().await;
    table
        .subscribe(CovSubscription {
            subscriber_mac: MacAddr::from_slice(&SUBSCRIBER),
            subscriber_network: None,
            subscriber_process_identifier: 1,
            monitored_object_identifier: ai1(),
            issue_confirmed_notifications: false,
            expires_at: None,
            last_notified_observation: None,
            monitored_property: None,
            monitored_property_array_index: None,
            cov_increment: None,
            notification_kind: CovNotificationKind::Single,
            timestamped: false,
        })
        .unwrap();
}

/// A change's COV notification waits for its own event notification while
/// that is held (#889).
#[tokio::test(start_paused = true)]
async fn cov_follows_its_own_held_event_notification() {
    let rig = Rig::start(ai_objects).await;
    subscribe_cov(&rig).await;
    rig.hold(true, FAULT);
    rig.reliability(Reliability::NO_SENSOR).await;
    rig.holding().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(rig.probe.wire(), [], "the COV waits");
    rig.release();
    assert_eq!(rig.wait_wire(2).await, [ev(ai1(), FAULT), Sent::Cov(ai1())]);
    let counters = rig.server.event_notification_counters();
    assert_eq!(counters.cov_wait_expired, 0);
}

/// The wait is capped at the APDU timeout: past it the COV goes, and the
/// wait is counted.
#[tokio::test(start_paused = true)]
async fn the_cov_wait_expires_at_the_apdu_timeout_and_counts() {
    let config = ServerConfig {
        cov_retry_timeout_ms: 300,
        ..ServerConfig::default()
    };
    let rig = Rig::start_with(config, ai_objects).await;
    subscribe_cov(&rig).await;
    rig.hold(true, FAULT);
    rig.reliability(Reliability::NO_SENSOR).await;
    rig.holding().await;
    assert_eq!(rig.wait_wire(1).await, [Sent::Cov(ai1())], "past the cap");
    let counters = rig.server.event_notification_counters();
    assert_eq!(counters.cov_wait_expired, 1);
    rig.release();
    assert_eq!(rig.wait_wire(2).await, [Sent::Cov(ai1()), ev(ai1(), FAULT)]);
}

/// An acknowledgment joins its recipients' lanes once its SimpleACK is
/// issued, so it never overtakes the transition it acknowledges.
#[tokio::test(start_paused = true)]
async fn an_acknowledgment_never_overtakes_its_held_transition() {
    let rig = Rig::start(|db| {
        ai_objects(db);
        let mut class = notification_class_0_broadcasting();
        class.ack_required = bacnet_types::bitstring::EventTransitionBits::all();
        db.remove(&class.object_identifier()).unwrap();
        db.add(Box::new(class)).unwrap();
    })
    .await;
    rig.hold(true, FAULT);
    rig.reliability(Reliability::NO_SENSOR).await;
    rig.holding().await;
    let timestamp = {
        let db = rig.server.database().read().await;
        let stamp = db
            .get(&ai1())
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, Some(2))
            .unwrap();
        let PropertyValue::ApplicationData(bytes) = stamp else {
            panic!("{stamp:?}");
        };
        decode_timestamp_choice(&bytes, 0).unwrap().0
    };
    let mut body = BytesMut::new();
    AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 71,
        event_object_identifier: ai1(),
        event_state_acknowledged: EventState::FAULT,
        timestamp,
        acknowledgment_source: "operator".into(),
        time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(77),
    }
    .encode(&mut body)
    .unwrap();
    let id = rig
        .request(ConfirmedServiceChoice::ACKNOWLEDGE_ALARM, body)
        .await;
    rig.acked(id).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(rig.probe.wire(), [], "the acknowledgment waits");
    rig.release();
    let acked = Sent::Event {
        broadcast: true,
        object: ai1(),
        to: FAULT,
        ack: true,
    };
    assert_eq!(rig.wait_wire(2).await, [ev(ai1(), FAULT), acked]);
}

/// `stop()` returns with a lane held, and nothing more goes out after it.
#[tokio::test(start_paused = true)]
async fn stop_with_a_held_lane_returns_and_sends_nothing_more() {
    let mut rig = Rig::start(ai_objects).await;
    rig.hold(true, FAULT);
    rig.reliability(Reliability::NO_SENSOR).await;
    rig.holding().await;
    rig.reliability(Reliability::NO_FAULT_DETECTED).await;
    let stopped = tokio::time::timeout(Duration::from_secs(30), rig.server.stop());
    stopped.await.expect("stop returns").unwrap();
    rig.release();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(rig.probe.wire(), [], "nothing sent after stop");
}

/// A Device recipient's lane keeps its order across two senders, as an
/// Address recipient's does.
#[tokio::test(start_paused = true)]
async fn a_device_recipient_keeps_commit_order_across_two_senders() {
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 77).unwrap();
    let bindings = vec![DeviceBinding::local(device, OTHER).unwrap()];
    let rig = Rig::start_bound(ServerConfig::default(), bindings, |db| {
        ai_objects(db);
        class_0(db, vec![to_other(BACnetRecipient::Device(device))]);
    })
    .await;
    rig.hold(false, FAULT);
    rig.reliability(Reliability::NO_SENSOR).await;
    rig.holding().await;
    rig.reliability(Reliability::NO_FAULT_DETECTED).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(rig.probe.wire(), []);
    rig.release();
    assert_eq!(rig.wait_wire(2).await, [unicast(FAULT), unicast(NORMAL)]);
}

/// A stalled destination's lane fills at 64: the next transition to it is
/// dropped and counted, and the 64 that waited still go, in order.
#[tokio::test(start_paused = true)]
async fn a_full_lane_drops_the_newest_and_counts_it() {
    let config = ServerConfig {
        cov_retry_timeout_ms: 20,
        ..ServerConfig::default()
    };
    let rig = Rig::start_with(config, ai_objects).await;
    let (spy, _guard) = LaneLockSpy::install(&rig.server.notification_transactions);
    rig.hold(true, FAULT);
    let flips = MAX_LANE_ENTRIES + 2;
    for flip in 0..flips {
        let reliability = if flip % 2 == 0 {
            Reliability::NO_SENSOR
        } else {
            Reliability::NO_FAULT_DETECTED
        };
        rig.reliability(reliability).await;
        // Each request waits out its capped COV wait in its admission slot,
        // so let it end before the next: one peer has 16.
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    let counters = rig.server.event_notification_counters();
    assert_eq!(counters.lane_full, 1, "one past the 64 waiting");
    assert_eq!((counters.lanes_entries_full, counters.lanes_full), (0, 0));
    assert!(counters.cov_wait_expired >= 1);
    // The drop's warning went out after the lanes' lock was let go.
    spy.assert_logged_unlocked(1);
    rig.release();
    let wire = rig.wait_wire(flips - 1).await;
    assert_eq!(wire.len(), flips - 1, "the held one and the 64 behind it");
    let alternating = wire
        .iter()
        .enumerate()
        .all(|(at, sent)| *sent == ev(ai1(), if at % 2 == 0 { FAULT } else { NORMAL }));
    assert!(alternating, "in commit order: {wire:?}");
}

/// What DCC would refuse is dropped from the lane, uncounted.
#[tokio::test(start_paused = true)]
async fn dcc_drops_what_waits_in_a_lane() {
    let rig = Rig::start(ai_objects).await;
    rig.hold(true, FAULT);
    rig.reliability(Reliability::NO_SENSOR).await;
    rig.holding().await;
    rig.reliability(Reliability::NO_FAULT_DETECTED).await;
    rig.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    rig.release();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(rig.probe.wire(), [ev(ai1(), FAULT)], "the clear is dropped");
    assert_eq!(
        rig.server.event_notification_counters(),
        EventNotificationCounters::default()
    );
}

/// A write that commits no transition pushes nothing and opens no lane.
#[tokio::test(start_paused = true)]
async fn a_write_with_no_transition_pushes_nothing() {
    let rig = Rig::start(ai_objects).await;
    let id = rig.write(ai1(), PV, PropertyValue::Real(21.0)).await;
    rig.acked(id).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(lanes(&rig.server.notification_transactions).open(), 0);
    assert_eq!(rig.probe.wire(), []);
}

/// A recipient's day and time window is judged when the transition
/// commits: one committed inside it goes out after the window has closed,
/// and one committed outside it doesn't go at all.
#[tokio::test(start_paused = true)]
async fn recipient_windows_are_judged_at_commit() {
    let clock = Arc::new(SetClock(StdMutex::new(at_hour(10))));
    let rig = Rig::start(|db| {
        ai_objects(db);
        let mut window = local_broadcast_destination();
        window.from_time = Time {
            hour: 10,
            minute: 0,
            second: 0,
            hundredths: 0,
        };
        window.to_time = Time {
            hour: 11,
            minute: 0,
            second: 0,
            hundredths: 0,
        };
        class_0(db, vec![window]);
    })
    .await;
    // Start installs the system clock; serve the set one.
    let reader: Arc<dyn ClockReader> = clock.clone();
    let database = rig.server.database();
    database.write().await.set_clock_reader(Some(reader));
    rig.hold(true, FAULT);
    rig.reliability(Reliability::NO_SENSOR).await;
    rig.holding().await;
    rig.reliability(Reliability::NO_FAULT_DETECTED).await;
    *clock.0.lock().unwrap() = at_hour(12);
    rig.reliability(Reliability::NO_SENSOR).await;
    rig.release();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(rig.probe.wire(), [ev(ai1(), FAULT), ev(ai1(), NORMAL)]);
}
