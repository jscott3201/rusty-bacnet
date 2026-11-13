//! The lanes when something goes wrong or a writer doesn't wait (#2537): a
//! server dropped without `stop()`, a send that panics, a Staging plan's
//! response behind another writer's held lane, a push made under the
//! committing guard, and two writes pipelined from one client.

use super::*;
use crate::server::event_notifications::commit_written_transition;
use bacnet_objects::event::EventStateChange;
use bacnet_objects::staging::{StagingConfig, StagingObject};
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};

fn stg1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::STAGING, 1).unwrap()
}

/// AI-1, BV-1 reporting every transition, and STG-1 whose top stage turns
/// BV-1 on at priority 8.
fn staging_objects(db: &mut ObjectDatabase) {
    ai_objects(db);
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    bv.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        all_transitions(),
        None,
    )
    .unwrap();
    db.add(Box::new(bv)).unwrap();
    let stage = |limit, active| BACnetStageLimitValue {
        limit,
        values: vec![active],
        deadband: 1.0,
    };
    let config = StagingConfig {
        present_value: 5.0,
        min_present_value: 0.0,
        units: EngineeringUnits::DEGREES_CELSIUS,
        priority_for_writing: 8,
        stages: vec![stage(10.0, false), stage(30.0, false), stage(50.0, true)],
        target_references: vec![BACnetDeviceObjectReference {
            device_identifier: None,
            object_identifier: bv1(),
        }],
        stage_names: None,
    };
    db.add(Box::new(StagingObject::new(1, "STG-1", config).unwrap()))
        .unwrap();
}

/// A server dropped without `stop()` with a lane held: the drop seals the
/// lanes, so nothing goes out after it, no lane or lane task is left, and
/// the database's last handles go with them.
#[tokio::test(start_paused = true)]
async fn a_server_dropped_without_stop_sends_nothing_more() {
    let rig = Rig::start(ai_objects).await;
    rig.hold(true, FAULT);
    rig.reliability(Reliability::NO_SENSOR).await;
    rig.holding().await;
    rig.reliability(Reliability::NO_FAULT_DETECTED).await;
    let transactions = Arc::clone(&rig.server.notification_transactions);
    let db = Arc::clone(rig.server.database());
    let Rig { server, probe, .. } = rig;
    drop(server);
    tokio::time::sleep(Duration::from_millis(100)).await;
    {
        let lanes = lanes(&transactions);
        assert_eq!((lanes.open(), lanes.tasks.len()), (0, 0), "no lane left");
    }
    assert_eq!(
        Arc::strong_count(&db),
        1,
        "only this test holds the database"
    );
    probe.release.add_permits(1);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(probe.wire(), [], "nothing sent after the drop");
}

/// A lane whose task panics in a send is taken over: the next notification
/// to that destination still goes out.
#[tokio::test(start_paused = true)]
async fn a_send_that_panics_does_not_strand_its_lane() {
    let rig = Rig::start(ai_objects).await;
    let (spy, _guard) = LaneLockSpy::install(&rig.server.notification_transactions);
    *rig.probe.panic_on.lock().unwrap() = Some(FAULT);
    rig.reliability(Reliability::NO_SENSOR).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        *rig.probe.panic_on.lock().unwrap(),
        None,
        "the send panicked"
    );
    rig.reliability(Reliability::NO_FAULT_DETECTED).await;
    assert_eq!(rig.wait_wire(1).await, [ev(ai1(), NORMAL)]);
    // The early end's warning, and the panic's as the next push reaps the
    // task, went out after the lanes' lock was let go.
    spy.assert_logged_unlocked(2);
    let lanes = lanes(&rig.server.notification_transactions);
    assert_eq!((lanes.open(), lanes.waiting()), (0, 0));
}

/// A send that panics with entries queued behind it: a new task takes the
/// rest of the lane, in order, and the lane closes.
#[tokio::test(start_paused = true)]
async fn a_new_task_takes_the_entries_behind_a_panicking_send() {
    let rig = Rig::start(ai_objects).await;
    rig.hold(true, FAULT);
    rig.reliability(Reliability::NO_SENSOR).await;
    rig.holding().await;
    *rig.probe.panic_on.lock().unwrap() = Some(FAULT);
    rig.reliability(Reliability::NO_FAULT_DETECTED).await;
    rig.reliability(Reliability::NO_SENSOR).await;
    rig.reliability(Reliability::NO_FAULT_DETECTED).await;
    assert_eq!(lanes(&rig.server.notification_transactions).waiting(), 3);
    rig.release();
    let wire = rig.wait_wire(3).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        wire,
        [ev(ai1(), FAULT), ev(ai1(), NORMAL), ev(ai1(), NORMAL)]
    );
    let lanes = lanes(&rig.server.notification_transactions);
    assert_eq!((lanes.open(), lanes.waiting()), (0, 0), "the lane closed");
}

/// Every send of one kind panics: each entry gets one try, the others go
/// out, and the lane closes rather than spinning.
#[tokio::test(start_paused = true)]
async fn a_send_that_always_panics_still_lets_the_lane_close() {
    let rig = Rig::start(ai_objects).await;
    rig.hold(true, FAULT);
    rig.reliability(Reliability::NO_SENSOR).await;
    rig.holding().await;
    *rig.probe.panic_always.lock().unwrap() = Some(NORMAL);
    for _ in 0..3 {
        rig.reliability(Reliability::NO_FAULT_DETECTED).await;
        rig.reliability(Reliability::NO_SENSOR).await;
    }
    assert_eq!(lanes(&rig.server.notification_transactions).waiting(), 6);
    rig.release();
    let wire = rig.wait_wire(4).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(wire, vec![ev(ai1(), FAULT); 4]);
    let lanes = lanes(&rig.server.notification_transactions);
    assert_eq!((lanes.open(), lanes.waiting()), (0, 0), "the lane closed");
}

/// A write whose Staging plan commits a transition answers at once while
/// another writer's notification holds the same lane; the plan target's
/// COV notification still follows its event notification.
#[tokio::test(start_paused = true)]
async fn a_staging_plan_does_not_hold_its_requests_response() {
    let rig = Rig::start(staging_objects).await;
    {
        let mut table = rig.server.cov_table.write().await;
        table
            .subscribe(CovSubscription {
                subscriber_mac: MacAddr::from_slice(&SUBSCRIBER),
                subscriber_network: None,
                subscriber_process_identifier: 1,
                monitored_object_identifier: bv1(),
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
    rig.hold(true, FAULT);
    rig.reliability(Reliability::NO_SENSOR).await;
    rig.holding().await;
    let id = rig.write(stg1(), PV, PropertyValue::Real(60.0)).await;
    let started = tokio::time::Instant::now();
    rig.acked(id).await;
    let waited = started.elapsed();
    assert!(
        waited < Duration::from_millis(10),
        "answered after {waited:?}"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(rig.probe.wire(), [], "the plan target's COV waits too");
    rig.release();
    let wire = rig.wait_wire(3).await;
    assert_eq!(
        wire,
        [ev(ai1(), FAULT), ev(bv1(), OFFNORMAL), Sent::Cov(bv1())]
    );
    assert_eq!(rig.server.event_notification_counters().cov_wait_expired, 0);
}

/// The entry is in its lane before the committing guard drops, on the path
/// every write takes ([`commit_written_transition`]: peer writes, local
/// writes, Staging plans, local authorization).
#[tokio::test(start_paused = true)]
async fn a_writes_notification_is_in_its_lane_before_the_guard_drops() {
    let rig = Rig::start(ai_objects).await;
    let writer = rig.server.local_writer();
    let delivery = writer.event_delivery();
    let mut settle = Settle::default();
    let mut db = rig.server.database().write().await;
    let value = PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw());
    db.get_mut(&ai1())
        .unwrap()
        .write_property(PropertyIdentifier::RELIABILITY, None, value, None)
        .unwrap();
    assert!(commit_written_transition(
        || delivery,
        &mut db,
        ai1(),
        &mut settle
    ));
    let lanes = lanes(&rig.server.notification_transactions);
    assert_eq!(
        (lanes.open(), lanes.waiting()),
        (1, 1),
        "pushed under the guard"
    );
}

/// The same for the sweep's and the Event Enrollment pass's path, which
/// push a transition they committed through
/// [`BACnetServer::enqueue_event_notification`] under their own guard.
#[tokio::test(start_paused = true)]
async fn a_committed_transitions_notification_is_pushed_under_the_callers_guard() {
    let rig = Rig::start(ai_objects).await;
    let writer = rig.server.local_writer();
    let delivery = writer.event_delivery();
    let mut db = rig.server.database().write().await;
    let change = EventStateChange {
        from: EventState::NORMAL,
        to: EventState::FAULT,
    };
    let transition = (change, EventType::CHANGE_OF_RELIABILITY);
    BACnetServer::enqueue_event_notification(&delivery, &mut db, &ai1(), transition, None);
    let lanes = lanes(&rig.server.notification_transactions);
    assert_eq!(
        (lanes.open(), lanes.waiting()),
        (1, 1),
        "pushed under the guard"
    );
}

/// Two writes pipelined from one client: the first's SimpleACK is held in
/// the transport and the second is sent without waiting for it. Each
/// notification joins the lane as its write commits, so they go out in
/// commit order.
#[tokio::test(start_paused = true)]
async fn pipelined_writes_go_out_in_commit_order() {
    let rig = Rig::start(ai_objects).await;
    *rig.probe.hold_ack.lock().unwrap() = Some(1);
    let fault = PropertyValue::Enumerated(Reliability::NO_SENSOR.to_raw());
    let first = rig
        .write(ai1(), PropertyIdentifier::RELIABILITY, fault)
        .await;
    assert_eq!(first, 1);
    let clear = PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw());
    let second = rig
        .write(ai1(), PropertyIdentifier::RELIABILITY, clear)
        .await;
    rig.acked(second).await;
    assert_eq!(
        rig.wait_wire(2).await,
        [ev(ai1(), FAULT), ev(ai1(), NORMAL)]
    );
    rig.probe.release_ack.add_permits(1);
    rig.acked(first).await;
    assert_eq!(rig.event_state(ai1()).await, NORMAL);
}
