//! `tick_schedules` without a server runs the Command lists its writes start
//! (#1178, Clause 12.10).
//!
//! SCH-1 drives CMD-1's Present_Value and defaults to 1. CMD-1's list 1
//! writes AO-1 to 50.0 at priority 8 and waits 5 seconds, then writes 1 to
//! CMD-2's Present_Value. CMD-2's list 1 writes AO-9, which doesn't exist,
//! then AO-2 to 30.0 at priority 8. Every command starts with its
//! write-successful flag TRUE. The clock is paused, so the post delay passes
//! as soon as nothing else can run.

use std::time::Duration;

use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::command::CommandObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::schedule::ScheduleObject;
use bacnet_types::constructed::{
    BACnetActionCommand, BACnetActionList, BACnetObjectPropertyReference,
};

use super::tests::SettableClock;
use super::*;
use bacnet_types::enums::EngineeringUnits;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn cmd(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::COMMAND, instance)
}

fn ao(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::ANALOG_OUTPUT, instance)
}

fn write(object: ObjectIdentifier, value: PropertyValue) -> BACnetActionCommand {
    BACnetActionCommand {
        device_identifier: None,
        object_identifier: object,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: value,
        priority: Some(8),
        post_delay: None,
        quit_on_failure: false,
        write_successful: true,
    }
}

fn command(instance: u32, commands: Vec<BACnetActionCommand>) -> Box<CommandObject> {
    let mut command = CommandObject::new(instance, format!("CMD-{instance}")).unwrap();
    command
        .set_action(vec![BACnetActionList { commands }])
        .unwrap();
    Box::new(command)
}

fn database() -> Arc<RwLock<ObjectDatabase>> {
    let mut db = ObjectDatabase::new();
    db.set_clock_reader(Some(SettableClock::at(2026, 10, 3, 12, 0)));
    db.add(Box::new(
        DeviceObject::new(DeviceConfig::default()).unwrap(),
    ))
    .unwrap();
    for instance in [1, 2] {
        db.add(Box::new(
            AnalogOutputObject::new(
                instance,
                format!("AO-{instance}"),
                EngineeringUnits::DEGREES_CELSIUS,
            )
            .unwrap(),
        ))
        .unwrap();
    }
    let delayed = BACnetActionCommand {
        post_delay: Some(5),
        ..write(ao(1), PropertyValue::Real(50.0))
    };
    let start_cmd2 = BACnetActionCommand {
        priority: None,
        ..write(cmd(2), PropertyValue::Unsigned(1))
    };
    db.add(command(1, vec![delayed, start_cmd2])).unwrap();
    db.add(command(
        2,
        vec![
            write(ao(9), PropertyValue::Real(1.0)),
            write(ao(2), PropertyValue::Real(30.0)),
        ],
    ))
    .unwrap();
    let mut schedule = ScheduleObject::new(1, "SCH-1", PropertyValue::Unsigned(1)).unwrap();
    schedule
        .add_object_property_reference(BACnetObjectPropertyReference::new(
            cmd(1),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ))
        .unwrap();
    db.add(Box::new(schedule)).unwrap();
    Arc::new(RwLock::new(db))
}

async fn read(
    db: &RwLock<ObjectDatabase>,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> PropertyValue {
    db.read()
        .await
        .get(&object)
        .unwrap()
        .read_property(property, index)
        .unwrap()
}

async fn slot8(db: &RwLock<ObjectDatabase>, object: ObjectIdentifier) -> PropertyValue {
    read(db, object, PropertyIdentifier::PRIORITY_ARRAY, Some(8)).await
}

/// In_Process, All_Writes_Successful and list 1's write-successful flags.
async fn state(db: &RwLock<ObjectDatabase>, command: ObjectIdentifier) -> (bool, bool, Vec<bool>) {
    let flag = |value| match value {
        PropertyValue::Boolean(value) => value,
        other => panic!("{other:?}"),
    };
    let in_process = flag(read(db, command, PropertyIdentifier::IN_PROCESS, None).await);
    let all = flag(read(db, command, PropertyIdentifier::ALL_WRITES_SUCCESSFUL, None).await);
    let PropertyValue::ApplicationData(bytes) =
        read(db, command, PropertyIdentifier::ACTION, Some(1)).await
    else {
        panic!("Action element 1 is framed bytes");
    };
    let (list, _) = bacnet_encoding::constructed::decode_action_list(&bytes, 0).unwrap();
    let flags = list.commands.iter().map(|c| c.write_successful).collect();
    (in_process, all, flags)
}

#[tokio::test(start_paused = true)]
async fn tick_schedules_runs_the_command_lists_its_writes_start() {
    let db = database();
    tick_schedules(&db).await;
    assert_eq!(slot8(&db, ao(1)).await, PropertyValue::Real(50.0));
    assert_eq!(state(&db, cmd(1)).await, (false, true, vec![true, true]));
    // CMD-1's run started CMD-2's, which ran to its end beside it: the
    // missing AO-9 failed and AO-2 was still written.
    assert_eq!(
        read(&db, cmd(2), PropertyIdentifier::PRESENT_VALUE, None).await,
        PropertyValue::Unsigned(1)
    );
    assert_eq!(slot8(&db, ao(2)).await, PropertyValue::Real(30.0));
    assert_eq!(state(&db, cmd(2)).await, (false, false, vec![false, true]));
}

#[tokio::test(start_paused = true)]
async fn tick_schedules_dropped_mid_run_ends_the_command_run_unsuccessful() {
    let db = database();
    // The tick is dropped during CMD-1's 5-second post delay.
    tokio::time::timeout(Duration::from_secs(1), tick_schedules(&db))
        .await
        .unwrap_err();
    assert_eq!(slot8(&db, ao(1)).await, PropertyValue::Real(50.0));
    assert_eq!(state(&db, cmd(1)).await, (false, false, vec![true, false]));
    assert_eq!(
        read(&db, cmd(2), PropertyIdentifier::PRESENT_VALUE, None).await,
        PropertyValue::Unsigned(0)
    );
}
