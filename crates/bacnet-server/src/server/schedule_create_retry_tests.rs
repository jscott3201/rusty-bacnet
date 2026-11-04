//! A Schedule's refused reference is retried as soon as the object it names
//! is created (#1440), not at the next 60-second pass: by CreateObject under
//! the request's own guard, and after the application's own `add` by the
//! task the database's waker wakes. The retry goes to the references naming
//! that object alone, and COV is owed for what it writes. Time is paused, so
//! the 60-second pass never runs here after the first one at start.

use super::cov_wire_test_support::*;
use super::*;
use crate::cov::CovSubscriptionTable;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_objects::schedule::ScheduleObject;
use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{ObjectType, Reliability};

fn sch1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::SCHEDULE, 1).unwrap()
}

fn msv(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, instance).unwrap()
}

/// A Multi-state Value, which a client can create, with three states.
fn multi_state_value(instance: u32) -> Box<MultiStateValueObject> {
    Box::new(MultiStateValueObject::new(instance, format!("MSV-{instance}"), 3).unwrap())
}

/// MSV-1, and SCH-1 commanding state 2 to MSV-1 and to MSV-9, which doesn't
/// exist.
fn with_schedule(db: &mut ObjectDatabase) {
    db.add(multi_state_value(1)).unwrap();
    let mut schedule = ScheduleObject::new(1, "SCH-1", PropertyValue::Unsigned(2)).unwrap();
    schedule
        .set_object_property_references(
            [msv(1), msv(9)]
                .map(|object| BACnetObjectPropertyReference::new(object, PV.to_raw()))
                .to_vec(),
        )
        .unwrap();
    db.add(Box::new(schedule)).unwrap();
}

async fn read_from(
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

async fn schedule_reliability(db: &RwLock<ObjectDatabase>) -> Reliability {
    match read_from(db, sch1(), PropertyIdentifier::RELIABILITY).await {
        PropertyValue::Enumerated(raw) => Reliability::from_raw(raw),
        other => panic!("Reliability read {other:?}"),
    }
}

/// Let the server run what it has ready, well short of the next pass.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(10)).await;
}

#[tokio::test(start_paused = true)]
async fn the_retry_writes_the_created_object_and_owes_cov_for_it() {
    let db = {
        let mut db = crate::server::clocked_test_database();
        db.add(Box::new(
            DeviceObject::new(DeviceConfig::default()).unwrap(),
        ))
        .unwrap();
        with_schedule(&mut db);
        Arc::new(RwLock::new(db))
    };
    crate::schedule::tick_schedules(&db).await;
    assert_eq!(
        schedule_reliability(&db).await,
        Reliability::CONFIGURATION_ERROR
    );
    let cov_table = RwLock::new(CovSubscriptionTable::new());
    let committed = {
        let mut db_w = db.write().await;
        db_w.add(multi_state_value(9)).unwrap();
        crate::membership::settle(&db, &mut db_w, &cov_table).await
    };
    // No pass ran, yet MSV-9 holds the value and the fault is gone; both
    // changed, so both are owed a COV pass. MSV-1, which took the value
    // already, isn't written again.
    assert_eq!(read_from(&db, msv(9), PV).await, PropertyValue::Unsigned(2));
    assert_eq!(
        schedule_reliability(&db).await,
        Reliability::NO_FAULT_DETECTED
    );
    assert_eq!(committed.coarse, [msv(9), sch1()]);
    // The queue is empty once taken.
    assert!(db.write().await.take_membership_work_internal().is_empty());
}

#[tokio::test(start_paused = true)]
async fn create_object_retries_the_refusal_before_its_answer() {
    let mut h = Harness::start_with(ServerConfig::default(), with_schedule).await;
    settle().await;
    let db = Arc::clone(h.server.database());
    assert_eq!(
        schedule_reliability(&db).await,
        Reliability::CONFIGURATION_ERROR
    );
    // Without the waker the background task never takes the queue, so only
    // the request's own settle, under its guard, can make the retry.
    db.write().await.set_membership_waker_internal(None);
    let mut body = BytesMut::new();
    CreateObjectRequest {
        object_specifier: ObjectSpecifier::Identifier(msv(9)),
        list_of_initial_values: Vec::new(),
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::CREATE_OBJECT, body).await;
    settle().await;
    assert_eq!(read_from(&db, msv(9), PV).await, PropertyValue::Unsigned(2));
    assert_eq!(
        schedule_reliability(&db).await,
        Reliability::NO_FAULT_DETECTED
    );
}

#[tokio::test(start_paused = true)]
async fn an_applications_own_add_is_retried_by_the_server_task() {
    let h = Harness::start_with(ServerConfig::default(), with_schedule).await;
    settle().await;
    let db = Arc::clone(h.server.database());
    assert_eq!(
        schedule_reliability(&db).await,
        Reliability::CONFIGURATION_ERROR
    );
    db.write().await.add(multi_state_value(9)).unwrap();
    settle().await;
    assert_eq!(read_from(&db, msv(9), PV).await, PropertyValue::Unsigned(2));
    assert_eq!(
        schedule_reliability(&db).await,
        Reliability::NO_FAULT_DETECTED
    );
}
