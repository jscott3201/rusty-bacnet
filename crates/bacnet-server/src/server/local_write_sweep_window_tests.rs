//! A local write and the intrinsic sweep (#2530).
//!
//! `set_reliability_local` used to commit under one database guard and run
//! its event pass under the next, so for a moment the object held the new
//! Reliability with the old Event_State. The sweep could commit the fault's
//! transition in that gap, and the COV fanout it then owed, delayed behind
//! its EventNotification, could run inside a later clear's gap, so the
//! subscriber heard IN_ALARM without FAULT. The write and its transition now
//! commit under one guard; these tests force those orders on paused time
//! with the database lock, which Tokio grants in turn, and a held
//! EventNotification.

use super::event_notifications_tests::notification_class_0_broadcasting;
use super::*;
use crate::server::test_transport::{BIP_LOCAL_MAC, SendLog, SentFrame, TestTransport};
use bacnet_encoding::constructed::decode_event_notification;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_types::enums::{EngineeringUnits, EventState, Reliability};
use std::sync::atomic::AtomicBool;
use tokio::sync::Semaphore;

/// A ReadProperty queued behind a write reads both of its steps or neither.
#[path = "write_event_gap_tests.rs"]
mod reads;

const IN_ALARM: u8 = 0x80;
const FAULT: u8 = 0x40;
const SUBSCRIBER: [u8; 6] = [10, 0, 0, 5, 0xBA, 0xC0];

fn ai() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

/// Holds the next EventNotification broadcast once armed, until released or
/// for 100 ms.
struct EventHold {
    armed: AtomicBool,
    held: Semaphore,
    release: Semaphore,
}

impl EventHold {
    fn new() -> Self {
        Self {
            armed: AtomicBool::new(false),
            held: Semaphore::new(0),
            release: Semaphore::new(0),
        }
    }

    async fn on_send(&self, frame: SentFrame) {
        if is_event_notification(&frame) && self.armed.swap(false, Ordering::SeqCst) {
            self.held.add_permits(1);
            // Bounded, so a write that sends this notification itself isn't
            // held for good.
            let release = self.release.acquire();
            if let Ok(permit) = tokio::time::timeout(Duration::from_millis(100), release).await {
                permit.unwrap().forget();
            }
        }
    }
}

fn is_event_notification(frame: &SentFrame) -> bool {
    frame.broadcast
        && matches!(
            frame.apdu(),
            Apdu::UnconfirmedRequest(request)
                if request.service_choice
                    == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION
        )
}

/// A Device, Notification Class 0 broadcasting every transition, and AI-1,
/// which reports all three transitions through it. With `alarm`, AI-1 reads
/// 50 between limits of 20 and 80, enabled, with a deadband of 2, and goes
/// into alarm after a Time_Delay of 1 s but back to normal at once.
fn database(alarm: bool) -> ObjectDatabase {
    let mut db = clocked_test_database();
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let mut write = |property, value| ai.write_property(property, None, value, None).unwrap();
    let bits = |unused_bits, bits| PropertyValue::BitString {
        unused_bits,
        data: vec![bits],
    };
    write(PropertyIdentifier::EVENT_ENABLE, bits(5, 0xE0));
    if alarm {
        write(PropertyIdentifier::HIGH_LIMIT, PropertyValue::Real(80.0));
        write(PropertyIdentifier::LOW_LIMIT, PropertyValue::Real(20.0));
        write(PropertyIdentifier::DEADBAND, PropertyValue::Real(2.0));
        write(PropertyIdentifier::LIMIT_ENABLE, bits(6, 0xC0));
        write(PropertyIdentifier::TIME_DELAY, PropertyValue::Unsigned(1));
        write(
            PropertyIdentifier::TIME_DELAY_NORMAL,
            PropertyValue::Unsigned(0),
        );
        ai.set_present_value(50.0);
    }
    db.add(Box::new(ai)).unwrap();
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
    db
}

/// The server on [`database`] over a transport that holds an armed
/// EventNotification, with one unconfirmed whole-object subscriber on AI-1,
/// and the instant the sweeps count from: they run at 0, 1, 2 ... s.
async fn start(
    alarm: bool,
    hold: &Arc<EventHold>,
) -> (BACnetServer<TestTransport>, SendLog, tokio::time::Instant) {
    let on_send = Arc::clone(hold);
    let transport = TestTransport::builder()
        .local_mac(&BIP_LOCAL_MAC)
        .on_send(move |frame| {
            let hold = Arc::clone(&on_send);
            async move {
                hold.on_send(frame).await;
                Ok(())
            }
        })
        .build();
    let sent = transport.sent();
    let server = BACnetServer::start(ServerConfig::default(), database(alarm), transport)
        .await
        .expect("server should start");
    let start = tokio::time::Instant::now();
    server
        .cov_table
        .write()
        .await
        .subscribe(CovSubscription {
            subscriber_mac: MacAddr::from_slice(&SUBSCRIBER),
            subscriber_network: None,
            subscriber_process_identifier: 1,
            monitored_object_identifier: ai(),
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
    (server, sent, start)
}

/// Present_Value and Status_Flags of each COV notification sent to the
/// subscriber, in order.
fn cov_reports(sent: &SendLog) -> Vec<(f32, u8)> {
    sent.unicasts()
        .into_iter()
        .filter(|frame| frame.mac.as_slice() == SUBSCRIBER)
        .filter_map(|frame| match frame.apdu() {
            Apdu::UnconfirmedRequest(request)
                if request.service_choice
                    == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION =>
            {
                Some(COVNotificationRequest::decode(&request.service_request).unwrap())
            }
            _ => None,
        })
        .map(|notification| {
            let value = |property| {
                notification
                    .list_of_values
                    .iter()
                    .find(|value| value.property_identifier == property)
                    .expect("reported")
                    .value
                    .clone()
            };
            let pv = value(PropertyIdentifier::PRESENT_VALUE);
            let flags = value(PropertyIdentifier::STATUS_FLAGS);
            assert_eq!(pv[0], 0x44, "a REAL Present_Value");
            assert_eq!(&flags[..2], &[0x82, 0x04], "four-bit Status_Flags");
            (f32::from_be_bytes(pv[1..5].try_into().unwrap()), flags[2])
        })
        .collect()
}

fn cov_flags(sent: &SendLog) -> Vec<u8> {
    cov_reports(sent)
        .into_iter()
        .map(|(_, flags)| flags)
        .collect()
}

/// The to-states of the EventNotifications broadcast, in order.
fn event_states(sent: &SendLog) -> Vec<EventState> {
    sent.broadcasts()
        .iter()
        .filter(|frame| is_event_notification(frame))
        .map(|frame| match frame.apdu() {
            Apdu::UnconfirmedRequest(request) => {
                decode_event_notification(&request.service_request)
                    .unwrap()
                    .to_state
            }
            _ => unreachable!("an EventNotification"),
        })
        .collect()
}

/// `step`, failing rather than hanging should the forced order not hold.
async fn within<F: std::future::Future>(step: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(5), step)
        .await
        .expect("the step finishes")
}

async fn event_state(server: &BACnetServer<TestTransport>) -> PropertyValue {
    server
        .database()
        .read()
        .await
        .get(&ai())
        .unwrap()
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap()
}

/// The order that failed in `test_reliability_local`. The fault is written
/// while the test holds the database, and the 1 s sweep queues behind it.
/// With the write's two steps apart, the sweep ran between them, committed
/// the FAULT transition and held its COV fanout behind its EventNotification;
/// released, that fanout ran inside the clear's gap and reported IN_ALARM
/// alone. Now the sweep finds the transition committed, and the hold catches
/// the write's own EventNotification for its 100 ms bound. The subscriber
/// hears FAULT with IN_ALARM, then NORMAL, and nothing between.
#[tokio::test(start_paused = true)]
async fn a_sweep_fanout_owed_from_the_fault_reports_no_half_cleared_flags() {
    let oid = ai();
    let hold = Arc::new(EventHold::new());
    let (server, sent, start) = start(false, &hold).await;

    tokio::time::sleep_until(start + Duration::from_millis(500)).await;
    let held = server.database().read().await;
    hold.armed.store(true, Ordering::SeqCst);
    let (fault, ()) = within(async {
        tokio::join!(
            server.set_reliability_local(&oid, Reliability::NO_SENSOR),
            async move {
                tokio::time::sleep_until(start + Duration::from_millis(1_001)).await;
                drop(held);
            }
        )
    })
    .await;
    fault.expect("the fault is taken");
    within(async { hold.held.acquire().await.unwrap().forget() }).await;
    assert_eq!(
        event_state(&server).await,
        PropertyValue::Enumerated(EventState::FAULT.to_raw())
    );
    assert_eq!(cov_flags(&sent), [IN_ALARM | FAULT], "the fault's report");

    // The clear, again behind the test's guard; a held send would go out
    // now and its fanout queue behind the clear.
    let held = server.database().read().await;
    let release = Arc::clone(&hold);
    let (clear, ()) = within(async {
        tokio::join!(
            server.set_reliability_local(&oid, Reliability::NO_FAULT_DETECTED),
            async move {
                tokio::time::sleep(Duration::from_millis(1)).await;
                release.release.add_permits(1);
                tokio::time::sleep(Duration::from_millis(1)).await;
                drop(held);
            }
        )
    })
    .await;
    clear.expect("the clear is taken");
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(
        event_state(&server).await,
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
    assert_eq!(
        cov_flags(&sent),
        [IN_ALARM | FAULT, 0],
        "Status_Flags reported in order: FAULT with IN_ALARM, then none"
    );
}

/// The second half of that order, with the gap closed: the sweep fires a
/// Time_Delay alarm, and its COV fanout, held behind its EventNotification,
/// runs only after the application has put the value back in range, a
/// write that returns AI-1 to NORMAL at once. Released `in_the_write`, the
/// fanout waits for the database behind that write's commit, where the gap
/// was; otherwise it runs once the write has made its own report.
///
/// Either way the fanout measures the object as it then is, against the
/// newest report decided, so the subscriber hears the value back in range
/// with no flags once, from whichever fanout looks first, and never the
/// value back in range with IN_ALARM. The alarm itself reaches it only as
/// the EventNotification: its COV moment had passed when the fanout ran.
async fn a_delayed_sweep_fanout(in_the_write: bool) {
    let oid = ai();
    let hold = Arc::new(EventHold::new());
    let (server, sent, start) = start(true, &hold).await;

    tokio::time::sleep_until(start + Duration::from_millis(200)).await;
    server
        .set_present_value_local(&oid, PropertyValue::Real(90.0))
        .await
        .unwrap();
    assert_eq!(cov_reports(&sent), [(90.0, 0)], "the value's report");
    // The alarm is due at 1.2 s; the sweep woken then fires it.
    hold.armed.store(true, Ordering::SeqCst);
    within(async { hold.held.acquire().await.unwrap().forget() }).await;
    assert_eq!(
        event_state(&server).await,
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
    );

    let back = server.set_present_value_local(&oid, PropertyValue::Real(50.0));
    if in_the_write {
        let held = server.database().read().await;
        let release = Arc::clone(&hold);
        let (back, ()) = within(async {
            tokio::join!(back, async move {
                tokio::time::sleep(Duration::from_millis(1)).await;
                release.release.add_permits(1);
                tokio::time::sleep(Duration::from_millis(1)).await;
                drop(held);
            })
        })
        .await;
        back.unwrap();
    } else {
        within(back).await.unwrap();
        assert_eq!(
            cov_reports(&sent),
            [(90.0, 0), (50.0, 0)],
            "the write's report"
        );
        hold.release.add_permits(1);
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(
        event_state(&server).await,
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
    assert_eq!(
        event_states(&sent),
        [EventState::HIGH_LIMIT, EventState::NORMAL],
        "the sweep's alarm, then the write's return"
    );
    assert_eq!(
        cov_reports(&sent),
        [(90.0, 0), (50.0, 0)],
        "one report of the value back in range, no IN_ALARM beside it"
    );
}

#[tokio::test(start_paused = true)]
async fn a_sweep_fanout_released_into_a_write_reports_its_result_once() {
    a_delayed_sweep_fanout(true).await;
}

#[tokio::test(start_paused = true)]
async fn a_sweep_fanout_released_after_a_write_reports_nothing_more() {
    a_delayed_sweep_fanout(false).await;
}
