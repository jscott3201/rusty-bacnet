//! A Notification Forwarder's Subscribed_Recipients through the server's
//! request dispatch (#1049): AddListElement adds and renews, RemoveListElement
//! finds entries by recipient and process identifier, ReadProperty and
//! ReadPropertyMultiple serve the list, and refusals go out as ChangeList-Error
//! bodies naming the element.
//!
//! PROPERTY 2, RESOURCES 3, SERVICES 5; NO_SPACE_TO_ADD_LIST_ELEMENT 19,
//! VALUE_OUT_OF_RANGE 37, LIST_ELEMENT_NOT_FOUND 81.

use super::mutation_list_wire_tests::{ADD, REMOVE, change_list_error, list_request, wire};
use super::mutation_tests::{Fixture, oid};
use super::*;
use crate::server::test_forwarder::TestForwarder;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::constructed::encode_event_notification_subscription_list;
use bacnet_objects::subscribed_recipients::MAX_SUBSCRIBED_RECIPIENTS;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
use bacnet_types::constructed::{
    BACnetEventNotificationSubscription, BACnetRecipient, PropertyReference,
    ReadAccessSpecification,
};

const SUBSCRIBED_RECIPIENTS: PropertyIdentifier = PropertyIdentifier::SUBSCRIBED_RECIPIENTS;
const SIMPLE_ACK_ADD: [u8; 3] = [0x20, 5, 8];
const SIMPLE_ACK_REMOVE: [u8; 3] = [0x20, 5, 9];

fn device(instance: u32, minutes: u32) -> BACnetEventNotificationSubscription {
    BACnetEventNotificationSubscription {
        recipient: BACnetRecipient::Device(oid(ObjectType::DEVICE, instance)),
        process_identifier: 3,
        issue_confirmed_notifications: false,
        time_remaining: minutes,
    }
}

fn framed(subscriptions: &[BACnetEventNotificationSubscription]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_event_notification_subscription_list(&mut buf, subscriptions).unwrap();
    buf.to_vec()
}

async fn forwarder_fixture() -> (Fixture, ObjectIdentifier) {
    let fixture = Fixture::new(None);
    let forwarder = TestForwarder::new(1);
    let oid = forwarder.object_identifier();
    fixture.db.write().await.add(Box::new(forwarder)).unwrap();
    (fixture, oid)
}

async fn edit(
    fixture: &Fixture,
    forwarder: ObjectIdentifier,
    service: ConfirmedServiceChoice,
    subscriptions: &[BACnetEventNotificationSubscription],
) -> Vec<u8> {
    let request = list_request(
        forwarder,
        SUBSCRIBED_RECIPIENTS,
        None,
        &framed(subscriptions),
    );
    wire(fixture, service, request).await
}

/// The ComplexAck body of a confirmed request with invoke ID 5.
async fn ack_body(fixture: &Fixture, service: ConfirmedServiceChoice, request: Bytes) -> Bytes {
    let payload = wire(fixture, service, request).await;
    match decode_apdu(Bytes::from(payload)).unwrap() {
        Apdu::ComplexAck(ack) => ack.service_ack,
        other => panic!("expected a ComplexAck, got {other:?}"),
    }
}

/// Subscribed_Recipients as a ReadProperty request reads it on the wire.
async fn read(fixture: &Fixture, forwarder: ObjectIdentifier) -> Vec<u8> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: forwarder,
        property_identifier: SUBSCRIBED_RECIPIENTS,
        property_array_index: None,
    }
    .encode(&mut request);
    let body = ack_body(
        fixture,
        ConfirmedServiceChoice::READ_PROPERTY,
        request.freeze(),
    )
    .await;
    let ack = ReadPropertyACK::decode(&body).unwrap();
    assert_eq!(
        (ack.object_identifier, ack.property_identifier),
        (forwarder, SUBSCRIBED_RECIPIENTS)
    );
    ack.property_value
}

#[tokio::test]
async fn subscribed_recipients_add_renew_remove_and_read_on_the_wire() {
    let (fixture, forwarder) = forwarder_fixture().await;
    assert_eq!(read(&fixture, forwarder).await, framed(&[]));

    let added = [device(7, 10), device(8, 10)];
    assert_eq!(edit(&fixture, forwarder, ADD, &added).await, SIMPLE_ACK_ADD);
    assert_eq!(read(&fixture, forwarder).await, framed(&added));

    // Naming device 7 again renews it in place, with its new members.
    let mut renewal = device(7, 30);
    renewal.issue_confirmed_notifications = true;
    assert_eq!(
        edit(&fixture, forwarder, ADD, std::slice::from_ref(&renewal)).await,
        SIMPLE_ACK_ADD
    );
    assert_eq!(
        read(&fixture, forwarder).await,
        framed(&[renewal.clone(), device(8, 10)])
    );

    // A removal names device 8 whatever its Time Remaining says.
    assert_eq!(
        edit(&fixture, forwarder, REMOVE, &[device(8, 1)]).await,
        SIMPLE_ACK_REMOVE
    );
    assert_eq!(
        read(&fixture, forwarder).await,
        framed(std::slice::from_ref(&renewal))
    );

    // Now absent, so the removal fails at its element and changes nothing.
    assert_eq!(
        edit(&fixture, forwarder, REMOVE, &[device(7, 1), device(8, 1)]).await,
        change_list_error(REMOVE, 5, 81, 2)
    );
    // No entry may hold zero minutes.
    assert_eq!(
        edit(&fixture, forwarder, ADD, &[device(9, 5), device(10, 0)]).await,
        change_list_error(ADD, 2, 37, 2)
    );
    assert_eq!(
        read(&fixture, forwarder).await,
        framed(std::slice::from_ref(&renewal))
    );
}

#[tokio::test]
async fn a_full_subscribed_recipients_refuses_the_entry_past_the_cap_on_the_wire() {
    let (fixture, forwarder) = forwarder_fixture().await;
    let full: Vec<_> = (1..=MAX_SUBSCRIBED_RECIPIENTS as u32)
        .map(|instance| device(instance, 5))
        .collect();
    assert_eq!(edit(&fixture, forwarder, ADD, &full).await, SIMPLE_ACK_ADD);
    // The renewal fits; the new entry after it does not.
    assert_eq!(
        edit(&fixture, forwarder, ADD, &[device(1, 9), device(99, 5)]).await,
        change_list_error(ADD, 3, 19, 2)
    );
    assert_eq!(read(&fixture, forwarder).await, framed(&full));
}

#[tokio::test]
async fn read_property_multiple_all_serves_subscribed_recipients() {
    let (fixture, forwarder) = forwarder_fixture().await;
    let added = [device(7, 10)];
    assert_eq!(edit(&fixture, forwarder, ADD, &added).await, SIMPLE_ACK_ADD);
    let mut request = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: forwarder,
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::ALL,
                property_array_index: None,
            }],
        }],
    }
    .encode(&mut request)
    .unwrap();
    let body = ack_body(
        &fixture,
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        request.freeze(),
    )
    .await;
    let ack = ReadPropertyMultipleACK::decode(&body).unwrap();
    let results = &ack.list_of_read_access_results[0].list_of_results;
    let served = results
        .iter()
        .find(|result| result.property_identifier == SUBSCRIBED_RECIPIENTS)
        .expect("ALL includes Subscribed_Recipients");
    assert_eq!(served.property_value.as_deref(), Some(&framed(&added)[..]));
}
