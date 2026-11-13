//! A local Reliability write and the intrinsic sweep (#2530).
//!
//! `set_reliability_local` commits under one database guard and runs its
//! event pass under the next, so for a moment the object holds the new
//! Reliability with the old Event_State. Here the sweep commits the fault's
//! transition in that gap, and the COV fanout it then owes runs inside the
//! clear's own gap, so the subscriber hears IN_ALARM without FAULT: a state
//! between a write and its event pass, which no notification should carry.
//! The order is forced with the database lock, which Tokio grants in turn,
//! and a held EventNotification, on paused time.

use super::event_notifications_tests::notification_class_0_broadcasting;
use super::*;
use crate::server::test_transport::{BIP_LOCAL_MAC, SendLog, SentFrame, TestTransport};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_types::enums::{EngineeringUnits, EventState, Reliability};
use std::sync::atomic::AtomicBool;
use tokio::sync::Semaphore;

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
        let event = frame.broadcast
            && matches!(
                frame.apdu(),
                Apdu::UnconfirmedRequest(request)
                    if request.service_choice
                        == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION
            );
        if event && self.armed.swap(false, Ordering::SeqCst) {
            self.held.add_permits(1);
            // Bounded, so a server whose write sends this notification
            // itself, with no sweep between, isn't held for good.
            let release = self.release.acquire();
            if let Ok(permit) = tokio::time::timeout(Duration::from_millis(100), release).await {
                permit.unwrap().forget();
            }
        }
    }
}

/// A Device, Notification Class 0 broadcasting every transition, and AI-1,
/// which reports all three transitions through it.
fn database() -> ObjectDatabase {
    let mut db = clocked_test_database();
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xE0],
        },
        None,
    )
    .unwrap();
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

/// The Status_Flags of each COV notification sent to the subscriber, in
/// order.
fn cov_flags(sent: &SendLog) -> Vec<u8> {
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
            let value = notification
                .list_of_values
                .iter()
                .find(|value| value.property_identifier == PropertyIdentifier::STATUS_FLAGS)
                .expect("Status_Flags reported");
            assert_eq!(&value.value[..2], &[0x82, 0x04], "four-bit Status_Flags");
            value.value[2]
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

/// The sweep commits the fault's transition between the write's commit and
/// its event pass; its COV fanout, held behind its EventNotification, runs
/// between the clear's commit and its event pass. The subscriber should hear
/// FAULT with IN_ALARM, then NORMAL. Today it also hears IN_ALARM alone, the
/// clear half done.
#[tokio::test(start_paused = true)]
async fn a_sweep_fanout_owed_from_the_fault_reports_no_half_cleared_flags() {
    let oid = ai();
    let hold = Arc::new(EventHold::new());
    let on_send = Arc::clone(&hold);
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
    let server = BACnetServer::start(ServerConfig::default(), database(), transport)
        .await
        .expect("server should start");
    // The sweeps run at 0, 1, 2 ... s from here.
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

    // The fault, asked for while the test holds the database: the write
    // queues for it, then the 1 s sweep queues behind the write, so the
    // sweep runs between the write's commit and its event pass.
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
    // The sweep committed the transition and holds in its EventNotification.
    within(async { hold.held.acquire().await.unwrap().forget() }).await;
    assert_eq!(
        event_state(&server).await,
        PropertyValue::Enumerated(EventState::FAULT.to_raw())
    );
    assert_eq!(cov_flags(&sent), [IN_ALARM | FAULT], "the fault's report");

    // The clear, again behind the test's guard. The sweep's send goes out
    // and its COV fanout queues for the database behind the clear's commit.
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
