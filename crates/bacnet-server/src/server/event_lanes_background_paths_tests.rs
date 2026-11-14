//! More background paths that commit their transition with their change
//! (#2535, #2537): a Schedule tick's Command runs start while the pass's
//! notifications are held, a Staging plan's membership work commits the
//! relinquished target's return (its COV inside the request's fanout, after
//! the response), and a CreateObject's run of the Schedule it creates
//! commits its target's transition.

use super::*;
use bacnet_encoding::constructed::{
    encode_device_object_property_reference, encode_device_object_reference,
};
use bacnet_objects::staging::{StagingConfig, StagingObject};
use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference, BACnetStageLimitValue,
};

fn sch2() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::SCHEDULE, 2).unwrap()
}

fn stg1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::STAGING, 1).unwrap()
}

fn bv(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_VALUE, instance).unwrap()
}

async fn present_value(rig: &Rig, oid: ObjectIdentifier) -> PropertyValue {
    let db = rig.server.database().read().await;
    db.get(&oid).unwrap().read_property(PV, None).unwrap()
}

/// A peer's WriteProperty of `property` with the encoded `value`.
async fn write_encoded(
    rig: &Rig,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: Vec<u8>,
) -> u8 {
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: value,
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    rig.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
        .await
}

/// AV-2, which CMD-1's list 1 writes 42.0 at priority 8, and SCH-2
/// commanding CMD-1's Present_Value, 0 (no list) to start with.
fn scheduled_command(db: &mut ObjectDatabase) {
    let units = EngineeringUnits::DEGREES_CELSIUS;
    db.add(Box::new(AnalogValueObject::new(2, "AV-2", units).unwrap()))
        .unwrap();
    let mut command = CommandObject::new(1, "CMD-1").unwrap();
    let list = BACnetActionList {
        commands: vec![BACnetActionCommand {
            device_identifier: None,
            object_identifier: av(2),
            property_identifier: PV,
            property_array_index: None,
            property_value: PropertyValue::Real(42.0),
            priority: Some(8),
            post_delay: None,
            quit_on_failure: false,
            write_successful: true,
        }],
    };
    command.set_action(vec![list]).unwrap();
    db.add(Box::new(command)).unwrap();
    let mut schedule = ScheduleObject::new(2, "SCH-2", PropertyValue::Unsigned(0)).unwrap();
    let reference = BACnetObjectPropertyReference::new(cmd1(), PV.to_raw());
    schedule.add_object_property_reference(reference).unwrap();
    db.add(Box::new(schedule)).unwrap();
}

/// The 60-second tick writes BV-1 active, whose OFFNORMAL is held, and
/// CMD-1 to list 1: the run starts with the pass, as a write's runs do, and
/// writes AV-2 while the pass's COV fanout still waits for that OFFNORMAL.
#[tokio::test(start_paused = true)]
async fn a_schedule_ticks_command_run_starts_while_its_notification_is_held() {
    let t0 = Instant::now();
    let config = ServerConfig {
        cov_retry_timeout_ms: 30_000,
        ..ServerConfig::default()
    };
    let rig = Rig::start_with(config, |db| {
        scheduled(db, false);
        scheduled_command(db);
    })
    .await;
    {
        let mut db = rig.server.database().write().await;
        let active = PropertyValue::Enumerated(1);
        let sch1 = db.get_mut(&sch1()).unwrap();
        sch1.write_property(SCHEDULE_DEFAULT, None, active, None)
            .unwrap();
        let sch2 = db.get_mut(&sch2()).unwrap();
        let list = PropertyValue::Unsigned(1);
        sch2.write_property(SCHEDULE_DEFAULT, None, list, None)
            .unwrap();
    }
    rig.hold(true, OFFNORMAL);
    sleep_until(t0, 60_001).await;
    assert!(held_within(&rig, 1).await, "the tick's OFFNORMAL, held");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(present_value(&rig, av(2)).await, PropertyValue::Real(42.0));
    assert_eq!(rig.probe.wire(), [], "still held");
    rig.release();
    assert_eq!(rig.wait_wire(1).await, [ev(bv1(), OFFNORMAL)]);
}

/// BV-1, reporting every transition, and BV-2, plain, with STG-1 in the
/// stage that drives its one target, BV-1, active at priority 8.
fn staged(db: &mut ObjectDatabase) {
    let mut value = BinaryValueObject::new(1, "BV-1").unwrap();
    let enable = all_transitions();
    value
        .write_property(PropertyIdentifier::EVENT_ENABLE, None, enable, None)
        .unwrap();
    db.add(Box::new(value)).unwrap();
    db.add(Box::new(BinaryValueObject::new(2, "BV-2").unwrap()))
        .unwrap();
    let stage = |limit: f32, active: bool| BACnetStageLimitValue {
        limit,
        values: vec![active],
        deadband: 1.0,
    };
    let config = StagingConfig {
        present_value: 15.0,
        min_present_value: 0.0,
        units: EngineeringUnits::DEGREES_CELSIUS,
        priority_for_writing: 8,
        stages: vec![stage(10.0, false), stage(20.0, true)],
        target_references: vec![local(bv1())],
        stage_names: None,
    };
    db.add(Box::new(StagingObject::new(1, "STG-1", config).unwrap()))
        .unwrap();
}

fn local(object_identifier: ObjectIdentifier) -> BACnetDeviceObjectReference {
    BACnetDeviceObjectReference {
        device_identifier: None,
        object_identifier,
    }
}

/// A peer moves STG-1's target from BV-1 to BV-2: the plan's guard writes
/// the NULL STG-1 owes BV-1 (#2289), and returns it to NORMAL.
async fn retarget_staging(rig: &Rig) -> u8 {
    let mut targets = BytesMut::new();
    encode_device_object_reference(&mut targets, &local(bv(2)));
    let property = PropertyIdentifier::TARGET_REFERENCES;
    write_encoded(rig, stg1(), property, targets.to_vec()).await
}

/// The Staging plan's membership work commits BV-1's return to NORMAL with
/// the NULL it writes: the notification goes then, BV-1's report has its
/// flags clear, and the sweeps after it send nothing more.
#[tokio::test(start_paused = true)]
async fn a_staging_plans_membership_work_commits_the_relinquished_targets_return() {
    let t0 = Instant::now();
    let rig = Rig::start(staged).await;
    assert_eq!(rig.event_state(bv1()).await, OFFNORMAL, "the start-up plan");
    let before = stamped(&rig, t0).len();
    watch(&rig, bv1()).await;
    sleep_until(t0, 500).await;
    let id = retarget_staging(&rig).await;
    rig.acked(id).await;
    sleep_until(t0, 501).await;
    let returned = [event(500, bv1(), NORMAL), cov(500, bv1(), 0)];
    assert_eq!(stamped(&rig, t0)[before..], returned);
    assert_eq!(
        present_value(&rig, bv(2)).await,
        PropertyValue::Enumerated(1)
    );
    sleep_until(t0, 3_000).await;
    assert_eq!(stamped(&rig, t0).len(), before + 2, "nothing more");
}

/// BV-1's COV report from a request's Staging membership work joins the
/// request's fanout, after its response: held SimpleACK, the NORMAL goes,
/// the report waits for the response.
#[tokio::test(start_paused = true)]
async fn a_requests_staging_membership_report_goes_after_its_response() {
    let t0 = Instant::now();
    let rig = Rig::start(staged).await;
    let before = stamped(&rig, t0).len();
    watch(&rig, bv1()).await;
    sleep_until(t0, 500).await;
    *rig.probe.hold_ack.lock().unwrap() = Some(1);
    let id = retarget_staging(&rig).await;
    assert_eq!(id, 1);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(stamped(&rig, t0)[before..], [event(500, bv1(), NORMAL)]);
    rig.probe.release_ack.add_permits(1);
    rig.acked(id).await;
    sleep_until(t0, 601).await;
    let returned = [event(500, bv1(), NORMAL), cov(600, bv1(), 0)];
    assert_eq!(stamped(&rig, t0)[before..], returned);
}

/// A peer's CreateObject of a Schedule holding BV-1 active runs it at once
/// (#2208) and commits BV-1's OFFNORMAL with the creation: the notification
/// goes then, the report carries IN_ALARM, and the sweeps after it send
/// nothing more.
#[tokio::test(start_paused = true)]
async fn a_created_schedules_run_commits_its_targets_transition() {
    let t0 = Instant::now();
    let rig = Rig::start(|db| {
        let mut value = BinaryValueObject::new(1, "BV-1").unwrap();
        let enable = all_transitions();
        value
            .write_property(PropertyIdentifier::EVENT_ENABLE, None, enable, None)
            .unwrap();
        db.add(Box::new(value)).unwrap();
    })
    .await;
    watch(&rig, bv1()).await;
    sleep_until(t0, 500).await;
    let mut reference = BytesMut::new();
    let target = BACnetDeviceObjectPropertyReference::new_local(bv1(), PV.to_raw());
    encode_device_object_property_reference(&mut reference, &target);
    let initial = |property_identifier, value| BACnetPropertyValue {
        property_identifier,
        property_array_index: None,
        value,
        priority: None,
    };
    let references = PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES;
    let mut body = BytesMut::new();
    CreateObjectRequest {
        object_specifier: ObjectSpecifier::Type(ObjectType::SCHEDULE),
        list_of_initial_values: vec![
            initial(SCHEDULE_DEFAULT, encoded(PropertyValue::Enumerated(1))),
            initial(references, reference.to_vec()),
        ],
    }
    .encode(&mut body)
    .unwrap();
    rig.request(ConfirmedServiceChoice::CREATE_OBJECT, body)
        .await;
    sleep_until(t0, 501).await;
    let alarmed = [event(500, bv1(), OFFNORMAL), cov(500, bv1(), IN_ALARM)];
    assert_eq!(stamped(&rig, t0), alarmed);
    sleep_until(t0, 3_000).await;
    assert_eq!(stamped(&rig, t0).len(), 2, "nothing more");
}
