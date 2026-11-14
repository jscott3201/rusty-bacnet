//! A Schedule removed by the application gives up its slots, and a
//! Schedule gives up only the slots it still holds (#2264).
//!
//! The application's own `ObjectDatabase::remove` queues the removed
//! Schedule's NULLs, which the server writes under the next guard its
//! Schedule task takes, or the first after start for one removed before it.
//! On every path that gives a slot up (DeleteObject, the application's
//! removal, RemoveListElement, a change of Priority_For_Writing) a slot
//! whose Value_Source names another writer keeps that writer's command.
//! Leaving Effective_Period writes nothing at all. A target that serves no
//! Value_Source_Array, such as a Channel, gets the NULL as before.
//!
//! The harness is the one in `schedule_write_tests`: Tuesday 29 September
//! 2026, 15:00; SCH-5 commands AV-1's Present_Value at priority 16 and
//! defaults to 10.0, written at start-up; the COV subscription watches AV-1,
//! whose Relinquish_Default is 0.0.
use super::cov_wire_test_support::*;
use super::schedule_reference_write_tests::slot;
use super::schedule_write_tests::{read, sch5, schedule, write_property};
use super::*;
use bacnet_encoding::constructed::{decode_value_source, encode_device_object_property_reference};
use bacnet_objects::channel::ChannelObject;
use bacnet_objects::command_source::CommandOrigin;
use bacnet_objects::schedule::ScheduleObject;
use bacnet_services::list_manipulation::ListElementRequest;
use bacnet_services::object_mgmt::DeleteObjectRequest;
use bacnet_types::calendar::SpecificDate;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference,
    BACnetObjectPropertyReference, BACnetValueSource,
};
use bacnet_types::enums::ObjectType;

fn ch1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::CHANNEL, 1).unwrap()
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

/// The application commands `object`'s Present_Value at priority 16, the
/// slot SCH-5 writes, with the server's Device as the writer.
async fn take_slot_16(h: &Harness, object: ObjectIdentifier, value: f32) {
    h.server
        .write_local(
            &object,
            PV,
            None,
            PropertyValue::Real(value),
            Some(16),
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
}

/// SCH-5 as the harness starts it, after the application took AV-1's slot
/// 16 from it with 42.0, and the COV that reports it.
async fn taken_over() -> Harness {
    let h = start_with(schedule).await;
    take_slot_16(&h, av1(), 42.0).await;
    assert_eq!(
        h.cov_notification().await.list_of_values[0].value,
        real(42.0)
    );
    h
}

/// AV-1 still holds the application's 42.0 at priority 16, and no COV
/// reported anything else.
async fn assert_kept(h: &Harness) {
    assert_eq!(slot(h, av1(), 16).await, PropertyValue::Real(42.0));
    assert_eq!(read(h, av1(), PV).await, PropertyValue::Real(42.0));
    h.no_notification().await;
}

async fn delete(h: &mut Harness, oid: ObjectIdentifier) {
    let mut body = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: oid,
    }
    .encode(&mut body);
    h.request(ConfirmedServiceChoice::DELETE_OBJECT, body).await;
    response(h).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn an_application_remove_relinquishes_the_targets_and_fans_cov_out() {
    let h = start_with(schedule).await;
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Real(10.0));
    let removed = h.server.database().write().await.remove(&sch5()).unwrap();
    assert!(removed.is_some());
    // The Schedule task takes the queued NULL under its next guard.
    let notification = h.cov_notification().await;
    assert_eq!(notification.monitored_object_identifier, av1());
    assert_eq!(notification.list_of_values[0].value, real(0.0));
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Null);
    assert_eq!(read(&h, av1(), PV).await, PropertyValue::Real(0.0));
    // The NULL carried the removed Schedule as its writer.
    assert_eq!(source_16(&h).await, sch5_source());
    h.no_notification().await;
}

/// AV-1's Value_Source_Array element for priority 16.
async fn source_16(h: &Harness) -> BACnetValueSource {
    let PropertyValue::ApplicationData(bytes) = h
        .server
        .database()
        .read()
        .await
        .get(&av1())
        .unwrap()
        .read_property(PropertyIdentifier::VALUE_SOURCE_ARRAY, Some(16))
        .unwrap()
    else {
        panic!("Value_Source_Array encodes as application data");
    };
    decode_value_source(&bytes, 0).unwrap().0
}

fn sch5_source() -> BACnetValueSource {
    BACnetValueSource::Object(BACnetDeviceObjectReference {
        device_identifier: None,
        object_identifier: sch5(),
    })
}

/// A new SCH-5 commanding AV-1 at priority 16 with 20.0.
fn fresh_sch5() -> Box<ScheduleObject> {
    let mut fresh = ScheduleObject::new(5, "SCH-5", PropertyValue::Real(20.0)).unwrap();
    fresh
        .add_object_property_reference(BACnetObjectPropertyReference::new(av1(), PV.to_raw()))
        .unwrap();
    Box::new(fresh)
}

#[tokio::test(start_paused = true)]
async fn replacement_written_before_the_settle_is_not_cleared() {
    // The old SCH-5's NULL is still queued when the pass runs; the pass
    // drains it first, so the new SCH-5's command lands after it.
    let h = start_with(schedule).await;
    {
        let mut db = h.server.database().write().await;
        db.remove(&sch5()).unwrap();
        db.add(fresh_sch5()).unwrap();
    }
    let _ = crate::schedule::tick_schedules_committed(
        h.server.database(),
        &h.server.cov_table,
        &mut |_, _| false,
    )
    .await;
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Real(20.0));
    h.settle().await;
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Real(20.0));
}

#[tokio::test(start_paused = true)]
async fn an_add_that_replaces_a_schedule_relinquishes_its_slots() {
    let h = start_with(schedule).await;
    h.server.database().write().await.add(fresh_sch5()).unwrap();
    let notification = h.cov_notification().await;
    assert_eq!(notification.list_of_values[0].value, real(0.0));
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Null);
    // The new SCH-5 commands the slot at its first pass.
    let _ = crate::schedule::tick_schedules_committed(
        h.server.database(),
        &h.server.cov_table,
        &mut |_, _| false,
    )
    .await;
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Real(20.0));
}

#[tokio::test(start_paused = true)]
async fn a_calculated_null_is_not_checked() {
    // A NULL Schedule_Default is SCH-5's value, not a relinquish: it goes
    // to AV-1's slot 16 whoever holds it (#2265 tracks that case).
    let mut h = taken_over().await;
    write_property(
        &mut h,
        PropertyIdentifier::SCHEDULE_DEFAULT,
        None,
        vec![0x00],
    )
    .await
    .unwrap();
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Null);
    assert_eq!(read(&h, av1(), PV).await, PropertyValue::Real(0.0));
}

#[tokio::test(start_paused = true)]
async fn an_application_remove_commits_under_the_guard_and_cov_waits_for_it() {
    let h = start_with(schedule).await;
    let (entered, release) = h.hold_notification_barrier(true);
    h.server.database().write().await.remove(&sch5()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), entered.acquire())
        .await
        .expect("AV-1's notification")
        .unwrap()
        .forget();
    // The notification is in the transport: the database guard is free,
    // and AV-1's slot is already empty.
    {
        let db = h
            .server
            .database()
            .try_write()
            .expect("no database guard held while COV is sent");
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
    assert_eq!(
        h.cov_notification().await.list_of_values[0].value,
        real(0.0)
    );
}

/// SCH-5 commanded AV-1 at priority 16 through the application's own pass,
/// with the origin a server pass gives it, and was then removed, all before
/// the server started.
fn removed_before_start(db: &mut ObjectDatabase) {
    schedule(db);
    let tuesday = SpecificDate::new(2026, 9, 29).unwrap();
    let write = db
        .get_mut(&sch5())
        .and_then(|object| object.schedule_runner())
        .and_then(|runner| runner.tick_schedule(tuesday, time(0), &|_| false))
        .expect("the first pass writes");
    let origin = CommandOrigin::Local {
        owner_device: ObjectIdentifier::new(ObjectType::DEVICE, 856).unwrap(),
        initiating_object: Some(sch5()),
    };
    db.get_mut(&av1())
        .unwrap()
        .write_property_from(PV, None, write.value, Some(write.priority), &origin)
        .unwrap();
    db.remove(&sch5()).unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_schedule_removed_before_the_server_starts_relinquishes_once_it_does() {
    let mut h = Harness::start_with(ServerConfig::default(), removed_before_start).await;
    h.settle().await;
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Null);
    h.subscribe_cov().await;
    assert_eq!(
        h.cov_notification().await.list_of_values[0].value,
        real(0.0)
    );
}

#[tokio::test(start_paused = true)]
async fn a_delete_leaves_a_slot_another_writer_took() {
    let mut h = taken_over().await;
    delete(&mut h, sch5()).await;
    assert!(h.server.database().read().await.get(&sch5()).is_none());
    assert_kept(&h).await;
}

#[tokio::test(start_paused = true)]
async fn an_application_remove_leaves_a_slot_another_writer_took() {
    let h = taken_over().await;
    h.server.database().write().await.remove(&sch5()).unwrap();
    h.settle().await;
    assert_kept(&h).await;
}

#[tokio::test(start_paused = true)]
async fn remove_list_element_leaves_a_slot_another_writer_took() {
    let mut h = taken_over().await;
    let mut elements = BytesMut::new();
    encode_device_object_property_reference(
        &mut elements,
        &BACnetDeviceObjectPropertyReference::new_local(av1(), PV.to_raw()),
    );
    let mut body = BytesMut::new();
    ListElementRequest {
        object_identifier: sch5(),
        property_identifier: PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES,
        property_array_index: None,
        list_of_elements: elements.to_vec(),
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::REMOVE_LIST_ELEMENT, body)
        .await;
    response(&h).await.unwrap();
    assert_kept(&h).await;
}

#[tokio::test(start_paused = true)]
async fn a_new_priority_leaves_the_old_slot_another_writer_took() {
    let mut h = taken_over().await;
    // Priority 9: SCH-5's 10.0 goes there, and wins over slot 16.
    write_property(
        &mut h,
        PropertyIdentifier::PRIORITY_FOR_WRITING,
        None,
        vec![0x21, 9],
    )
    .await
    .unwrap();
    assert_eq!(slot(&h, av1(), 9).await, PropertyValue::Real(10.0));
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Real(42.0));
    assert_eq!(
        h.cov_notification().await.list_of_values[0].value,
        real(10.0)
    );
}

#[tokio::test(start_paused = true)]
async fn leaving_the_period_writes_nothing_to_a_slot_another_writer_took() {
    let mut h = taken_over().await;
    // An Effective_Period that ended yesterday: SCH-5 goes inactive and
    // writes nothing, and holds nothing a later delete would give up. This
    // pins the exit, not the Value_Source check: with nothing held, the
    // delete owes no NULL for the check to stop.
    write_property(
        &mut h,
        PropertyIdentifier::EFFECTIVE_PERIOD,
        None,
        vec![0xA4, 126, 9, 1, 2, 0xA4, 126, 9, 28, 1],
    )
    .await
    .unwrap();
    assert_kept(&h).await;
    delete(&mut h, sch5()).await;
    assert_kept(&h).await;
}

/// SCH-5 commanding CH-1's Present_Value at priority 16, and CH-1 passing
/// what it holds on to AV-1.
fn through_a_channel(db: &mut ObjectDatabase) {
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
        .add_object_property_reference(BACnetObjectPropertyReference::new(ch1(), PV.to_raw()))
        .unwrap();
    db.add(Box::new(schedule)).unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_target_without_value_source_still_gets_the_null() {
    // CH-1 serves no Value_Source_Array, so who wrote it last can't be
    // told: the application's 42.0 there is relinquished as before.
    let mut h = Harness::start_with(ServerConfig::default(), through_a_channel).await;
    h.subscribe_cov().await;
    while h.cov_notification().await.list_of_values[0].value != real(10.0) {}
    take_slot_16(&h, ch1(), 42.0).await;
    while h.cov_notification().await.list_of_values[0].value != real(42.0) {}
    delete(&mut h, sch5()).await;
    let notification = h.cov_notification().await;
    assert_eq!(notification.monitored_object_identifier, av1());
    assert_eq!(notification.list_of_values[0].value, real(0.0));
    assert_eq!(read(&h, ch1(), PV).await, PropertyValue::Null);
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Null);
}
