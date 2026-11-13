//! A Staging object whose Target_References or Priority_For_Writing changes
//! gives up the Present_Value slots the new configuration no longer writes,
//! as a Schedule does, but only those its Value_Source still names (#2289).
//!
//! STG-1 starts at 15.0, in its second stage, which drives BO-1 and BO-2
//! ACTIVE at priority 8; BO-3 is a spare target. The clock is paused.
use super::cov_wire_test_support::*;
use super::test_transport::TestTransport;
use super::*;
use bacnet_encoding::constructed::encode_device_object_reference;
use bacnet_objects::binary::BinaryOutputObject;
use bacnet_objects::staging::{StagingConfig, StagingObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::object_mgmt::DeleteObjectRequest;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};
use bacnet_types::enums::EngineeringUnits;

const TARGETS: PropertyIdentifier = PropertyIdentifier::TARGET_REFERENCES;
const PRIORITY: PropertyIdentifier = PropertyIdentifier::PRIORITY_FOR_WRITING;
const INACTIVE: PropertyValue = PropertyValue::Enumerated(0);
const ACTIVE: PropertyValue = PropertyValue::Enumerated(1);
const NULL: PropertyValue = PropertyValue::Null;

fn stg1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::STAGING, 1).unwrap()
}

fn bo(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, instance).unwrap()
}

fn local(object_identifier: ObjectIdentifier) -> BACnetDeviceObjectReference {
    BACnetDeviceObjectReference {
        device_identifier: None,
        object_identifier,
    }
}

/// The encoded Target_References array naming `instances`.
fn targets(instances: &[u32]) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    for instance in instances {
        encode_device_object_reference(&mut bytes, &local(bo(*instance)));
    }
    bytes.to_vec()
}

fn unsigned(value: u64) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut bytes, value);
    bytes.to_vec()
}

fn staging(present_value: f32, instances: &[u32]) -> StagingObject {
    let stage = |limit: f32, active: bool| BACnetStageLimitValue {
        limit,
        values: vec![active; instances.len()],
        deadband: 1.0,
    };
    let config = StagingConfig {
        present_value,
        min_present_value: 0.0,
        units: EngineeringUnits::DEGREES_CELSIUS,
        priority_for_writing: 8,
        stages: vec![stage(10.0, false), stage(20.0, true)],
        target_references: instances.iter().map(|i| local(bo(*i))).collect(),
        stage_names: None,
    };
    StagingObject::new(1, "STG-1", config).unwrap()
}

fn add_outputs(db: &mut ObjectDatabase) {
    for instance in 1..=3 {
        db.add(Box::new(
            BinaryOutputObject::new(instance, format!("BO-{instance}")).unwrap(),
        ))
        .unwrap();
    }
}

async fn start() -> Harness {
    let h = Harness::start_with(ServerConfig::default(), |db| {
        add_outputs(db);
        db.add(Box::new(staging(15.0, &[1, 2]))).unwrap();
    })
    .await;
    assert_slots(&h, 8, [ACTIVE, ACTIVE, NULL]).await;
    h
}

/// Slot `priority` of BO-1, BO-2 and BO-3's Priority_Array.
async fn assert_slots(h: &Harness, priority: u32, expected: [PropertyValue; 3]) {
    let db = h.server.database().read().await;
    let slots = [1, 2, 3].map(|instance| {
        db.get(&bo(instance))
            .unwrap()
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(priority))
            .unwrap()
    });
    assert_eq!(slots, expected, "slot {priority} of BO-1, BO-2, BO-3");
}

async fn write_property(h: &mut Harness, property: PropertyIdentifier, value: Vec<u8>) {
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: stg1(),
        property_identifier: property,
        property_array_index: None,
        property_value: value,
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
        .await;
    assert_eq!(response(h).await, Ok(()));
    h.settle().await;
}

/// One WritePropertyMultiple of `writes` to STG-1, in order.
async fn write_multiple(h: &mut Harness, writes: Vec<(PropertyIdentifier, Vec<u8>)>) {
    let mut body = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: stg1(),
            list_of_properties: writes
                .into_iter()
                .map(|(property_identifier, value)| BACnetPropertyValue {
                    property_identifier,
                    property_array_index: None,
                    value,
                    priority: None,
                })
                .collect(),
        }],
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, body)
        .await;
    assert_eq!(response(h).await, Ok(()));
    h.settle().await;
}

async fn write_local(
    h: &Harness,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
    at: Option<u8>,
) {
    h.server
        .write_local(
            &object,
            property,
            None,
            value,
            at,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    h.settle().await;
}

#[tokio::test(start_paused = true)]
async fn a_dropped_target_is_relinquished_and_a_kept_one_is_left_alone() {
    let mut h = start().await;
    write_property(&mut h, TARGETS, targets(&[3, 2])).await;
    assert_slots(&h, 8, [NULL, ACTIVE, ACTIVE]).await;
}

#[tokio::test(start_paused = true)]
async fn uninitialized_references_hold_nothing_so_every_slot_is_relinquished() {
    let mut h = start().await;
    // Both members uninitialized (Clause 12.62.14): the plan writes nothing,
    // and its source's NULLs still go out.
    let mut bytes = BytesMut::new();
    let unset =
        ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, ObjectIdentifier::MAX_INSTANCE).unwrap();
    for _ in 0..2 {
        encode_device_object_reference(&mut bytes, &local(unset));
    }
    write_property(&mut h, TARGETS, bytes.to_vec()).await;
    assert_slots(&h, 8, [NULL, NULL, NULL]).await;
}

#[tokio::test(start_paused = true)]
async fn a_new_priority_relinquishes_every_slot_at_the_old_one() {
    let mut h = start().await;
    write_property(&mut h, PRIORITY, unsigned(12)).await;
    assert_slots(&h, 8, [NULL, NULL, NULL]).await;
    assert_slots(&h, 12, [ACTIVE, ACTIVE, NULL]).await;
}

#[tokio::test(start_paused = true)]
async fn a_slot_another_writer_took_is_kept() {
    let mut h = start().await;
    // The server's Device takes priority 8 on BO-1 after STG-1 wrote it.
    write_local(&h, bo(1), PV, INACTIVE, Some(8)).await;
    write_property(&mut h, TARGETS, targets(&[3, 2])).await;
    assert_slots(&h, 8, [INACTIVE, ACTIVE, ACTIVE]).await;
}

#[tokio::test(start_paused = true)]
async fn a_reference_and_a_priority_change_in_one_request_release_each_old_slot() {
    let mut h = start().await;
    write_multiple(
        &mut h,
        vec![(TARGETS, targets(&[3, 2])), (PRIORITY, unsigned(12))],
    )
    .await;
    assert_slots(&h, 8, [NULL, NULL, NULL]).await;
    assert_slots(&h, 12, [NULL, ACTIVE, ACTIVE]).await;
}

#[tokio::test(start_paused = true)]
async fn unchanged_writes_release_nothing() {
    let mut h = start().await;
    write_multiple(
        &mut h,
        vec![(TARGETS, targets(&[1, 2])), (PRIORITY, unsigned(8))],
    )
    .await;
    // The same targets swapped: both are still written at 8.
    write_property(&mut h, TARGETS, targets(&[2, 1])).await;
    assert_slots(&h, 8, [ACTIVE, ACTIVE, NULL]).await;
}

#[tokio::test(start_paused = true)]
async fn a_target_dropped_and_put_back_in_one_request_keeps_its_command() {
    let mut h = start().await;
    write_multiple(
        &mut h,
        vec![(TARGETS, targets(&[3, 2])), (TARGETS, targets(&[1, 2]))],
    )
    .await;
    assert_slots(&h, 8, [ACTIVE, ACTIVE, NULL]).await;
}

#[tokio::test(start_paused = true)]
async fn out_of_service_the_targets_keep_their_slots_until_it_returns() {
    let mut h = start().await;
    write_local(
        &h,
        stg1(),
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
        None,
    )
    .await;
    write_property(&mut h, PRIORITY, unsigned(12)).await;
    assert_slots(&h, 8, [ACTIVE, ACTIVE, NULL]).await;
    write_local(
        &h,
        stg1(),
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(false),
        None,
    )
    .await;
    assert_slots(&h, 8, [NULL, NULL, NULL]).await;
    assert_slots(&h, 12, [ACTIVE, ACTIVE, NULL]).await;
}

#[tokio::test(start_paused = true)]
async fn deleting_it_after_an_out_of_service_change_relinquishes_the_dropped_slot() {
    let mut h = start().await;
    write_local(
        &h,
        stg1(),
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
        None,
    )
    .await;
    write_property(&mut h, TARGETS, targets(&[3, 2])).await;
    let mut body = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: stg1(),
    }
    .encode(&mut body);
    h.request(ConfirmedServiceChoice::DELETE_OBJECT, body).await;
    assert_eq!(response(&h).await, Ok(()));
    h.settle().await;
    assert_slots(&h, 8, [NULL, NULL, NULL]).await;
}

#[tokio::test(start_paused = true)]
async fn a_removed_objects_owed_null_lands_before_its_replacement_writes() {
    // Before start: the old STG-1 wrote BO-1 and BO-2 at 8, then dropped
    // BO-1 out of service, so it still owes BO-1's NULL as it is removed.
    // The new STG-1, in stage 1, commands BO-1 INACTIVE at 8 at start-up
    // under the same identifier, so only the order keeps its command.
    let h = Harness::start_with(ServerConfig::default(), |db| {
        use bacnet_objects::traits::{BACnetObject, StagingSource};
        add_outputs(db);
        let mut old = staging(15.0, &[1, 2]);
        assert!(old.take_staging_write_plan().is_some());
        old.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        old.write_property(
            TARGETS,
            None,
            PropertyValue::List(
                [3, 2]
                    .map(|i| {
                        let mut bytes = BytesMut::new();
                        encode_device_object_reference(&mut bytes, &local(bo(i)));
                        PropertyValue::ApplicationData(bytes.to_vec())
                    })
                    .to_vec(),
            ),
            None,
        )
        .unwrap();
        db.add(Box::new(old)).unwrap();
        db.remove(&stg1()).unwrap().unwrap();
        db.add(Box::new(staging(5.0, &[1]))).unwrap();
    })
    .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let slot = h
        .server
        .database()
        .read()
        .await
        .get(&bo(1))
        .unwrap()
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
        .unwrap();
    assert_eq!(slot, INACTIVE);
}

/// STG-1 queues a plan for stage 1 and is edited behind it, so the plan is
/// stale at its first guard: `edit` runs on the object under the same guard.
async fn run_stale_plan(h: &Harness, edit: impl FnOnce(&mut dyn BACnetObject)) {
    let plans = {
        let mut db = h.server.database().write().await;
        let object = db.get_mut(&stg1()).unwrap();
        object
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(5.0),
                None,
            )
            .unwrap();
        let plans = BACnetServer::<TestTransport>::take_staging_plans(&mut db, &[stg1()]);
        assert_eq!(plans.len(), 1);
        edit(db.get_mut(&stg1()).unwrap());
        plans
    };
    let _runs = BACnetServer::<TestTransport>::execute_staging_plans(
        &h.server.local_event_delivery(),
        &h.server.local_cov_context(),
        plans,
        None,
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_stale_plans_guard_still_sends_the_nulls_owed() {
    let h = start().await;
    run_stale_plan(&h, |object| {
        object
            .write_property(PRIORITY, None, PropertyValue::Unsigned(12), None)
            .unwrap();
    })
    .await;
    assert_slots(&h, 8, [NULL, NULL, NULL]).await;
}

#[tokio::test(start_paused = true)]
async fn a_stale_plans_guard_sends_nothing_while_out_of_service() {
    let h = start().await;
    run_stale_plan(&h, |object| {
        object
            .write_property(
                PropertyIdentifier::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(true),
                None,
            )
            .unwrap();
        object
            .write_property(PRIORITY, None, PropertyValue::Unsigned(12), None)
            .unwrap();
    })
    .await;
    assert_slots(&h, 8, [ACTIVE, ACTIVE, NULL]).await;
}
