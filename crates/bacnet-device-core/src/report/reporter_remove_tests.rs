//! An object the host deletes (#2384): its subscriptions, queued and owed
//! reports and confirmed notifications go with it, and an object created
//! again under its identifier reports to nobody.

use super::*;
use bacnet_encoding::apdu::ErrorPdu;
use bacnet_encoding::constructed::decode_cov_subscription;
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_types::enums::{ErrorClass, ErrorCode};

/// Every notification `pop` hands out at `now`, by subscriber process; an
/// unconfirmed one is reported sent.
fn sent(device: &mut Device, now: Instant) -> Vec<u32> {
    let mut sent = Vec::new();
    while let Some((notification, apdu)) = device.pop(now) {
        if notification.invoke_id.is_none() {
            device
                .reporter
                .delivered(notification.token, Delivery::Sent, &mut device.ids, now);
        }
        sent.push(reported(&apdu).0);
    }
    sent
}

/// The objects the Device's Active_COV_Subscriptions lists at `now`.
fn listed(device: &mut Device, now: Instant) -> Vec<ObjectIdentifier> {
    let mut body = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap(),
        property_identifier: P::ACTIVE_COV_SUBSCRIPTIONS,
        property_array_index: None,
    }
    .encode(&mut body);
    let Apdu::ComplexAck(ack) = device.request(ConfirmedServiceChoice::READ_PROPERTY, &body, now)
    else {
        panic!("a ComplexACK");
    };
    let value = ReadPropertyACK::decode(&ack.service_ack)
        .unwrap()
        .property_value;
    let mut objects = Vec::new();
    let mut at = 0;
    while at < value.len() {
        let (subscription, next) = decode_cov_subscription(&value, at).unwrap();
        objects.push(subscription.monitored_property_reference.object_identifier);
        at = next;
    }
    objects
}

/// An Error answering the confirmed notification sent under `invoke_id`.
fn refuse(device: &mut Device, invoke_id: u8, now: Instant) -> bool {
    let mut apdu = BytesMut::new();
    let error = Apdu::Error(ErrorPdu {
        invoke_id,
        service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
        error_class: ErrorClass::SERVICES,
        error_code: ErrorCode::SERVICE_REQUEST_DENIED,
        error_data: Bytes::new(),
    });
    encode_apdu(&mut apdu, &error).unwrap();
    device
        .reporter
        .reply(&peer_address(&peer()), &apdu, &mut device.ids, now)
}

/// AI-1 has a confirmed notification out, a queued report and an owed one
/// when it is deleted: none of them is sent, its invoke ID goes back, and
/// AI-2's queued report still goes. An AI-1 created again isn't listed and
/// reports nothing.
#[test]
fn a_deleted_object_takes_its_reports_with_it() {
    let mut device = Device::new(OutboxLimits {
        entries: 2,
        octets: 256,
    });
    device.subscribe(1, true, T0);
    let (out, _) = device.pop(T0).expect("1's first notification");
    assert_eq!(out.invoke_id, Some(0));
    device.subscribe(2, false, T0);
    device.subscribe_to(ai2(), 3, false, T0);
    device.subscribe(4, false, T0);
    assert_eq!(
        device.reporter.owed.len(),
        1,
        "4's first report found no room"
    );
    assert_eq!(listed(&mut device, T0), [ai(), ai(), ai(), ai2()]);

    device.db.remove(&ai()).unwrap();
    device.reporter.remove_for_object(ai(), &mut device.ids);
    assert!(device.ids.taken.is_empty(), "1's invoke ID went back");
    assert!(device.reporter.owed.is_empty(), "4 is owed nothing");
    assert!(device.reporter.confirmed.next_deadline().is_none());
    let table = device.reporter.table();
    assert!(table.object_entries(ai()).next().is_none());
    assert_eq!(table.len(), 1);
    assert_eq!(sent(&mut device, T0), [3], "2's queued report died");
    // Past where 1's notification would have been resent and given up.
    let end = T0 + Duration::from_secs(20);
    while let Some(due) = device.reporter.next_deadline().filter(|due| *due <= end) {
        device
            .reporter
            .on_deadline(&mut device.db, &mut device.ids, due);
        assert!(
            sent(&mut device, due).is_empty(),
            "nothing for AI-1 at {due:?}"
        );
    }
    let counters = device.reporter.counters();
    assert_eq!((counters.resent, counters.failed), (0, 0));

    // The identifier reused: the old subscriptions are gone from the list
    // and from the reporting.
    let again = AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap();
    device.db.add(Box::new(again)).unwrap();
    assert_eq!(listed(&mut device, end), [ai2()]);
    device.write(P::OUT_OF_SERVICE, PropertyValue::Boolean(true), end);
    assert!(sent(&mut device, end).is_empty());
}

/// A SimpleACK or an Error to a deleted object's notification, after its
/// invoke ID went back, ends nothing: not the notification about AI-2 still
/// out, nor the one sent after the deletion. The host lends invoke IDs in
/// turn, so the later one has an ID of its own.
#[test]
fn a_late_answer_to_a_deleted_objects_notification_ends_nothing() {
    let mut device = Device::new(OutboxLimits::default());
    device.subscribe(1, true, T0);
    let (gone, _) = device.pop(T0).unwrap();
    device.subscribe_to(ai2(), 2, true, T0);
    let (kept, _) = device.pop(T0).unwrap();
    device.db.remove(&ai()).unwrap();
    device.reporter.remove_for_object(ai(), &mut device.ids);
    device.subscribe_to(ai2(), 3, true, T0);
    let (later, _) = device.pop(T0).unwrap();
    let ids = [gone, kept, later].map(|sent| sent.invoke_id.unwrap());
    assert_eq!(ids, [0, 1, 2]);
    let held = |device: &Device| {
        let mut held: Vec<_> = device.ids.taken.iter().map(|taken| taken.1).collect();
        held.sort_unstable();
        held
    };
    assert_eq!(held(&device), [1, 2]);

    assert!(!device.ack(ids[0], T0));
    assert!(!refuse(&mut device, ids[0], T0));
    assert_eq!(held(&device), [1, 2], "AI-2's notifications are still out");
    assert_eq!(device.reporter.counters().failed, 0);
    assert!(device.ack(ids[1], T0));
    assert!(device.ack(ids[2], T0));
    assert!(device.ids.taken.is_empty());
}

/// Removing an object with no subscription changes nothing; removing one
/// twice does nothing the second time.
#[test]
fn removing_nothing_or_twice_changes_nothing() {
    let mut device = Device::new(OutboxLimits::default());
    device.subscribe_to(ai2(), 1, false, T0);
    let (counters, due) = (device.reporter.counters(), device.reporter.next_deadline());
    device.reporter.remove_for_object(ai(), &mut device.ids);
    assert_eq!(device.reporter.counters(), counters);
    assert_eq!(device.reporter.next_deadline(), due);
    assert_eq!(device.reporter.table().len(), 1);
    assert_eq!(sent(&mut device, T0), [1], "AI-2's report is still queued");

    device.subscribe_to(ai2(), 2, true, T0);
    device.db.remove(&ai2()).unwrap();
    device.reporter.remove_for_object(ai2(), &mut device.ids);
    let counters = device.reporter.counters();
    assert!(device.reporter.table().is_empty());
    let due = device.reporter.next_deadline();
    device.reporter.remove_for_object(ai2(), &mut device.ids);
    assert_eq!(device.reporter.counters(), counters);
    assert_eq!(device.reporter.next_deadline(), due);
    assert!(device.reporter.table().is_empty());
    assert!(device.pop(T0).is_none());
    assert!(device.ids.taken.is_empty());
}

/// Two confirmed notifications out for AI-1, and one for BO-1: the same
/// instance under a type whose encoding differs from AI's in the first
/// octet alone. Deleting AI-1 ends both of its own and gives their invoke
/// IDs back; BO-1's stays out and its answer still ends it.
#[test]
fn a_deletion_ends_each_of_the_objects_notifications_and_only_those() {
    let mut device = Device::new(OutboxLimits::default());
    let bo = ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1).unwrap();
    let output = bacnet_objects::binary::BinaryOutputObject::new(1, "BO-1").unwrap();
    device.db.add(Box::new(output)).unwrap();
    assert_eq!(ai().encode()[1..], bo.encode()[1..]);
    device.subscribe(1, true, T0);
    device.subscribe(2, true, T0);
    device.subscribe_to(bo, 3, true, T0);
    let mut out = Vec::new();
    while let Some((sent, apdu)) = device.pop(T0) {
        out.push((reported(&apdu).0, sent.invoke_id.unwrap()));
    }
    out.sort_unstable();
    let [(1, _), (2, _), (3, kept)] = out[..] else {
        panic!("one notification out for each: {out:?}");
    };

    device.db.remove(&ai()).unwrap();
    device.reporter.remove_for_object(ai(), &mut device.ids);
    let held: Vec<_> = device.ids.taken.iter().map(|taken| taken.1).collect();
    assert_eq!(held, [kept], "AI-1's two invoke IDs went back");
    for (process, id) in &out[..2] {
        assert!(!device.ack(*id, T0), "{process}'s notification ended");
    }
    assert!(device.ack(kept, T0), "BO-1's is still out");
    assert!(device.ids.taken.is_empty());
}

/// A look owed to a deleted object's subscription, and a Life Safety
/// object's capture, go with the object: no deadline comes for the look,
/// and a Point created again under the identifier isn't compared with the
/// old one's values.
#[test]
fn a_deleted_objects_owed_look_and_capture_go() {
    let mut device = Device::new(OutboxLimits::default());
    device.subscribe(1, true, T0);
    let (out, _) = device.pop(T0).unwrap();
    device.write(P::OUT_OF_SERVICE, PropertyValue::Boolean(true), T0);
    assert!(device.ack(out.invoke_id.unwrap(), T0));
    assert_eq!(device.reporter.next_deadline(), Some(T0), "a look is owed");
    let point = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap();
    let lsp = bacnet_objects::life_safety::LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    device.db.add(Box::new(lsp)).unwrap();
    device.reporter.capture(&device.db, &[point]);
    assert!(device.reporter.snapshots.holds(point));

    device.db.remove(&ai()).unwrap();
    device.reporter.remove_for_object(ai(), &mut device.ids);
    assert!(device.reporter.looks.is_empty());
    assert!(
        device.reporter.next_deadline().is_none_or(|due| due > T0),
        "no look comes due"
    );
    device.db.remove(&point).unwrap();
    device.reporter.remove_for_object(point, &mut device.ids);
    assert!(!device.reporter.snapshots.holds(point));
}
