//! DeleteObject of a Schedule on a running server (#2256): the slots it holds
//! on its targets are relinquished under the request's guard, at the
//! priority it wrote them, and COV goes out for each target once the guard
//! is dropped. A Schedule that holds nothing writes nothing, and a target
//! that refuses its NULL doesn't stop the delete.
//!
//! The harness is the one in `schedule_write_tests`: Tuesday 29 September
//! 2026, 15:00; SCH-5 commands AV-1's Present_Value at priority 16 and
//! defaults to 10.0, written at start-up; the COV subscription watches AV-1,
//! whose Relinquish_Default is 0.0.
use super::cov_wire_test_support::*;
use super::schedule_reference_write_tests::slot;
use super::schedule_write_tests::{read, sch5, schedule};
use super::*;
use bacnet_objects::channel::ChannelObject;
use bacnet_objects::schedule::ScheduleObject;
use bacnet_services::object_mgmt::DeleteObjectRequest;
use bacnet_types::calendar::SpecificDate;
use bacnet_types::constructed::{
    BACnetDateRange, BACnetDeviceObjectPropertyReference, BACnetObjectPropertyReference,
};
use bacnet_types::enums::ObjectType;

fn sch6() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::SCHEDULE, 6).unwrap()
}

/// An Analog Value the database doesn't hold.
fn av9() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 9).unwrap()
}

/// SCH-5 as the harness builds it, with `also` ahead of AV-1 in its references.
fn schedule_also(also: ObjectIdentifier) -> impl FnOnce(&mut ObjectDatabase) {
    move |db| {
        let mut schedule = ScheduleObject::new(5, "SCH-5", PropertyValue::Real(10.0)).unwrap();
        for oid in [also, av1()] {
            schedule
                .add_object_property_reference(BACnetObjectPropertyReference::new(oid, PV.to_raw()))
                .unwrap();
        }
        db.add(Box::new(schedule)).unwrap();
    }
}

/// SCH-6, a seasonal partner of SCH-5: AV-1 at the same priority 16, but its
/// Effective_Period ended in August 2026, so it has never written.
fn out_of_season(db: &mut ObjectDatabase) {
    let mut partner = ScheduleObject::new(6, "SCH-6", PropertyValue::Real(55.0)).unwrap();
    partner
        .add_object_property_reference(BACnetObjectPropertyReference::new(av1(), PV.to_raw()))
        .unwrap();
    partner
        .set_effective_period(BACnetDateRange {
            start_date: SpecificDate::new(2026, 6, 1).unwrap().to_date(),
            end_date: SpecificDate::new(2026, 8, 31).unwrap().to_date(),
        })
        .unwrap();
    db.add(Box::new(partner)).unwrap();
}

async fn start_with(extend: impl FnOnce(&mut ObjectDatabase)) -> Harness {
    let mut h = Harness::start_with(ServerConfig::default(), extend).await;
    h.subscribe_cov().await;
    assert_eq!(
        h.cov_notification().await.list_of_values[0].value,
        real(10.0)
    );
    h
}

async fn delete(h: &mut Harness, oid: ObjectIdentifier) {
    let mut body = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: oid,
    }
    .encode(&mut body);
    h.request(ConfirmedServiceChoice::DELETE_OBJECT, body).await;
}

async fn assert_gone(h: &Harness, oid: ObjectIdentifier) {
    assert!(h.server.database().read().await.get(&oid).is_none());
}

#[tokio::test(start_paused = true)]
async fn deleting_a_schedule_relinquishes_its_target_and_fans_cov_out() {
    let mut h = start_with(schedule).await;
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Real(10.0));
    delete(&mut h, sch5()).await;
    response(&h).await.unwrap();
    assert_gone(&h, sch5()).await;
    // AV-1's slot 16 is empty again: it falls to Relinquish_Default.
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Null);
    assert_eq!(read(&h, av1(), PV).await, PropertyValue::Real(0.0));
    let notification = h.cov_notification().await;
    assert_eq!(notification.monitored_object_identifier, av1());
    assert_eq!(notification.list_of_values[0].value, real(0.0));
    h.no_notification().await;
}

#[tokio::test(start_paused = true)]
async fn the_relinquish_commits_with_the_delete_and_cov_waits_for_the_guard() {
    let mut h = start_with(schedule).await;
    let (entered, release) = h.hold_notification_barrier(true);
    delete(&mut h, sch5()).await;
    tokio::time::timeout(Duration::from_secs(5), entered.acquire())
        .await
        .expect("AV-1's notification")
        .unwrap()
        .forget();
    // The notification is in the transport: the database guard is free,
    // and what it guards shows the Schedule gone and its slot empty at once.
    {
        let db = h
            .server
            .database()
            .try_write()
            .expect("no database guard held while COV is sent");
        assert!(db.get(&sch5()).is_none());
        assert_eq!(
            db.get(&av1())
                .unwrap()
                .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(16))
                .unwrap(),
            PropertyValue::Null
        );
    }
    assert!(h.server.cov_table.try_write().is_ok());
    release.add_permits(1);
    response(&h).await.unwrap();
    let notification = h.cov_notification().await;
    assert_eq!(notification.list_of_values[0].value, real(0.0));
}

#[tokio::test(start_paused = true)]
async fn a_schedule_that_never_wrote_relinquishes_nothing() {
    // SCH-6 shares AV-1's slot 16 with SCH-5 but has stayed out of season:
    // deleting it must not clear SCH-5's command.
    let mut h = start_with(|db| {
        schedule(db);
        out_of_season(db);
    })
    .await;
    delete(&mut h, sch6()).await;
    response(&h).await.unwrap();
    assert_gone(&h, sch6()).await;
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Real(10.0));
    assert_eq!(read(&h, av1(), PV).await, PropertyValue::Real(10.0));
    h.no_notification().await;
}

#[tokio::test(start_paused = true)]
async fn a_target_that_refuses_the_null_does_not_stop_the_delete() {
    // AV-9 doesn't exist, so its NULL fails, as its value did; AV-1 is
    // relinquished all the same and the delete succeeds.
    let mut h = start_with(schedule_also(av9())).await;
    delete(&mut h, sch5()).await;
    response(&h).await.unwrap();
    assert_gone(&h, sch5()).await;
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Null);
    let notification = h.cov_notification().await;
    assert_eq!(notification.list_of_values[0].value, real(0.0));
}

/// SCH-5 commanding CH-1's Present_Value at priority 16, and CH-1 passing
/// what it holds on to AV-1 at once.
fn through_a_channel(db: &mut ObjectDatabase) {
    let ch1 = ObjectIdentifier::new(ObjectType::CHANNEL, 1).unwrap();
    let mut channel = ChannelObject::new(1, "CH-1", 1).unwrap();
    channel
        .set_members(vec![BACnetDeviceObjectPropertyReference::new_local(
            av1(),
            PV.to_raw(),
        )])
        .unwrap();
    db.add(Box::new(channel)).unwrap();
    let mut schedule = ScheduleObject::new(5, "SCH-5", PropertyValue::Real(10.0)).unwrap();
    schedule
        .add_object_property_reference(BACnetObjectPropertyReference::new(ch1, PV.to_raw()))
        .unwrap();
    db.add(Box::new(schedule)).unwrap();
}

#[tokio::test(start_paused = true)]
async fn the_relinquish_starts_the_run_of_a_channel_target() {
    // The NULL relinquishes CH-1's slot 16, and the distribution that write
    // starts carries it on to AV-1: the run reached the request's effects.
    let mut h = Harness::start_with(ServerConfig::default(), through_a_channel).await;
    h.subscribe_cov().await;
    // The start-up run may still be distributing when the subscription's
    // first report goes out; wait for AV-1 to report 10.0.
    while h.cov_notification().await.list_of_values[0].value != real(10.0) {}
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Real(10.0));
    delete(&mut h, sch5()).await;
    response(&h).await.unwrap();
    let notification = h.cov_notification().await;
    assert_eq!(notification.monitored_object_identifier, av1());
    assert_eq!(notification.list_of_values[0].value, real(0.0));
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Null);
}
