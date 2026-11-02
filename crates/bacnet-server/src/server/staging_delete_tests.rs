//! A Staging object that leaves the database gives up the Present_Value
//! slots its stage writes filled, as a Schedule does, but only those its
//! Value_Source still names (#2265).
//!
//! STG-5 has stages up to 10, 30 and 50 and commands BO-5 at priority 8:
//! INACTIVE in stage 1, ACTIVE in the others. Present_Value starts at 5, so
//! the server writes INACTIVE at start-up. The clock is paused.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::binary::BinaryOutputObject;
use bacnet_objects::staging::{StagingConfig, StagingObject};
use bacnet_services::object_mgmt::DeleteObjectRequest;
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};
use bacnet_types::enums::EngineeringUnits;

fn stg5() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::STAGING, 5).unwrap()
}

fn bo5() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 5).unwrap()
}

const INACTIVE: PropertyValue = PropertyValue::Enumerated(0);
const ACTIVE: PropertyValue = PropertyValue::Enumerated(1);

fn staging(present_value: f32) -> StagingObject {
    let stage = |limit: f32, active: bool| BACnetStageLimitValue {
        limit,
        values: vec![active],
        deadband: 1.0,
    };
    StagingObject::new(
        5,
        "STG-5",
        StagingConfig {
            present_value,
            min_present_value: 0.0,
            units: EngineeringUnits::DEGREES_CELSIUS,
            priority_for_writing: 8,
            stages: vec![stage(10.0, false), stage(30.0, true), stage(50.0, true)],
            target_references: vec![BACnetDeviceObjectReference {
                device_identifier: None,
                object_identifier: bo5(),
            }],
            stage_names: None,
        },
    )
    .unwrap()
}

async fn start() -> Harness {
    let h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(BinaryOutputObject::new(5, "BO-5").unwrap()))
            .unwrap();
        db.add(Box::new(staging(5.0))).unwrap();
    })
    .await;
    assert_eq!(slot8(&h).await, INACTIVE);
    h
}

async fn slot8(h: &Harness) -> PropertyValue {
    h.server
        .database()
        .read()
        .await
        .get(&bo5())
        .unwrap()
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
        .unwrap()
}

async fn delete(h: &mut Harness) {
    let mut body = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: stg5(),
    }
    .encode(&mut body);
    h.request(ConfirmedServiceChoice::DELETE_OBJECT, body).await;
    assert_eq!(response(h).await, Ok(()));
    assert!(h.server.database().read().await.get(&stg5()).is_none());
}

async fn write_local(h: &Harness, object: ObjectIdentifier, value: PropertyValue, at: Option<u8>) {
    h.server
        .write_local(
            &object,
            PV,
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
async fn deleting_a_staging_object_relinquishes_the_slot_it_holds() {
    let mut h = start().await;
    // A stage change writes the same slot again; one NULL gives it up.
    write_local(&h, stg5(), PropertyValue::Real(20.0), None).await;
    assert_eq!(slot8(&h).await, ACTIVE);
    delete(&mut h).await;
    assert_eq!(slot8(&h).await, PropertyValue::Null);
}

#[tokio::test(start_paused = true)]
async fn deleting_a_staging_object_keeps_a_slot_another_writer_took() {
    let mut h = start().await;
    // The server's Device takes priority 8 on BO-5 after STG-5 wrote it.
    write_local(&h, bo5(), ACTIVE, Some(8)).await;
    delete(&mut h).await;
    assert_eq!(slot8(&h).await, ACTIVE);
}

#[tokio::test(start_paused = true)]
async fn an_application_removing_a_staging_object_relinquishes_its_slot() {
    let h = start().await;
    h.server
        .database()
        .write()
        .await
        .remove(&stg5())
        .unwrap()
        .unwrap();
    h.settle().await;
    assert_eq!(slot8(&h).await, PropertyValue::Null);
}

#[tokio::test(start_paused = true)]
async fn a_removed_staging_objects_null_lands_before_its_replacement_writes() {
    // Before start: the old STG-5 selected stage 2 (so it holds BO-5's slot
    // 8) and was removed, queueing its NULL; a new STG-5 in stage 1 took its
    // place. Start-up writes the new one's INACTIVE, which names the same
    // identifier as the old NULL, so only the order keeps it.
    let h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(BinaryOutputObject::new(5, "BO-5").unwrap()))
            .unwrap();
        let mut old = staging(20.0);
        assert!(bacnet_objects::traits::StagingSource::take_staging_write_plan(&mut old).is_some());
        db.add(Box::new(old)).unwrap();
        db.remove(&stg5()).unwrap().unwrap();
        db.add(Box::new(staging(5.0))).unwrap();
    })
    .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(slot8(&h).await, INACTIVE);
}

#[tokio::test(start_paused = true)]
async fn replacing_a_staging_object_through_add_relinquishes_the_old_ones_slot() {
    let h = start().await;
    // The replacement starts from what it holds: nothing until it writes.
    h.server
        .database()
        .write()
        .await
        .add(Box::new(staging(5.0)))
        .unwrap();
    h.settle().await;
    assert_eq!(slot8(&h).await, PropertyValue::Null);
}
