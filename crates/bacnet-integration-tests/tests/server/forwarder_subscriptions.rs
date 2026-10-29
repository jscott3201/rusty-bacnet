use super::*;
use bacnet_encoding::constructed::encode_event_notification_subscription_list;
use bacnet_objects::subscribed_recipients::SubscribedRecipients;
use bacnet_objects::traits::MonotonicClock;
use bacnet_types::constructed::{
    BACnetEventNotificationSubscription, BACnetRecipient, PropertyReference,
    ReadAccessSpecification,
};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::{Error, ErrorDetail};
use bacnet_types::primitives::PropertyValue;
use std::borrow::Cow;

const SUBSCRIBED_RECIPIENTS: PropertyIdentifier = PropertyIdentifier::SUBSCRIBED_RECIPIENTS;

/// An application's own Notification Forwarder type, reduced to the identity
/// properties and Subscribed_Recipients.
struct Forwarder {
    oid: ObjectIdentifier,
    subscribed_recipients: SubscribedRecipients,
}

impl BACnetObject for Forwarder {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "Forwarder"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            PropertyIdentifier::OBJECT_IDENTIFIER => Ok(PropertyValue::ObjectIdentifier(self.oid)),
            PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.object_name().into()))
            }
            PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::NOTIFICATION_FORWARDER.to_raw(),
            )),
            SUBSCRIBED_RECIPIENTS => Ok(self.subscribed_recipients.read()),
            _ => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            )),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        match property {
            SUBSCRIBED_RECIPIENTS => self.subscribed_recipients.write(value),
            _ => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::WRITE_ACCESS_DENIED,
            )),
        }
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            SUBSCRIBED_RECIPIENTS,
        ])
    }

    fn bind_monotonic_clock_internal(&mut self, clock: Option<Arc<MonotonicClock>>) {
        self.subscribed_recipients.bind_monotonic_clock(clock);
    }

    fn advance_monotonic_time_internal(&mut self, now: Duration) -> bool {
        self.subscribed_recipients.advance_to(now)
    }

    fn next_monotonic_deadline_internal(&self) -> Option<Duration> {
        self.subscribed_recipients.next_deadline()
    }
}

fn device(instance: u32, minutes: u32) -> BACnetEventNotificationSubscription {
    BACnetEventNotificationSubscription {
        recipient: BACnetRecipient::Device(
            ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        ),
        process_identifier: 4,
        issue_confirmed_notifications: false,
        time_remaining: minutes,
    }
}

fn framed(subscriptions: &[BACnetEventNotificationSubscription]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_event_notification_subscription_list(&mut buf, subscriptions).unwrap();
    buf.to_vec()
}

/// A Notification Forwarder's Subscribed_Recipients end to end over loopback
/// UDP (#1049): AddListElement adds and renews, RemoveListElement finds an
/// entry by recipient and process identifier, ReadProperty and
/// ReadPropertyMultiple ALL return the list, and a refusal reaches the client
/// as a ChangeList-Error naming the element.
#[tokio::test]
async fn forwarder_subscribed_recipients_list_services_and_reads_reach_the_client() {
    let mut server = make_server().await;
    let mut client = make_client().await;
    let server_mac = server.local_mac().to_vec();
    let forwarder = ObjectIdentifier::new(ObjectType::NOTIFICATION_FORWARDER, 1).unwrap();
    server
        .database()
        .write()
        .await
        .add(Box::new(Forwarder {
            oid: forwarder,
            subscribed_recipients: SubscribedRecipients::new(),
        }))
        .unwrap();
    let read = || async {
        client
            .read_property(&server_mac, forwarder, SUBSCRIBED_RECIPIENTS, None)
            .await
            .unwrap()
            .property_value
    };

    client
        .add_list_element(
            &server_mac,
            forwarder,
            SUBSCRIBED_RECIPIENTS,
            None,
            framed(&[device(7, 10), device(8, 10)]),
        )
        .await
        .unwrap();
    assert_eq!(read().await, framed(&[device(7, 10), device(8, 10)]));

    let mut renewal = device(8, 60);
    renewal.issue_confirmed_notifications = true;
    client
        .add_list_element(
            &server_mac,
            forwarder,
            SUBSCRIBED_RECIPIENTS,
            None,
            framed(std::slice::from_ref(&renewal)),
        )
        .await
        .unwrap();
    assert_eq!(read().await, framed(&[device(7, 10), renewal.clone()]));

    client
        .remove_list_element(
            &server_mac,
            forwarder,
            SUBSCRIBED_RECIPIENTS,
            None,
            framed(&[device(7, 1)]),
        )
        .await
        .unwrap();
    assert_eq!(read().await, framed(std::slice::from_ref(&renewal)));

    let refused = client
        .remove_list_element(
            &server_mac,
            forwarder,
            SUBSCRIBED_RECIPIENTS,
            None,
            framed(&[device(8, 1), device(7, 1)]),
        )
        .await;
    match refused {
        Err(Error::Structured {
            class,
            code,
            detail,
        }) => {
            assert_eq!(
                (class, code, *detail),
                (
                    ErrorClass::SERVICES,
                    ErrorCode::LIST_ELEMENT_NOT_FOUND,
                    ErrorDetail::FirstFailedElementNumber(2)
                )
            );
        }
        other => panic!("expected a ChangeList-Error, got {other:?}"),
    }
    assert_eq!(read().await, framed(std::slice::from_ref(&renewal)));

    let ack = client
        .read_property_multiple(
            &server_mac,
            vec![ReadAccessSpecification {
                object_identifier: forwarder,
                list_of_property_references: vec![PropertyReference {
                    property_identifier: PropertyIdentifier::ALL,
                    property_array_index: None,
                }],
            }],
        )
        .await
        .unwrap();
    let served = ack.list_of_read_access_results[0]
        .list_of_results
        .iter()
        .find(|result| result.property_identifier == SUBSCRIBED_RECIPIENTS)
        .and_then(|result| result.property_value.clone());
    assert_eq!(served, Some(framed(&[renewal])));

    server.stop().await.unwrap();
    client.stop().await.unwrap();
}
