//! The slots a Staging object's plans fill, and the NULLs its removal owes
//! for them (#2265).

use super::*;
use crate::database::ObjectDatabase;
use crate::traits::StagingSource;
use bacnet_types::constructed::BACnetStageLimitValue;
use bacnet_types::enums::EngineeringUnits;

fn bo(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(bacnet_types::enums::ObjectType::BINARY_OUTPUT, instance).unwrap()
}

fn staging() -> super::super::StagingObject {
    let stage = |limit: f32, values: Vec<bool>| BACnetStageLimitValue {
        limit,
        values,
        deadband: 1.0,
    };
    let reference = |object_identifier| bacnet_types::constructed::BACnetDeviceObjectReference {
        device_identifier: None,
        object_identifier,
    };
    super::super::StagingObject::new(
        1,
        "STG-1",
        super::super::StagingConfig {
            present_value: 5.0,
            min_present_value: 0.0,
            units: EngineeringUnits::DEGREES_CELSIUS,
            priority_for_writing: 8,
            stages: vec![
                stage(10.0, vec![false, true]),
                stage(20.0, vec![true, false]),
            ],
            target_references: vec![reference(bo(1)), reference(bo(2))],
            stage_names: None,
        },
    )
    .unwrap()
}

fn null_to(priority: u8, targets: &[u32]) -> ScheduleWrite {
    let references = targets
        .iter()
        .map(|instance| {
            BACnetObjectPropertyReference::new(
                bo(*instance),
                PropertyIdentifier::PRESENT_VALUE.to_raw(),
            )
        })
        .collect();
    ScheduleWrite::new(PropertyValue::Null, priority, references).with_relinquish(true)
}

fn write(
    object: &mut super::super::StagingObject,
    property: PropertyIdentifier,
    value: PropertyValue,
) {
    crate::traits::BACnetObject::write_property(object, property, None, value, None).unwrap();
}

#[test]
fn a_staging_object_that_never_wrote_owes_nothing() {
    assert!(staging().take_relinquish_writes().is_empty());
}

#[test]
fn each_slot_written_is_owed_one_null_per_priority_and_only_once() {
    let mut object = staging();
    // Stage 1, then stage 2: the same two slots at 8, recorded once each.
    assert!(object.take_staging_write_plan().is_some());
    write(
        &mut object,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Real(15.0),
    );
    // A new priority leaves the slots at 8 held and fills those at 12.
    write(
        &mut object,
        PropertyIdentifier::PRIORITY_FOR_WRITING,
        PropertyValue::Unsigned(12),
    );
    assert_eq!(
        object.take_relinquish_writes(),
        vec![null_to(8, &[1, 2]), null_to(12, &[1, 2])]
    );
    assert!(object.take_relinquish_writes().is_empty());
    // The plan still queued is dropped with them.
    assert!(object.take_staging_write_plan().is_none());
}

#[test]
fn removing_a_staging_object_queues_its_nulls_and_wakes_the_server() {
    let mut db = ObjectDatabase::new();
    let mut object = staging();
    assert!(object.take_staging_write_plan().is_some());
    let oid = crate::traits::BACnetObject::object_identifier(&object);
    db.add(Box::new(object)).unwrap();
    let woken = alloc::sync::Arc::new(core::sync::atomic::AtomicUsize::new(0));
    let counter = alloc::sync::Arc::clone(&woken);
    db.set_membership_waker_internal(Some(alloc::sync::Arc::new(move || {
        counter.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    })));
    db.remove(&oid).unwrap().unwrap();
    assert_eq!(woken.load(core::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(
        db.take_membership_work_internal().relinquishes,
        vec![(oid, null_to(8, &[1, 2]))]
    );
}

#[test]
fn a_replacement_never_shares_a_plan_generation_with_the_object_it_replaced() {
    let old = staging().take_staging_write_plan().unwrap();
    let new = staging().take_staging_write_plan().unwrap();
    assert_ne!(old.generation, new.generation);
}
