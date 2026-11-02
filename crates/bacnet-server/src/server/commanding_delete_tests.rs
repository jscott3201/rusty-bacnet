//! A Command or Timer deleted while its run is in progress writes nothing
//! more, and releases nothing it wrote (#2265).
//!
//! The run's generation is checked under the guard that would commit each
//! local write, and before each attempt of a write in another device, so a
//! delete that lands while the run waits (a Post_Delay, a binding lookup, a
//! retry) stops it there. What the run already wrote stays: these writes
//! are one-shot (Clauses 12.10 and 12.57). The clock is paused.
use super::channel_run_tests::{Log, Recorder, rec};
use super::command_action_wire_tests::{ao, cmd, outputs, read_db, slot8, write};
use super::command_remote_write_tests::{
    deliver, device, remote_write, sent_writes, start_local, start_unbound,
};
use super::cov_wire_test_support::*;
use super::remote_write_discovery_tests::{i_am, next_who_is};
use super::*;
use bacnet_objects::command::CommandObject;
use bacnet_objects::timer::TimerObject;
use bacnet_services::object_mgmt::DeleteObjectRequest;
use bacnet_types::constructed::{
    BACnetActionCommand, BACnetActionList, BACnetDeviceObjectPropertyReference,
    BACnetTimerStateChangeValue as V,
};

async fn delete(h: &mut Harness, oid: ObjectIdentifier) {
    let mut body = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: oid,
    }
    .encode(&mut body);
    h.request(ConfirmedServiceChoice::DELETE_OBJECT, body).await;
    h.settle().await;
    assert!(h.server.database().read().await.get(&oid).is_none());
}

/// CMD-1's list 1 writes AO-2 to 30.0 at priority 8, waits 5 seconds, then
/// writes AO-1 to 70.0.
fn delayed_command(db: &mut ObjectDatabase) {
    outputs(db);
    let mut command = CommandObject::new(1, "CMD-1").unwrap();
    command
        .set_action(vec![BACnetActionList {
            commands: vec![
                BACnetActionCommand {
                    post_delay: Some(5),
                    ..write(ao(2), PropertyValue::Real(30.0), 8)
                },
                write(ao(1), PropertyValue::Real(70.0), 8),
            ],
        }])
        .unwrap();
    db.add(Box::new(command)).unwrap();
}

async fn write_command(h: &Harness, value: u64) {
    h.server
        .write_local(
            &cmd(1),
            PV,
            None,
            PropertyValue::Unsigned(value),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    h.settle().await;
}

#[tokio::test(start_paused = true)]
async fn command_deleted_during_a_post_delay_writes_no_further_action() {
    let mut h = Harness::start_with(ServerConfig::default(), delayed_command).await;
    write_command(&h, 1).await;
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(30.0));
    delete(&mut h, cmd(1)).await;
    tokio::time::sleep(Duration::from_secs(10)).await;
    // The first write stays, with nothing to relinquish it; the second is
    // never made.
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(30.0));
    assert_eq!(slot8(&h, ao(1)).await, PropertyValue::Null);
}

#[tokio::test(start_paused = true)]
async fn command_deleted_during_a_post_delay_leaves_a_target_taking_any_writer_unwritten() {
    // REC-1 takes writes with no command origin, so only the run's own
    // checks keep the second action from it once CMD-1 is gone.
    let log = Log::default();
    let recorder = Recorder {
        oid: rec(),
        log: Arc::clone(&log),
    };
    let mut h = Harness::start_with(ServerConfig::default(), move |db| {
        db.add(Box::new(recorder)).unwrap();
        let property = PropertyIdentifier::from_raw;
        let mut command = CommandObject::new(1, "CMD-1").unwrap();
        command
            .set_action(vec![BACnetActionList {
                commands: vec![
                    BACnetActionCommand {
                        property_identifier: property(1000),
                        post_delay: Some(5),
                        ..write(rec(), PropertyValue::Real(30.0), 8)
                    },
                    BACnetActionCommand {
                        property_identifier: property(1001),
                        ..write(rec(), PropertyValue::Real(70.0), 8)
                    },
                ],
            }])
            .unwrap();
        db.add(Box::new(command)).unwrap();
    })
    .await;
    write_command(&h, 1).await;
    assert_eq!(log.lock().unwrap().len(), 1);
    delete(&mut h, cmd(1)).await;
    tokio::time::sleep(Duration::from_secs(10)).await;
    let log = log.lock().unwrap();
    assert_eq!(log.len(), 1, "{log:?}");
}

#[tokio::test(start_paused = true)]
async fn command_deleted_during_an_outstanding_remote_write_sends_no_retry() {
    let mut h = start_local().await;
    write_command(&h, 1).await;
    remote_write(&h).await;
    delete(&mut h, cmd(1)).await;
    // The retries would go out 3, 6 and 9 seconds on; the first finds the
    // run stale and ends the write, freeing its invoke ID.
    tokio::time::sleep(Duration::from_secs(15)).await;
    assert!(sent_writes(&h).is_empty());
    assert_eq!(h.server.notification_transactions.active_count(), 0);
    // The local action after it is never written.
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Null);
}

#[tokio::test(start_paused = true)]
async fn command_deleted_while_its_remote_device_is_looked_for_sends_no_write() {
    let mut h = start_unbound().await;
    write_command(&h, 1).await;
    next_who_is(&h).await;
    delete(&mut h, cmd(1)).await;
    // The I-Am binds Device 9, but the run is gone: no WriteProperty.
    deliver(&h, &i_am(9), &PEER, None).await;
    tokio::time::sleep(Duration::from_secs(15)).await;
    assert!(sent_writes(&h).is_empty());
    assert_eq!(h.server.notification_transactions.active_count(), 0);
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Null);
}

fn tmr() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::TIMER, 1).unwrap()
}

/// TMR-1 runs 400 ms when started, writing 10.0 to its references at
/// priority 9 on IDLE_TO_RUNNING and 40.0 on RUNNING_TO_EXPIRED.
fn timer(references: Vec<BACnetDeviceObjectPropertyReference>) -> TimerObject {
    let real = |value| V::Primitive(PropertyValue::Real(value));
    let mut timer = TimerObject::new(1, "TMR-1").unwrap();
    timer.set_pres_value_range(100, 600_000).unwrap();
    timer.set_default_timeout(400).unwrap();
    timer
        .set_state_change(
            [
                real(10.0),
                V::NoValue,
                V::NoValue,
                real(40.0),
                V::NoValue,
                V::NoValue,
                V::NoValue,
            ],
            references,
            9,
        )
        .unwrap();
    timer
}

async fn start_timer(h: &Harness) {
    h.server
        .write_local(
            &tmr(),
            PropertyIdentifier::TIMER_RUNNING,
            None,
            PropertyValue::Boolean(true),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    h.settle().await;
}

async fn slot9(h: &Harness, object: ObjectIdentifier) -> PropertyValue {
    read_db(h, object, PropertyIdentifier::PRIORITY_ARRAY, Some(9)).await
}

#[tokio::test(start_paused = true)]
async fn running_timer_deleted_makes_no_expiry_writes_and_keeps_its_start_writes() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        outputs(db);
        let local = BACnetDeviceObjectPropertyReference::new_local(ao(1), PV.to_raw());
        db.add(Box::new(timer(vec![local]))).unwrap();
    })
    .await;
    start_timer(&h).await;
    assert_eq!(slot9(&h, ao(1)).await, PropertyValue::Real(10.0));
    delete(&mut h, tmr()).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(slot9(&h, ao(1)).await, PropertyValue::Real(10.0));
}

#[tokio::test(start_paused = true)]
async fn timer_deleted_while_its_start_writes_wait_on_another_device_writes_no_more() {
    // AO-1 in Device 9 first, then the local AO-2.
    let remote = BACnetDeviceObjectPropertyReference {
        device_identifier: Some(device(9)),
        ..BACnetDeviceObjectPropertyReference::new_local(ao(1), PV.to_raw())
    };
    let local = BACnetDeviceObjectPropertyReference::new_local(ao(2), PV.to_raw());
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        outputs(db);
        db.add(Box::new(timer(vec![remote, local]))).unwrap();
    })
    .await;
    h.server
        .device_bindings
        .write()
        .await
        .insert_configured(DeviceBinding::local(device(9), PEER).unwrap(), |_| false)
        .unwrap();
    start_timer(&h).await;
    let sent = sent_writes(&h);
    assert_eq!(sent.len(), 1, "{sent:?}");
    delete(&mut h, tmr()).await;
    tokio::time::sleep(Duration::from_secs(15)).await;
    assert!(sent_writes(&h).is_empty());
    assert_eq!(h.server.notification_transactions.active_count(), 0);
    assert_eq!(slot9(&h, ao(2)).await, PropertyValue::Null);
}
