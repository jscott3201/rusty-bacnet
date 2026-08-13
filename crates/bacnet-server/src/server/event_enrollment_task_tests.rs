//! Lifecycle wiring proof for the Event Enrollment `Time_Delay` countdown
//! (#163): the spawned `event_enrollment_task` drives
//! `evaluate_event_enrollments` on `event_enrollment_interval_secs`, and each
//! pass advances a pending countdown by one tick. The per-pass semantics
//! themselves are pinned deterministically in
//! `crate::event_enrollment::tests::delays`; what this test adds is the
//! wall-clock proof that the *real spawned task* advances and fires the
//! countdown — the same style of proof `event_notifications_tests.rs` uses
//! for the intrinsic `Time_Delay` path.

use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};
use bacnet_types::enums::{EventState, EventType};
use std::sync::{Arc as StdArc, Mutex as StdMutex};
use tokio::sync::mpsc;

/// Records broadcasts (unused here — #127 means the EE path still only logs)
/// and discards unicasts; the same minimal harness the other server tests use.
#[derive(Clone, Default)]
struct RecordingTransport {
    sent_broadcast: StdArc<StdMutex<Vec<bytes::Bytes>>>,
}

impl TransportPort for RecordingTransport {
    async fn start(
        &mut self,
    ) -> Result<mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>, Error> {
        let (_tx, rx) = mpsc::channel(1);
        Ok(rx)
    }
    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }
    async fn send_unicast(&self, _npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.sent_broadcast
            .lock()
            .unwrap()
            .push(bytes::Bytes::copy_from_slice(npdu));
        Ok(())
    }
    fn local_mac(&self) -> &[u8] {
        &[127, 0, 0, 1, 0xBA, 0xC0]
    }
}

/// Time_Delay=2 on a one-second evaluation interval: the first pass of the
/// out-of-range condition seeds the countdown, and the transition fires on
/// the pass where it reaches zero (~2s later) — before that the observable
/// `Event_State` holds NORMAL (Clause 13.2.4), after that HIGH_LIMIT.
///
/// Wall-clock assertions use margins around the interval boundary: at 1.4s
/// exactly two passes have run (tokio's interval ticks immediately, then at
/// 1s) and the countdown stands at 1; by 2.6s the 2s tick has fired it.
#[tokio::test(start_paused = true)]
async fn spawned_task_advances_and_fires_the_time_delay_countdown() {
    let transport = RecordingTransport::default();
    let sent = StdArc::clone(&transport.sent_broadcast);

    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    ai.set_present_value(85.0); // above the high limit below
    let ai_oid = ai.object_identifier();

    let mut ee = EventEnrollmentObject::new(1, "EE-OOR", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 2,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee.set_event_enable(0x07);
    let ee_oid = ee.object_identifier();

    let mut db = ObjectDatabase::new();
    db.add(Box::new(ai)).unwrap();
    db.add(Box::new(ee)).unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 1,
            name: "Dev".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();

    let config = ServerConfig {
        event_enrollment_interval_secs: 1,
        ..ServerConfig::default()
    };
    let server = BACnetServer::start(config, db, transport)
        .await
        .expect("server should start");

    let read_state = |db: &ObjectDatabase| match db
        .get(&ee_oid)
        .unwrap()
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap()
    {
        PropertyValue::Enumerated(v) => EventState::from_raw(v),
        other => panic!("EVENT_STATE must read Enumerated, got {other:?}"),
    };

    // t≈0: let the immediate first tick run; it seeds the countdown.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        read_state(&*server.database().read().await),
        EventState::NORMAL,
        "first pass seeds the countdown; the confirmed state must not move"
    );

    // t≈1.4s: two passes in, countdown stands at 1 — still NORMAL.
    tokio::time::sleep(Duration::from_millis(1300)).await;
    assert_eq!(
        read_state(&*server.database().read().await),
        EventState::NORMAL,
        "delay not yet elapsed: NORMAL holds (13.2.4)"
    );

    // t≈2.6s: the t=2s tick counted down to zero and fired HIGH_LIMIT.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(
        read_state(&*server.database().read().await),
        EventState::HIGH_LIMIT,
        "Time_Delay elapsed under the real spawned task: the transition fired"
    );

    // Nothing was distributed — #127 owns sending; this tranche only logs.
    assert!(
        sent.lock().unwrap().is_empty(),
        "the EE path still does not emit notifications (#127)"
    );
}
