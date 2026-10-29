//! Runs held between their take and their owner end when dropped there, at
//! once or once the database is free, and a run ends only once (#1324).
//!
//! CMD-1's list 1 writes AO-1 twice; CH-1 has AO-1 as its one member. Every
//! command starts with its write-successful flag TRUE, so a FALSE is the
//! drop's doing.
use super::*;
use bacnet_encoding::constructed::decode_action_list;
use bacnet_objects::channel::ChannelObject;
use bacnet_objects::command::CommandObject;
use bacnet_types::constructed::{
    BACnetActionCommand, BACnetActionList, BACnetDeviceObjectPropertyReference,
};
use bacnet_types::enums::{ObjectType, PropertyIdentifier, Reliability, WriteStatus};
use bacnet_types::primitives::PropertyValue;

const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;

fn cmd1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::COMMAND, 1).unwrap()
}

fn ch1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::CHANNEL, 1).unwrap()
}

fn ao1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap()
}

fn database() -> Arc<RwLock<ObjectDatabase>> {
    let command = |value| BACnetActionCommand {
        device_identifier: None,
        object_identifier: ao1(),
        property_identifier: PV,
        property_array_index: None,
        property_value: PropertyValue::Real(value),
        priority: Some(8),
        post_delay: None,
        quit_on_failure: false,
        write_successful: true,
    };
    let mut cmd = CommandObject::new(1, "CMD-1").unwrap();
    cmd.set_action(vec![BACnetActionList {
        commands: vec![command(1.0), command(2.0)],
    }])
    .unwrap();
    let mut ch = ChannelObject::new(1, "CH-1", 7).unwrap();
    ch.set_members(vec![BACnetDeviceObjectPropertyReference::new_local(
        ao1(),
        PV.to_raw(),
    )])
    .unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(cmd)).unwrap();
    db.add(Box::new(ch)).unwrap();
    Arc::new(RwLock::new(db))
}

fn write_pv(db: &mut ObjectDatabase, object: ObjectIdentifier, value: PropertyValue) {
    db.get_mut(&object)
        .unwrap()
        .write_property(PV, None, value, None)
        .unwrap();
}

/// Write CMD-1's and CH-1's Present_Value, then take the runs those writes
/// queued under the same guard.
async fn write_and_take(database: &Arc<RwLock<ObjectDatabase>>) -> TakenRuns {
    let mut db = database.write().await;
    write_pv(&mut db, cmd1(), PropertyValue::Unsigned(1));
    write_pv(&mut db, ch1(), PropertyValue::Real(1.0));
    TakenRuns::take(database, &mut db, &[cmd1(), ch1()])
}

fn read(
    db: &ObjectDatabase,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    db.get(&object)
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

fn in_process(db: &ObjectDatabase) -> bool {
    read(db, cmd1(), PropertyIdentifier::IN_PROCESS) == PropertyValue::Boolean(true)
}

fn write_status(db: &ObjectDatabase) -> WriteStatus {
    match read(db, ch1(), PropertyIdentifier::WRITE_STATUS) {
        PropertyValue::Enumerated(raw) => WriteStatus::from_raw(raw),
        other => panic!("not an ENUMERATED: {other:?}"),
    }
}

/// Both runs ended as if none of their writes had been made.
fn assert_ended_unmade(db: &ObjectDatabase) {
    assert!(!in_process(db));
    assert_eq!(
        read(db, cmd1(), PropertyIdentifier::ALL_WRITES_SUCCESSFUL),
        PropertyValue::Boolean(false)
    );
    let PropertyValue::ApplicationData(element) = db
        .get(&cmd1())
        .unwrap()
        .read_property(PropertyIdentifier::ACTION, Some(1))
        .unwrap()
    else {
        panic!("an encoded action list");
    };
    let (list, _) = decode_action_list(&element, 0).unwrap();
    assert!(
        list.commands
            .iter()
            .all(|command| !command.write_successful)
    );
    assert_eq!(write_status(db), WriteStatus::FAILED);
    assert_eq!(
        read(db, ch1(), PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(Reliability::PROCESS_ERROR.to_raw())
    );
}

#[tokio::test]
async fn taken_runs_dropped_unstarted_end_at_once_as_if_nothing_was_written() {
    let database = database();
    // Gathered the way a request's effects gather them.
    let mut runs = TakenRuns::default();
    runs.extend(write_and_take(&database).await);
    assert_eq!(runs.iter().count(), 2);
    drop(runs);
    assert_ended_unmade(&database.try_read().unwrap());
}

#[tokio::test]
async fn taken_runs_dropped_while_the_database_is_busy_end_once_it_is_free() {
    let database = database();
    let runs = write_and_take(&database).await;
    let held = Arc::clone(&database).read_owned().await;
    drop(runs);
    assert!(in_process(&held));
    assert_eq!(write_status(&held), WriteStatus::IN_PROGRESS);
    drop(held);
    // The ending task runs as soon as this one yields.
    tokio::task::yield_now().await;
    assert_ended_unmade(&database.try_read().unwrap());
}

#[tokio::test]
async fn taken_runs_handed_over_belong_to_their_new_owner() {
    let database = database();
    let mut runs = write_and_take(&database).await;
    let split = runs.split_off(|run| run.source == ch1());
    let mut owned = Vec::new();
    runs.hand_over(|run| owned.push(run));
    drop(split);
    // CMD-1's run went on to its owner and is still in progress; CH-1's was
    // dropped with the split and ended.
    let db = database.try_read().unwrap();
    assert_eq!(owned.len(), 1);
    assert_eq!(owned[0].source, cmd1());
    assert!(in_process(&db));
    assert_eq!(write_status(&db), WriteStatus::FAILED);
}

#[tokio::test]
async fn an_owner_that_panics_part_way_leaves_the_runs_not_yet_handed_to_the_drop() {
    let database = database();
    let runs = write_and_take(&database).await;
    let mut owned = Vec::new();
    // The owner keeps CMD-1's run, then panics before CH-1's is handed over.
    let handing = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runs.hand_over(|run| {
            owned.push(run);
            panic!("the owner fails after taking its first run");
        })
    }));
    assert!(handing.is_err());
    let db = database.try_read().unwrap();
    assert_eq!(owned.len(), 1);
    assert_eq!(owned[0].source, cmd1());
    assert!(in_process(&db));
    assert_eq!(write_status(&db), WriteStatus::FAILED);
}

#[tokio::test]
async fn a_dropped_run_ends_once_and_never_ends_its_objects_next_run() {
    let database = database();
    let runs = write_and_take(&database).await;
    let stale: Vec<CommandRun> = runs.iter().cloned().collect();
    drop(runs);
    let mut db = database.write().await;
    assert_ended_unmade(&db);
    // Ending the same runs again changes nothing.
    for run in &stale {
        assert!(!Unfinished::start(run).end(&mut db));
    }
    // Nor does it touch the runs the next writes start.
    write_pv(&mut db, cmd1(), PropertyValue::Unsigned(1));
    write_pv(&mut db, ch1(), PropertyValue::Real(2.0));
    let next = TakenRuns::take(&database, &mut db, &[cmd1(), ch1()]);
    for run in &stale {
        assert!(!Unfinished::start(run).end(&mut db));
    }
    assert!(in_process(&db));
    assert_eq!(write_status(&db), WriteStatus::IN_PROGRESS);
    let again: Vec<CommandRun> = next.iter().cloned().collect();
    assert_eq!(next.end(&mut db), [cmd1(), ch1()]);
    assert_ended_unmade(&db);
    for run in &again {
        assert!(!Unfinished::start(run).end(&mut db));
    }
}
