//! The slots a Staging object's plans fill, and the NULLs its removal
//! (#2265) or a change of its targets or priority (#2289) owes for them.

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
    // A new priority owes the slots at 8 a NULL and fills those at 12.
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

fn reference_list(targets: &[u32]) -> PropertyValue {
    PropertyValue::List(
        targets
            .iter()
            .map(|instance| {
                let mut bytes = bytes::BytesMut::new();
                bacnet_encoding::constructed::encode_device_object_reference(
                    &mut bytes,
                    &bacnet_types::constructed::BACnetDeviceObjectReference {
                        device_identifier: None,
                        object_identifier: bo(*instance),
                    },
                );
                PropertyValue::ApplicationData(bytes.to_vec())
            })
            .collect(),
    )
}

/// The object after its start-up plan: BO-1 and BO-2 held at 8.
fn written() -> super::super::StagingObject {
    let mut object = staging();
    assert!(object.take_staging_write_plan().is_some());
    object
}

#[test]
fn a_dropped_target_is_owed_a_null_and_a_kept_one_stays_held() {
    let mut object = written();
    write(
        &mut object,
        PropertyIdentifier::TARGET_REFERENCES,
        reference_list(&[3, 2]),
    );
    assert_eq!(object.take_owed_relinquishes(), vec![null_to(8, &[1])]);
    assert!(object.take_owed_relinquishes().is_empty());
    // Removal now owes BO-2, still held, and BO-3, the new plan's.
    assert_eq!(object.take_relinquish_writes(), vec![null_to(8, &[2, 3])]);
}

#[test]
fn a_new_priority_owes_every_slot_at_the_old_one() {
    let mut object = written();
    write(
        &mut object,
        PropertyIdentifier::PRIORITY_FOR_WRITING,
        PropertyValue::Unsigned(12),
    );
    assert_eq!(object.take_owed_relinquishes(), vec![null_to(8, &[1, 2])]);
    assert_eq!(object.take_relinquish_writes(), vec![null_to(12, &[1, 2])]);
}

#[test]
fn a_priority_and_a_reference_change_together_owe_each_old_slot_once() {
    let mut object = written();
    write(
        &mut object,
        PropertyIdentifier::TARGET_REFERENCES,
        reference_list(&[3, 2]),
    );
    write(
        &mut object,
        PropertyIdentifier::PRIORITY_FOR_WRITING,
        PropertyValue::Unsigned(12),
    );
    assert_eq!(
        object.take_owed_relinquishes(),
        vec![null_to(8, &[1, 2, 3])]
    );
}

#[test]
fn unchanged_writes_and_reordered_references_owe_nothing() {
    let mut object = written();
    write(
        &mut object,
        PropertyIdentifier::TARGET_REFERENCES,
        reference_list(&[1, 2]),
    );
    write(
        &mut object,
        PropertyIdentifier::PRIORITY_FOR_WRITING,
        PropertyValue::Unsigned(8),
    );
    // The same targets in a new order: each is still written at 8.
    write(
        &mut object,
        PropertyIdentifier::TARGET_REFERENCES,
        reference_list(&[2, 1]),
    );
    assert!(object.take_owed_relinquishes().is_empty());
}

#[test]
fn a_slot_written_again_before_its_null_is_sent_is_no_longer_owed() {
    let mut object = written();
    // BO-1 dropped, then put back: its NULL would follow the new write.
    write(
        &mut object,
        PropertyIdentifier::TARGET_REFERENCES,
        reference_list(&[3, 2]),
    );
    write(
        &mut object,
        PropertyIdentifier::TARGET_REFERENCES,
        reference_list(&[1, 2]),
    );
    assert_eq!(object.take_owed_relinquishes(), vec![null_to(8, &[3])]);
}

/// Out of service, then moved to priority 12.
fn moved_out_of_service() -> super::super::StagingObject {
    let mut object = written();
    write(
        &mut object,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    );
    write(
        &mut object,
        PropertyIdentifier::PRIORITY_FOR_WRITING,
        PropertyValue::Unsigned(12),
    );
    object
}

#[test]
fn out_of_service_the_nulls_wait_for_the_return_to_service() {
    let mut object = moved_out_of_service();
    assert!(object.take_owed_relinquishes().is_empty());
    // Back in service the NULLs go out, ahead of the writes at 12.
    write(
        &mut object,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(false),
    );
    assert_eq!(object.take_owed_relinquishes(), vec![null_to(8, &[1, 2])]);
}

#[test]
fn removal_out_of_service_takes_the_nulls_still_owed() {
    let mut object = moved_out_of_service();
    assert_eq!(object.take_relinquish_writes(), vec![null_to(8, &[1, 2])]);
    assert!(object.take_owed_relinquishes().is_empty());
}
