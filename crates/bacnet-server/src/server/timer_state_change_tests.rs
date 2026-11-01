//! A Timer's transitions write its State_Change_Values to its referenced
//! properties, on a running server (#2206, Clause 12.57).
//!
//! TMR-1 accepts timeouts of 100 ms to 10 minutes. Its references are AO-1,
//! AO-9 (which doesn't exist) and AO-2, written at priority 9. Each
//! transition writes its own REAL, except that RUNNING_TO_IDLE relinquishes
//! (NULL) and EXPIRED_TO_IDLE writes nothing. The clock is paused, and the
//! server's operation task expires the Timer at its deadline.
use super::command_action_wire_tests::{ao, read_db};
use super::command_remote_write_tests::{ack, device, sent_writes};
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::timer::TimerObject;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetTimerStateChangeValue as V,
};
use bacnet_types::enums::{Reliability, TimerState};

fn tmr() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::TIMER, 1).unwrap()
}

fn reference(object: ObjectIdentifier) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference::new_local(object, PV.to_raw())
}

fn real(value: f32) -> V {
    V::Primitive(PropertyValue::Real(value))
}

fn timer(references: Vec<BACnetDeviceObjectPropertyReference>) -> TimerObject {
    let mut timer = TimerObject::new(1, "TMR-1").unwrap();
    timer.set_pres_value_range(100, 600_000).unwrap();
    timer.set_default_timeout(400).unwrap();
    timer
        .set_state_change(
            [
                real(10.0),                        // IDLE_TO_RUNNING
                V::Primitive(PropertyValue::Null), // RUNNING_TO_IDLE
                real(30.0),                        // RUNNING_TO_RUNNING
                real(40.0),                        // RUNNING_TO_EXPIRED
                real(50.0),                        // FORCED_TO_EXPIRED
                V::NoValue,                        // EXPIRED_TO_IDLE
                real(70.0),                        // EXPIRED_TO_RUNNING
            ],
            references,
            9,
        )
        .unwrap();
    timer
}

async fn start(references: Vec<BACnetDeviceObjectPropertyReference>) -> Harness {
    Harness::start_with(ServerConfig::default(), |db| {
        super::command_action_wire_tests::outputs(db);
        db.add(Box::new(timer(references))).unwrap();
    })
    .await
}

/// Write one of TMR-1's properties through the server's local write path.
async fn write_timer(h: &Harness, property: PropertyIdentifier, value: PropertyValue) {
    h.server
        .write_local(
            &tmr(),
            property,
            None,
            value,
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    h.settle().await;
}

/// Slot 9 of AO-1's and AO-2's Priority_Arrays.
async fn slots(h: &Harness) -> [PropertyValue; 2] {
    let slot = |object| read_db(h, object, PropertyIdentifier::PRIORITY_ARRAY, Some(9));
    [slot(ao(1)).await, slot(ao(2)).await]
}

fn both(value: f32) -> [PropertyValue; 2] {
    [PropertyValue::Real(value), PropertyValue::Real(value)]
}

#[tokio::test(start_paused = true)]
async fn each_transition_writes_its_value_at_the_priority_and_a_failure_stops_none() {
    let mut h = start(vec![reference(ao(1)), reference(ao(9)), reference(ao(2))]).await;
    let running = PropertyIdentifier::TIMER_RUNNING;
    let state = PropertyIdentifier::TIMER_STATE;
    assert_eq!(slots(&h).await, [PropertyValue::Null, PropertyValue::Null]);

    write_timer(&h, PV, PropertyValue::Unsigned(300)).await;
    // AO-9 fails between the two, and AO-2 is still written.
    assert_eq!(slots(&h).await, both(10.0));
    // The failure is recorded: the target doesn't exist.
    assert_eq!(
        read_db(&h, tmr(), PropertyIdentifier::RELIABILITY, None).await,
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );
    write_timer(&h, PV, PropertyValue::Unsigned(300)).await;
    assert_eq!(slots(&h).await, both(30.0));
    // The operation task expires it at 300 ms.
    tokio::time::sleep(Duration::from_millis(300)).await;
    h.settle().await;
    assert_eq!(
        read_db(&h, tmr(), state, None).await,
        PropertyValue::Enumerated(TimerState::EXPIRED.to_raw())
    );
    assert_eq!(slots(&h).await, both(40.0));
    write_timer(&h, running, PropertyValue::Boolean(true)).await;
    assert_eq!(slots(&h).await, both(70.0));
    write_timer(&h, PV, PropertyValue::Unsigned(0)).await;
    assert_eq!(slots(&h).await, both(50.0));
    // EXPIRED_TO_IDLE has no value: nothing is written.
    let idle = PropertyValue::Enumerated(TimerState::IDLE.to_raw());
    write_timer(&h, state, idle.clone()).await;
    assert_eq!(slots(&h).await, both(50.0));
    write_timer(&h, running, PropertyValue::Boolean(true)).await;
    assert_eq!(slots(&h).await, both(10.0));
    // RUNNING_TO_IDLE relinquishes slot 9.
    write_timer(&h, state, idle).await;
    assert_eq!(slots(&h).await, [PropertyValue::Null, PropertyValue::Null]);
    h.server.stop().await.unwrap();
}

/// A harness whose Timer writes AO-1 in Device 9, bound to the peer, then
/// the local AO-2.
async fn start_remote() -> Harness {
    let remote = BACnetDeviceObjectPropertyReference {
        device_identifier: Some(device(9)),
        ..reference(ao(1))
    };
    let h = start(vec![remote, reference(ao(2))]).await;
    h.server
        .device_bindings
        .write()
        .await
        .insert_configured(DeviceBinding::local(device(9), PEER).unwrap(), |_| false)
        .unwrap();
    h
}

/// The one WriteProperty the server has sent since the last call: its invoke
/// ID and value.
async fn next_remote(h: &Harness) -> (u8, Vec<u8>) {
    for _ in 0..100 {
        let mut sent = sent_writes(h);
        if let Some((invoke_id, request)) = sent.pop() {
            assert!(sent.is_empty());
            assert_eq!(request.object_identifier, ao(1));
            assert_eq!(request.priority, Some(9));
            return (invoke_id, request.property_value);
        }
        h.settle().await;
    }
    panic!("no WriteProperty to Device 9");
}

fn encoded(value: f32) -> Vec<u8> {
    super::cov_wire_test_support::real(value)
}

#[tokio::test(start_paused = true)]
async fn an_expiry_writes_after_the_pass_lets_the_database_go() {
    let mut h = start_remote().await;
    write_timer(&h, PV, PropertyValue::Unsigned(200)).await;
    let (invoke_id, value) = next_remote(&h).await;
    assert_eq!(value, encoded(10.0));
    h.respond(ack(invoke_id)).await;
    h.settle().await;
    assert_eq!(slots(&h).await[1], PropertyValue::Real(10.0));

    // The operation task's pass expires the Timer under its guard; the
    // remote write goes out once that guard is gone, and while it waits for
    // its answer the database is free.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (invoke_id, value) = next_remote(&h).await;
    assert_eq!(value, encoded(40.0));
    assert!(h.server.database().try_write().is_ok());
    assert_eq!(slots(&h).await[1], PropertyValue::Real(10.0));
    h.respond(ack(invoke_id)).await;
    h.settle().await;
    assert_eq!(slots(&h).await[1], PropertyValue::Real(40.0));
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_newer_transition_supersedes_a_run_still_writing() {
    let mut h = start_remote().await;
    write_timer(&h, PV, PropertyValue::Unsigned(60_000)).await;
    let (first, value) = next_remote(&h).await;
    assert_eq!(value, encoded(10.0));
    // Restarted while IDLE_TO_RUNNING's remote write is unanswered.
    write_timer(&h, PV, PropertyValue::Unsigned(60_000)).await;
    h.respond(ack(first)).await;
    // The stale run stops: AO-2 never takes 10.0, and the new run writes
    // RUNNING_TO_RUNNING's 30.0, remote first.
    let (second, value) = next_remote(&h).await;
    assert_eq!(value, encoded(30.0));
    assert_eq!(slots(&h).await[1], PropertyValue::Null);
    h.respond(ack(second)).await;
    h.settle().await;
    assert_eq!(slots(&h).await[1], PropertyValue::Real(30.0));
    h.server.stop().await.unwrap();
}

/// A refused write that waited behind a held guard past the deadline applies
/// the due expiry first; its writes still go out, though the write that made
/// the transition fails and the operation task's pass then finds nothing due.
#[tokio::test(start_paused = true)]
async fn an_expiry_a_refused_write_applied_still_writes() {
    let mut h = start(vec![reference(ao(1)), reference(ao(2))]).await;
    write_timer(&h, PV, PropertyValue::Unsigned(200)).await;
    assert_eq!(slots(&h).await, both(10.0));
    let oid = tmr();
    let guard = h.server.database().write().await;
    // Timer_State RUNNING is refused, but only after the write has applied
    // whatever expiry is due by the time it holds the database.
    let mut refused = Box::pin(h.server.write_local(
        &oid,
        PropertyIdentifier::TIMER_STATE,
        None,
        PropertyValue::Enumerated(TimerState::RUNNING.to_raw()),
        None,
        crate::LocalCommandSource::ServerDevice,
    ));
    // Polled once, it waits for the guard ahead of the operation task.
    assert!(
        tokio::time::timeout(Duration::ZERO, &mut refused)
            .await
            .is_err()
    );
    tokio::time::advance(Duration::from_millis(250)).await;
    h.settle().await;
    drop(guard);
    assert!(refused.await.is_err());
    for _ in 0..5 {
        h.settle().await;
    }
    assert_eq!(
        read_db(&h, tmr(), PropertyIdentifier::TIMER_STATE, None).await,
        PropertyValue::Enumerated(TimerState::EXPIRED.to_raw())
    );
    assert_eq!(slots(&h).await, both(40.0));
    h.server.stop().await.unwrap();
}

/// A lighting-command value reaches a Lighting Output's Lighting_Command as
/// the command a WriteProperty of it would carry.
#[tokio::test(start_paused = true)]
async fn a_lighting_command_value_drives_a_lighting_output() {
    use bacnet_objects::lighting::LightingOutputObject;
    use bacnet_types::constructed::BACnetLightingCommand;
    use bacnet_types::enums::LightingOperation;
    let lo = ObjectIdentifier::new(ObjectType::LIGHTING_OUTPUT, 1).unwrap();
    let command = BACnetLightingCommand {
        target_level: Some(40.0),
        ..BACnetLightingCommand::new(LightingOperation::FADE_TO)
    };
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(LightingOutputObject::new(1, "LO-1").unwrap()))
            .unwrap();
        let mut values = core::array::from_fn(|_| V::NoValue);
        values[0] = V::LightingCommand(command);
        let mut timer = TimerObject::new(1, "TMR-1").unwrap();
        let target = BACnetDeviceObjectPropertyReference::new_local(
            lo,
            PropertyIdentifier::LIGHTING_COMMAND.to_raw(),
        );
        timer.set_state_change(values, vec![target], 9).unwrap();
        db.add(Box::new(timer)).unwrap();
    })
    .await;
    write_timer(&h, PV, PropertyValue::Unsigned(60_000)).await;
    let mut encoded = BytesMut::new();
    bacnet_encoding::constructed::encode_lighting_command(&mut encoded, &command);
    assert_eq!(
        read_db(&h, lo, PropertyIdentifier::LIGHTING_COMMAND, None).await,
        PropertyValue::ApplicationData(encoded.to_vec())
    );
    h.server.stop().await.unwrap();
}
