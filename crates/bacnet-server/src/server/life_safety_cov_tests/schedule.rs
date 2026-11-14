use super::*;

use bacnet_objects::schedule::ScheduleObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::BACnetObjectPropertyReference;

async fn tick(db: &Arc<RwLock<ObjectDatabase>>) -> crate::committed_cov::CommittedCov {
    let table = RwLock::new(CovSubscriptionTable::new());
    crate::schedule::tick_schedules_committed(db, &table, &mut |_, _| false).await
}

#[tokio::test]
async fn live_schedule_retains_actual_life_safety_status_delta() {
    let mut schedule = ScheduleObject::new(1, "schedule", PropertyValue::Boolean(false)).unwrap();
    schedule
        .add_object_property_reference(BACnetObjectPropertyReference::new(
            point_oid(),
            PropertyIdentifier::OUT_OF_SERVICE.to_raw(),
        ))
        .unwrap();
    schedule
        .write_property(
            PropertyIdentifier::SCHEDULE_DEFAULT,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();

    let mut db = life_safety_db();
    db.add(Box::new(schedule)).unwrap();
    let db = Arc::new(RwLock::new(db));

    let committed = tick(&db).await;

    assert!(committed.coarse.is_empty());
    assert_eq!(
        committed.life_safety,
        vec![bacnet_device_core::cov::life_safety::LifeSafetyCovChange {
            object_identifier: point_oid(),
            changed_properties: vec![PropertyIdentifier::STATUS_FLAGS],
        }]
    );
    assert_eq!(
        db.read()
            .await
            .get(&point_oid())
            .unwrap()
            .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
            .unwrap(),
        PropertyValue::Boolean(true)
    );
}

struct FixedScheduleClock;
impl bacnet_objects::clock::ClockReader for FixedScheduleClock {
    fn read_clock(&self) -> Option<bacnet_objects::clock::ClockFrame> {
        Some(bacnet_objects::clock::ClockFrame {
            local_date: bacnet_types::primitives::Date {
                year: 126,
                month: 9,
                day: 21,
                day_of_week: 1,
            },
            local_time: bacnet_types::primitives::Time {
                hour: 12,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

#[tokio::test]
async fn schedule_indexed_target_and_later_unindexed_target_survive_failure() {
    use bacnet_objects::multistate::MultiStateOutputObject;
    let target = MultiStateOutputObject::new(7, "target", 2).unwrap();
    let target_oid = target.object_identifier();
    let mut schedule =
        ScheduleObject::new(2, "schedule", PropertyValue::CharacterString("Home".into())).unwrap();
    // An invalid index must not prevent either subsequent valid target write.
    for index in [3, 2] {
        schedule
            .add_object_property_reference(BACnetObjectPropertyReference::new_indexed(
                target_oid,
                PropertyIdentifier::STATE_TEXT.to_raw(),
                index,
            ))
            .unwrap();
    }
    schedule
        .add_object_property_reference(BACnetObjectPropertyReference::new(
            target_oid,
            PropertyIdentifier::DESCRIPTION.to_raw(),
        ))
        .unwrap();
    schedule
        .write_property(
            PropertyIdentifier::SCHEDULE_DEFAULT,
            None,
            PropertyValue::CharacterString("Away".into()),
            None,
        )
        .unwrap();
    let mut db = ObjectDatabase::new();
    db.set_clock_reader(Some(Arc::new(FixedScheduleClock)));
    db.add(Box::new(target)).unwrap();
    db.add(Box::new(schedule)).unwrap();
    let db = Arc::new(RwLock::new(db));
    // An ordinary target owes a whole-object COV fanout, not a Life Safety
    // delta. Index 3 is past the end of State_Text, a reference the target
    // can't write, so the Schedule's Reliability changes too (#1433).
    let schedule_oid = ObjectIdentifier::new(ObjectType::SCHEDULE, 2).unwrap();
    let committed = tick(&db).await;
    assert!(committed.life_safety.is_empty());
    assert_eq!(committed.coarse, vec![target_oid, schedule_oid]);
    let db = db.read().await;
    assert_eq!(
        db.get(&schedule_oid)
            .unwrap()
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(bacnet_types::enums::Reliability::CONFIGURATION_ERROR.to_raw())
    );
    let target = db.get(&target_oid).unwrap();
    assert_eq!(
        target
            .read_property(PropertyIdentifier::STATE_TEXT, Some(1))
            .unwrap(),
        PropertyValue::CharacterString("State 1".into())
    );
    assert_eq!(
        target
            .read_property(PropertyIdentifier::STATE_TEXT, Some(2))
            .unwrap(),
        PropertyValue::CharacterString("Away".into())
    );
    assert_eq!(
        target
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("Away".into())
    );
}

#[tokio::test]
async fn schedule_unindexed_command_retains_fixed_priority_sixteen() {
    use bacnet_objects::multistate::MultiStateOutputObject;
    let target = MultiStateOutputObject::new(8, "command target", 2).unwrap();
    let target_oid = target.object_identifier();
    let mut schedule =
        ScheduleObject::new(3, "command schedule", PropertyValue::Unsigned(1)).unwrap();
    schedule
        .add_object_property_reference(BACnetObjectPropertyReference::new(
            target_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ))
        .unwrap();
    schedule
        .write_property(
            PropertyIdentifier::SCHEDULE_DEFAULT,
            None,
            PropertyValue::Unsigned(2),
            None,
        )
        .unwrap();
    assert_eq!(
        schedule
            .read_property(PropertyIdentifier::PRIORITY_FOR_WRITING, None)
            .unwrap(),
        PropertyValue::Unsigned(16)
    );
    let mut db = ObjectDatabase::new();
    db.set_clock_reader(Some(Arc::new(FixedScheduleClock)));
    db.add(Box::new(
        bacnet_objects::device::DeviceObject::new(bacnet_objects::device::DeviceConfig::default())
            .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(target)).unwrap();
    db.add(Box::new(schedule)).unwrap();
    let db = Arc::new(RwLock::new(db));
    tick(&db).await;
    let db = db.read().await;
    let target = db.get(&target_oid).unwrap();
    assert_eq!(
        target
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(2)
    );
    let PropertyValue::ApplicationData(bytes) = target
        .read_property(PropertyIdentifier::VALUE_SOURCE, None)
        .unwrap()
    else {
        panic!("typed source")
    };
    let (source, consumed) = bacnet_encoding::constructed::decode_value_source(&bytes, 0).unwrap();
    assert_eq!(consumed, bytes.len());
    assert_eq!(
        source,
        bacnet_types::constructed::BACnetValueSource::Object(
            bacnet_types::constructed::BACnetDeviceObjectReference {
                device_identifier: None,
                object_identifier: ObjectIdentifier::new(ObjectType::SCHEDULE, 3).unwrap(),
            }
        )
    );
    for index in 1..=16 {
        assert_eq!(
            target
                .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(index))
                .unwrap(),
            if index == 16 {
                PropertyValue::Unsigned(2)
            } else {
                PropertyValue::Null
            }
        );
    }
}
