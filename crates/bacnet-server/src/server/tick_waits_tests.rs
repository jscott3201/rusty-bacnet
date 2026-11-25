//! The application-driven ticks, `tick_schedules` and `poll_trend_logs`,
//! and the start-up Staging pass, each polled once and then left unpolled,
//! hold up nothing the server serves (#2781), parked as [`super`] parks the
//! local calls: the test holds the database, polls the call once, has the
//! peer send a WriteProperty that queues behind it, lets the database go,
//! and has the peer read the Device's Active_COV_Subscriptions. Polled at
//! last, the call returns what a call awaited normally returns.

use bacnet_objects::schedule::ScheduleObject;
use bacnet_types::constructed::BACnetObjectPropertyReference;

use super::*;

/// SCH-1, which writes AV-2's Present_Value at its Priority_For_Writing,
/// added beside the node's objects once the server's own first Schedule
/// pass has run.
async fn schedule_av2(node: &Node) -> ObjectIdentifier {
    // The server's 60-second Schedule task makes its first pass at once;
    // let it, so only the test's ticks evaluate SCH-1 from here.
    tokio::time::sleep(Duration::from_millis(10)).await;
    let av2 = oid(ObjectType::ANALOG_VALUE, 2);
    let mut schedule = ScheduleObject::new(1, "SCH-1", PropertyValue::Real(55.0)).unwrap();
    schedule
        .add_object_property_reference(BACnetObjectPropertyReference::new(
            av2,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ))
        .unwrap();
    let mut db = node.server.db.write().await;
    db.add(Box::new(
        AnalogValueObject::new(2, "Scheduled", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(schedule)).unwrap();
    av2
}

#[tokio::test(start_paused = true)]
async fn a_parked_tick_schedules_holds_up_no_request() {
    let mut node = node().await;
    let av2 = schedule_av2(&node).await;
    let db = Arc::clone(node.server.database());
    let mut held = hold(&node).await;
    // A default no pass has seen: only the parked tick's pass writes it.
    held.get_mut(&oid(ObjectType::SCHEDULE, 1))
        .unwrap()
        .write_property(
            PropertyIdentifier::SCHEDULE_DEFAULT,
            None,
            PropertyValue::Real(66.0),
            None,
        )
        .unwrap();
    let mut parked: Call<'_, ()> = Box::pin(crate::schedule::tick_schedules(&db));
    assert!(poll_once(&mut parked).is_pending());
    node.served_past(held).await;
    parked.await;
    assert_eq!(
        node.value(av2, PropertyIdentifier::PRESENT_VALUE).await,
        PropertyValue::Real(66.0)
    );
    node.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_parked_poll_trend_logs_holds_up_no_request() {
    let mut node = node().await;
    let db = Arc::clone(node.server.database());
    let awaited = crate::trend_log::poll_trend_logs(&db).await;
    let held = hold(&node).await;
    let mut parked: Call<'_, Duration> = Box::pin(crate::trend_log::poll_trend_logs(&db));
    assert!(poll_once(&mut parked).is_pending());
    node.served_past(held).await;
    assert_eq!(parked.await, awaited);
    node.server.stop().await.unwrap();
}

/// The pass `start()` makes of the Staging plans due at start-up, which
/// runs once the dispatch loop serves requests, made again on the running
/// server.
#[tokio::test(start_paused = true)]
async fn a_parked_start_up_staging_pass_holds_up_no_request() {
    let mut node = node().await;
    let held = hold(&node).await;
    let mut parked: Call<'_, ()> = Box::pin(node.server.execute_initial_staging_plans());
    assert!(poll_once(&mut parked).is_pending());
    node.served_past(held).await;
    parked.await;
    node.server.stop().await.unwrap();
}

/// The ticks' futures stay `Send` and `Sync`, and `start()`'s `Send` (a
/// transport's start was never `Sync`), though each may now hold a boxed
/// task across an await (#2781): checked when this compiles.
#[allow(dead_code)]
fn the_ticks_and_start_stay_send<T: TransportPort + 'static>(
    db: &Arc<RwLock<ObjectDatabase>>,
    config: ServerConfig,
    objects: ObjectDatabase,
    transport: T,
) {
    fn check(_: impl Send + Sync) {}
    fn check_send(_: impl Send) {}
    check(crate::schedule::tick_schedules(db));
    check(crate::trend_log::poll_trend_logs(db));
    check_send(BACnetServer::start(config, objects, transport));
}
