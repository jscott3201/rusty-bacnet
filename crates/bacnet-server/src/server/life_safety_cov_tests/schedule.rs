use super::*;

use bacnet_objects::schedule::ScheduleObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::BACnetObjectPropertyReference;

#[tokio::test]
async fn live_schedule_retains_actual_life_safety_status_delta() {
    let mut schedule = ScheduleObject::new(1, "schedule", PropertyValue::Boolean(false)).unwrap();
    schedule.add_object_property_reference(BACnetObjectPropertyReference::new(
        point_oid(),
        PropertyIdentifier::OUT_OF_SERVICE.to_raw(),
    ));
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

    let changes = crate::schedule::tick_schedules_with_life_safety_cov(&db, 0).await;

    assert_eq!(
        changes,
        vec![crate::life_safety_cov::LifeSafetyCovChange {
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
