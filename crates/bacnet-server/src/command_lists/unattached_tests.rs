//! Runs the chain check refuses are ended even when the wait for the
//! database is cut short, and as if none of their writes were made (#1151).
use super::*;
use crate::command_lists::admit;
use bacnet_objects::channel::ChannelObject;
use bacnet_objects::command::{CommandObject, RunPlan};
use bacnet_types::constructed::{
    BACnetActionCommand, BACnetActionList, BACnetDeviceObjectPropertyReference,
};
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier, WriteStatus};
use bacnet_types::primitives::PropertyValue;
use std::time::Duration;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// A run of `source` with no ancestors, standing for the writer.
fn parent(source: ObjectIdentifier) -> CommandRun {
    CommandRun {
        source,
        generation: 0,
        plan: RunPlan::Actions(Vec::new()),
        chain: Arc::from([]),
    }
}

fn write_pv(db: &mut ObjectDatabase, object: ObjectIdentifier, value: PropertyValue) {
    db.get_mut(&object)
        .unwrap()
        .write_property(PropertyIdentifier::PRESENT_VALUE, None, value, None)
        .unwrap();
}

/// The run `object`'s Present_Value write queued, taken as its writer takes
/// it.
async fn take(db: &Arc<RwLock<ObjectDatabase>>, object: ObjectIdentifier) -> TakenRuns {
    let runs = TakenRuns::take(db, &mut *db.write().await, &[object]);
    assert_eq!(runs.iter().count(), 1);
    runs
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

#[tokio::test(start_paused = true)]
async fn refused_run_dropped_while_waiting_for_the_database_still_ends() {
    let ch1 = oid(ObjectType::CHANNEL, 1);
    let mut channel = ChannelObject::new(1, "CH-1", 7).unwrap();
    channel
        .set_members(vec![BACnetDeviceObjectPropertyReference::new_local(
            oid(ObjectType::ANALOG_OUTPUT, 1),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )])
        .unwrap();
    let mut objects = ObjectDatabase::new();
    objects.add(Box::new(channel)).unwrap();
    write_pv(&mut objects, ch1, PropertyValue::Real(1.0));
    let db = Arc::new(RwLock::new(objects));
    let runs = take(&db, ch1).await;
    let (started, _queued) = mpsc::unbounded_channel();
    let host = Unattached { db: &db, started };
    // CH-1's run is queued by CH-1's own write, so it's refused; the wait
    // for the database to end it is cut short.
    let reader = db.read().await;
    let refused = tokio::time::timeout(
        Duration::from_millis(10),
        admit(&host, &parent(ch1), runs, |runs| assert!(runs.is_empty())),
    )
    .await;
    assert!(refused.is_err(), "the wait was cut short");
    drop(reader);
    tokio::time::sleep(Duration::from_millis(1)).await;
    let mut objects = db.write().await;
    assert_eq!(
        read(&objects, ch1, PropertyIdentifier::WRITE_STATUS),
        PropertyValue::Enumerated(WriteStatus::FAILED.to_raw())
    );
    // Nothing is left IN_PROGRESS, so the next write isn't BUSY.
    write_pv(&mut objects, ch1, PropertyValue::Real(2.0));
}

#[tokio::test]
async fn refused_command_run_marks_every_command_unsuccessful() {
    let cmd1 = oid(ObjectType::COMMAND, 1);
    let command = |value| BACnetActionCommand {
        device_identifier: None,
        object_identifier: oid(ObjectType::ANALOG_OUTPUT, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: PropertyValue::Real(value),
        priority: Some(8),
        post_delay: None,
        quit_on_failure: false,
        // Left over from an earlier run.
        write_successful: true,
    };
    let mut object = CommandObject::new(1, "CMD-1").unwrap();
    object
        .set_action(vec![BACnetActionList {
            commands: vec![command(1.0), command(2.0)],
        }])
        .unwrap();
    let mut objects = ObjectDatabase::new();
    objects.add(Box::new(object)).unwrap();
    write_pv(&mut objects, cmd1, PropertyValue::Unsigned(1));
    let db = Arc::new(RwLock::new(objects));
    let runs = take(&db, cmd1).await;
    let (started, _queued) = mpsc::unbounded_channel();
    let host = Unattached { db: &db, started };
    assert!(admit(&host, &parent(cmd1), runs, |_| {}).await.is_err());
    let objects = db.read().await;
    for property in [
        PropertyIdentifier::IN_PROCESS,
        PropertyIdentifier::ALL_WRITES_SUCCESSFUL,
    ] {
        assert_eq!(
            read(&objects, cmd1, property),
            PropertyValue::Boolean(false),
            "{property:?}"
        );
    }
    let PropertyValue::ApplicationData(octets) = objects
        .get(&cmd1)
        .unwrap()
        .read_property(PropertyIdentifier::ACTION, Some(1))
        .unwrap()
    else {
        panic!("Action element 1 is framed octets");
    };
    let (list, _) = bacnet_encoding::constructed::decode_action_list(&octets, 0).unwrap();
    assert_eq!(
        list.commands
            .iter()
            .map(|command| command.write_successful)
            .collect::<Vec<_>>(),
        [false, false]
    );
}

#[tokio::test]
async fn unattached_channel_member_in_another_device_fails_unsent() {
    // No network here: the member in Device 9 fails as a process error, and
    // the local one is still written.
    let ch1 = oid(ObjectType::CHANNEL, 1);
    let ao1 = oid(ObjectType::ANALOG_VALUE, 1);
    let local = BACnetDeviceObjectPropertyReference::new_local(
        ao1,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    );
    let remote = BACnetDeviceObjectPropertyReference {
        device_identifier: Some(oid(ObjectType::DEVICE, 9)),
        ..local.clone()
    };
    let mut channel = ChannelObject::new(1, "CH-1", 7).unwrap();
    channel.set_members(vec![remote, local]).unwrap();
    let mut objects = ObjectDatabase::new();
    let device = bacnet_objects::device::DeviceConfig::default();
    objects
        .add(Box::new(
            bacnet_objects::device::DeviceObject::new(device).unwrap(),
        ))
        .unwrap();
    objects.add(Box::new(channel)).unwrap();
    objects
        .add(Box::new(
            bacnet_objects::analog::AnalogValueObject::new(
                1,
                "AV-1",
                EngineeringUnits::DEGREES_CELSIUS,
            )
            .unwrap(),
        ))
        .unwrap();
    write_pv(&mut objects, ch1, PropertyValue::Real(4.0));
    let db = Arc::new(RwLock::new(objects));
    let runs = take(&db, ch1).await;
    run_unattached(&db, runs).await;
    let objects = db.read().await;
    assert_eq!(
        read(&objects, ch1, PropertyIdentifier::WRITE_STATUS),
        PropertyValue::Enumerated(WriteStatus::FAILED.to_raw())
    );
    assert_eq!(
        read(&objects, ch1, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(bacnet_types::enums::Reliability::PROCESS_ERROR.to_raw())
    );
    assert_eq!(
        read(&objects, ao1, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Real(4.0)
    );
}
