//! COV (Change of Value) subscription example.
//!
//! Demonstrates:
//! - Subscribing to COV on an analog input, and to its Present_Value with
//!   SubscribeCOVPropertyMultiple
//! - Receiving both kinds of notification on one broadcast channel
//! - Simulating value changes from the server side
//!
//! Run from the workspace root with:
//! `cargo run -p bacnet-integration-tests --example cov_subscriptions`

#![allow(clippy::print_stdout)] // an example demonstrates itself by printing

use bacnet_client::client::{BACnetClient, COVEvent};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};
use bacnet_types::constructed::PropertyReference;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use std::net::Ipv4Addr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Build server with an analog input
    let identity = bacnet_objects::DeviceIdentity::new(1000, 0)
        .expect("valid Device identity")
        .with_name("Sensor Controller");
    let mut db = ObjectDatabase::new();
    db.add(Box::new(DeviceObject::new(DeviceConfig {
        identity: identity.clone(),
        ..DeviceConfig::default()
    })?))?;
    let mut ai = AnalogInputObject::new(1, "Zone Temp", EngineeringUnits::DEGREES_FAHRENHEIT)?;
    ai.set_present_value(72.0);
    db.add(Box::new(ai))?;

    let mut server = BACnetServer::bip_builder()
        .device(identity)
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(db)
        .build()
        .await?;

    let server_mac = server.local_mac().to_vec();

    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .build()
        .await?;

    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)?;

    // Start receiving notifications in a background task: both kinds
    // arrive on the one channel, in the order they came.
    let mut rx = client.cov_notifications();
    let listener = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            match event {
                COVEvent::Object(notif) => {
                    println!(
                        "  COV (process {}): object={:?}, values={} properties",
                        notif.notification.subscriber_process_identifier,
                        notif.notification.monitored_object_identifier,
                        notif.notification.list_of_values.len()
                    );
                    for val in &notif.notification.list_of_values {
                        println!("    {:?}", val.property_identifier);
                    }
                }
                COVEvent::Multiple(notif) => {
                    println!(
                        "  COV-multiple (process {}): {} objects",
                        notif.notification.subscriber_process_identifier,
                        notif.notification.list_of_cov_notifications.len()
                    );
                    for object in &notif.notification.list_of_cov_notifications {
                        for val in &object.list_of_values {
                            println!(
                                "    {:?} {:?} changed at {:?}",
                                object.monitored_object_identifier,
                                val.property_identifier,
                                val.time_of_change
                            );
                        }
                    }
                }
            }
        }
    });

    // Subscribe to COV (confirmed, 60s lifetime)
    client
        .subscribe_cov(&server_mac, 1, ai_oid, true, Some(60))
        .await?;
    println!("Subscribed to COV on AI:1");

    // And to AI:1's Present_Value alone with SubscribeCOVPropertyMultiple,
    // timestamped (unconfirmed, 60s lifetime, at most 1s of delay).
    let present_value = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 2,
        issue_confirmed_notifications: false,
        lifetime: Some(60),
        max_notification_delay: Some(1),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: ai_oid,
            list_of_cov_references: vec![COVReference {
                monitored_property: PropertyReference {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                },
                cov_increment: None,
                timestamped: true,
            }],
        }],
    };
    client
        .subscribe_cov_property_multiple(&server_mac, &present_value)
        .await?;
    println!("Subscribed to AI:1's Present_Value with SubscribeCOVPropertyMultiple");

    // Simulate value changes. `set_present_value_local` applies the server's
    // ownership checks and runs COV processing; writing through the raw
    // `database()` handle would do neither, and this in-service Analog Input
    // rejects a Present_Value write made that way.
    for temp in [73.0f32, 74.5, 71.0, 75.0] {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        server
            .set_present_value_local(&ai_oid, PropertyValue::Real(temp))
            .await?;
        println!("Server wrote: {}", temp);
    }

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Unsubscribe: the object subscription, then the whole COV-multiple
    // context (no specifications and no timing).
    client.unsubscribe_cov(&server_mac, 1, ai_oid).await?;
    let cancel = SubscribeCOVPropertyMultipleRequest {
        lifetime: None,
        max_notification_delay: None,
        list_of_cov_subscription_specifications: Vec::new(),
        ..present_value
    };
    client
        .subscribe_cov_property_multiple(&server_mac, &cancel)
        .await?;
    println!("\nUnsubscribed from COV");

    listener.abort();
    client.stop().await?;
    server.stop().await?;

    Ok(())
}
