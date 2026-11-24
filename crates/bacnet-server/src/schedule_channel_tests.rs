//! `tick_schedules` without a server runs the Channel distributions its
//! writes start (#1151, #1178; Clause 12.53).
//!
//! SCH-1 drives CH-1's Present_Value and defaults to REAL 1.0. CH-1 writes
//! AO-1 at once and AO-2 after 500 ms. The clock is paused, so the delay
//! passes as soon as nothing else can run.

use std::time::Duration;

use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::channel::ChannelObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::schedule::ScheduleObject;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetObjectPropertyReference,
};
use bacnet_types::enums::{EngineeringUnits, WriteStatus};

use super::tests::SettableClock;
use super::*;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn ch(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::CHANNEL, instance)
}

fn ao(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::ANALOG_OUTPUT, instance)
}

fn channel(instance: u32, members: &[(ObjectIdentifier, u32)]) -> ChannelObject {
    let mut channel = ChannelObject::new(instance, format!("CH-{instance}"), 1).unwrap();
    let (references, delays): (Vec<_>, Vec<_>) = members
        .iter()
        .map(|&(object, delay)| {
            (
                BACnetDeviceObjectPropertyReference::new_local(
                    object,
                    PropertyIdentifier::PRESENT_VALUE.to_raw(),
                ),
                delay,
            )
        })
        .unzip();
    channel.set_members(references).unwrap();
    channel.set_execution_delay(delays).unwrap();
    channel
}

/// A database whose SCH-1 writes `target`'s Present_Value, with `objects`.
fn database(target: ObjectIdentifier, objects: Vec<ChannelObject>) -> Arc<RwLock<ObjectDatabase>> {
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
    for object in objects {
        db.add(Box::new(object)).unwrap();
    }
    let mut schedule = ScheduleObject::new(1, "SCH-1", PropertyValue::Real(1.0)).unwrap();
    schedule
        .add_object_property_reference(BACnetObjectPropertyReference::new(
            target,
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
) -> PropertyValue {
    db.read()
        .await
        .get(&object)
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

async fn write_status(db: &RwLock<ObjectDatabase>, instance: u32) -> WriteStatus {
    match read(db, ch(instance), PropertyIdentifier::WRITE_STATUS).await {
        PropertyValue::Enumerated(raw) => WriteStatus::from_raw(raw),
        other => panic!("Write_Status read {other:?}"),
    }
}

fn ch1() -> Vec<ChannelObject> {
    vec![channel(1, &[(ao(1), 0), (ao(2), 500)])]
}

#[tokio::test(start_paused = true)]
async fn tick_schedules_runs_the_channel_distributions_its_writes_start() {
    let db = database(ch(1), ch1());
    tick_schedules(&db).await;
    for target in [ao(1), ao(2)] {
        assert_eq!(
            read(&db, target, PropertyIdentifier::PRESENT_VALUE).await,
            PropertyValue::Real(1.0),
            "{target}"
        );
    }
    assert_eq!(write_status(&db, 1).await, WriteStatus::SUCCESSFUL);
}

#[tokio::test(start_paused = true)]
async fn tick_schedules_dropped_mid_distribution_ends_it_failed() {
    let db = database(ch(1), ch1());
    // The tick is dropped during AO-2's 500 ms delay.
    tokio::time::timeout(Duration::from_millis(100), tick_schedules(&db))
        .await
        .unwrap_err();
    tokio::task::yield_now().await;
    assert_eq!(
        read(&db, ao(1), PropertyIdentifier::PRESENT_VALUE).await,
        PropertyValue::Real(1.0)
    );
    assert_eq!(
        read(&db, ao(2), PropertyIdentifier::PRESENT_VALUE).await,
        PropertyValue::Real(0.0)
    );
    assert_eq!(write_status(&db, 1).await, WriteStatus::FAILED);
}

#[tokio::test(start_paused = true)]
async fn tick_schedules_stops_two_channels_naming_each_other() {
    // CH-5 writes CH-6 after 100 ms and CH-6 writes CH-5 after 100 ms: the
    // tick returns once CH-5's second run is refused.
    let db = database(
        ch(5),
        vec![channel(5, &[(ch(6), 100)]), channel(6, &[(ch(5), 100)])],
    );
    tokio::time::timeout(Duration::from_secs(10), tick_schedules(&db))
        .await
        .expect("the tick returned");
    assert_eq!(write_status(&db, 5).await, WriteStatus::FAILED);
    assert_eq!(write_status(&db, 6).await, WriteStatus::FAILED);
}

#[tokio::test(start_paused = true)]
async fn tick_schedules_dropped_with_a_run_queued_but_not_started_ends_it() {
    use futures_util::FutureExt;

    // CH-1 writes CH-2 at once, then waits a second for AO-1. CH-2's run is
    // queued by that write; the tick is dropped after its first poll, before
    // the queued run is taken up.
    let db = database(
        ch(1),
        vec![
            channel(1, &[(ch(2), 0), (ao(1), 1000)]),
            channel(2, &[(ao(2), 0)]),
        ],
    );
    assert!(tick_schedules(&db).now_or_never().is_none());
    assert_eq!(
        read(&db, ch(2), PropertyIdentifier::PRESENT_VALUE).await,
        PropertyValue::Real(1.0)
    );
    assert_eq!(write_status(&db, 2).await, WriteStatus::FAILED);
    assert_eq!(write_status(&db, 1).await, WriteStatus::FAILED);
    assert_eq!(
        read(&db, ao(2), PropertyIdentifier::PRESENT_VALUE).await,
        PropertyValue::Real(0.0)
    );
}
