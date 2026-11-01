//! AddListElement and RemoveListElement on a Notification Forwarder's
//! Subscribed_Recipients (#1049). Clause 12.51.9 narrows the comparison to the
//! recipient and process identifier: an add naming an entry renews it with the
//! element's other members, and a removal finds an entry whatever the
//! element's confirmation flag and Time Remaining.

use super::*;
use crate::server::test_forwarder::TestForwarder;
use bacnet_encoding::constructed::encode_event_notification_subscription_list;
use bacnet_objects::subscribed_recipients::MAX_SUBSCRIBED_RECIPIENTS;
use bacnet_services::list_manipulation::ListElementRequest;
use bacnet_types::constructed::{
    BACnetAddress, BACnetEventNotificationSubscription, BACnetRecipient,
};
use std::time::Duration;

const MINUTE: Duration = Duration::from_secs(60);

fn device(instance: u32, minutes: u32) -> BACnetEventNotificationSubscription {
    BACnetEventNotificationSubscription {
        recipient: BACnetRecipient::Device(
            ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        ),
        process_identifier: 1,
        issue_confirmed_notifications: false,
        time_remaining: minutes,
    }
}

fn framed(subscriptions: &[BACnetEventNotificationSubscription]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_event_notification_subscription_list(&mut buf, subscriptions).unwrap();
    buf.to_vec()
}

fn forwarder_db(
    entries: &[BACnetEventNotificationSubscription],
) -> (ObjectDatabase, ObjectIdentifier) {
    let mut forwarder = TestForwarder::new(1);
    forwarder
        .subscribed_recipients
        .write(PropertyValue::ApplicationData(framed(entries)))
        .unwrap();
    let oid = forwarder.object_identifier();
    let mut db = ObjectDatabase::new();
    db.set_monotonic_clock(Some(std::sync::Arc::new(|| NOW.with(|now| now.get()))));
    db.add(Box::new(forwarder)).unwrap();
    (db, oid)
}

std::thread_local! {
    /// The monotonic time `advance` has reached, for this test's thread.
    static NOW: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
}

/// Move the database's monotonic clock on by `by` and run the forwarder's
/// pass at the new time.
fn advance(db: &mut ObjectDatabase, oid: ObjectIdentifier, by: Duration) {
    let now = NOW.with(|now| {
        now.set(now.get() + by);
        now.get()
    });
    let object = db.get_mut(&oid).unwrap();
    object.timed().unwrap().advance_monotonic_time(now);
}

fn edit(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    list_of_elements: Vec<u8>,
    remove: bool,
) -> Result<(), Error> {
    let mut buf = BytesMut::new();
    ListElementRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::SUBSCRIBED_RECIPIENTS,
        property_array_index: None,
        list_of_elements,
    }
    .encode(&mut buf)
    .unwrap();
    if remove {
        handle_remove_list_element(db, &buf)
    } else {
        handle_add_list_element(db, &buf)
    }
}

/// The list as ReadProperty serves it.
fn read(db: &ObjectDatabase, oid: ObjectIdentifier) -> PropertyValue {
    db.get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::SUBSCRIBED_RECIPIENTS, None)
        .unwrap()
}

fn served(subscriptions: &[BACnetEventNotificationSubscription]) -> PropertyValue {
    PropertyValue::ApplicationData(framed(subscriptions))
}

#[test]
fn add_list_element_appends_new_entries_and_renews_named_ones() {
    let (mut db, oid) = forwarder_db(&[device(1, 5), device(2, 5)]);
    let mut renewal = device(1, 30);
    renewal.issue_confirmed_notifications = true;
    edit(
        &mut db,
        oid,
        framed(&[renewal.clone(), device(3, 5)]),
        false,
    )
    .unwrap();
    // Device 1 is renewed in place; device 3 joins the end.
    assert_eq!(
        read(&db, oid),
        served(&[renewal, device(2, 5), device(3, 5)])
    );
}

#[test]
fn entries_are_named_by_recipient_and_process_identifier_only() {
    let (mut db, oid) = forwarder_db(&[device(1, 5)]);
    let mut other_process = device(1, 5);
    other_process.process_identifier = 2;
    let address = BACnetEventNotificationSubscription {
        recipient: BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: bacnet_types::MacAddr::from_slice(&[10, 0, 0, 1, 0xBA, 0xC0]),
        }),
        ..device(1, 5)
    };
    // The same device under another process, and an address, are new entries.
    edit(
        &mut db,
        oid,
        framed(&[other_process.clone(), address.clone()]),
        false,
    )
    .unwrap();
    assert_eq!(
        read(&db, oid),
        served(&[device(1, 5), other_process, address])
    );
}

#[test]
fn a_repeat_within_one_add_renews_what_the_request_added() {
    let (mut db, oid) = forwarder_db(&[]);
    edit(&mut db, oid, framed(&[device(1, 5), device(1, 9)]), false).unwrap();
    assert_eq!(read(&db, oid), served(&[device(1, 9)]));
}

#[test]
fn remove_list_element_finds_entries_whatever_their_other_members() {
    let (mut db, oid) = forwarder_db(&[device(1, 5), device(2, 5), device(3, 5)]);
    let mut named = device(2, 1440);
    named.issue_confirmed_notifications = true;
    edit(&mut db, oid, framed(&[named]), true).unwrap();
    assert_eq!(read(&db, oid), served(&[device(1, 5), device(3, 5)]));
}

#[test]
fn remove_list_element_refuses_an_absent_entry_and_removes_nothing() {
    let (mut db, oid) = forwarder_db(&[device(1, 5), device(2, 5)]);
    let mut other_process = device(1, 5);
    other_process.process_identifier = 2;
    for (elements, position) in [
        (vec![device(1, 5), device(9, 5)], 2),
        (vec![other_process], 1),
    ] {
        assert_eq!(
            list_refusal(edit(&mut db, oid, framed(&elements), true)),
            (
                ErrorClass::SERVICES,
                ErrorCode::LIST_ELEMENT_NOT_FOUND,
                position
            )
        );
        assert_eq!(read(&db, oid), served(&[device(1, 5), device(2, 5)]));
    }
}

#[test]
fn add_list_element_names_a_time_remaining_out_of_range() {
    let (mut db, oid) = forwarder_db(&[device(1, 5)]);
    for (elements, position) in [
        // A new entry for no time at all.
        (vec![device(2, 5), device(3, 0)], 2),
        // A renewal past a day: the renewed entry names its element.
        (vec![device(2, 5), device(1, 1441)], 2),
        // Added, then renewed to zero by a later element.
        (vec![device(2, 5), device(2, 0)], 2),
    ] {
        assert_eq!(
            list_refusal(edit(&mut db, oid, framed(&elements), false)),
            (
                ErrorClass::PROPERTY,
                ErrorCode::VALUE_OUT_OF_RANGE,
                position
            ),
            "{elements:?}"
        );
        assert_eq!(read(&db, oid), served(&[device(1, 5)]), "nothing changes");
    }
}

#[test]
fn a_full_list_takes_renewals_but_refuses_another_entry() {
    let full: Vec<_> = (1..=MAX_SUBSCRIBED_RECIPIENTS as u32)
        .map(|instance| device(instance, 5))
        .collect();
    let (mut db, oid) = forwarder_db(&full);
    assert_eq!(
        list_refusal(edit(
            &mut db,
            oid,
            framed(&[device(1, 9), device(999, 5)]),
            false
        )),
        (
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT,
            2
        )
    );
    assert_eq!(read(&db, oid), served(&full), "nothing changes");
    edit(&mut db, oid, framed(&[device(1, 9)]), false).unwrap();
    let mut renewed = full;
    renewed[0] = device(1, 9);
    assert_eq!(read(&db, oid), served(&renewed));
}

#[test]
fn malformed_elements_are_refused_by_position() {
    let (mut db, oid) = forwarder_db(&[device(1, 5)]);
    let whole = framed(&[device(1, 5)]);
    // Framed tags, but no subscription: one lacks its [2] confirmation flag
    // ([1] then [3]), the other is a bare application-tagged Unsigned.
    let unflagged = [&whole[..], &whole[..9], &whole[11..]].concat();
    let untagged = [&whole[..], &[0x21, 0x05]].concat();
    for (elements, remove) in [
        (&unflagged, false),
        (&unflagged, true),
        (&untagged, false),
        (&untagged, true),
    ] {
        assert_eq!(
            list_refusal(edit(&mut db, oid, elements.clone(), remove)),
            (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE, 2),
            "{elements:02X?} remove {remove}"
        );
        assert_eq!(read(&db, oid), served(&[device(1, 5)]));
    }
}

#[test]
fn an_edit_leaves_the_deadlines_of_other_entries_alone() {
    let (mut db, oid) = forwarder_db(&[device(1, 2)]);
    advance(&mut db, oid, MINUTE / 2);
    edit(&mut db, oid, framed(&[device(2, 5)]), false).unwrap();
    // Device 1 still lapses two minutes after it was written, not at its
    // rounded-up minute count from the edit.
    assert_eq!(
        db.get_mut(&oid)
            .unwrap()
            .timed()
            .unwrap()
            .next_monotonic_deadline(),
        Some(2 * MINUTE)
    );
    advance(&mut db, oid, MINUTE + MINUTE / 2);
    assert_eq!(read(&db, oid), served(&[device(2, 4)]));
}

#[test]
fn a_lapsed_entry_is_not_found_and_can_be_added_afresh() {
    let (mut db, oid) = forwarder_db(&[device(1, 1)]);
    advance(&mut db, oid, MINUTE);
    assert_eq!(read(&db, oid), served(&[]));
    assert_eq!(
        list_refusal(edit(&mut db, oid, framed(&[device(1, 1)]), true)),
        (ErrorClass::SERVICES, ErrorCode::LIST_ELEMENT_NOT_FOUND, 1)
    );
    edit(&mut db, oid, framed(&[device(1, 3)]), false).unwrap();
    assert_eq!(read(&db, oid), served(&[device(1, 3)]));
}
