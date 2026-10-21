//! COV (Change of Value) subscription example.
//!
//! Demonstrates:
//! - Subscribing to COV on an analog input
//! - Receiving notifications via broadcast channel
//! - Simulating value changes from the server side
//!
//! Run from the workspace root with:
//! `cargo run -p bacnet-integration-tests --example cov_subscriptions`

#![allow(clippy::print_stdout)] // an example demonstrates itself by printing

use bacnet_client::client::BACnetClient;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_types::enums::{EngineeringUnits, ObjectType};
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

    // Subscribe to COV (confirmed, 60s lifetime)
    client
        .subscribe_cov(&server_mac, 1, ai_oid, true, Some(60))
        .await?;
    println!("Subscribed to COV on AI:1");

    // Start receiving notifications in a background task
    let mut rx = client.cov_notifications();
    let listener = tokio::spawn(async move {
        while let Ok(notif) = rx.recv().await {
            println!(
                "  COV: object={:?}, values={} properties",
                notif.notification.monitored_object_identifier,
                notif.notification.list_of_values.len()
            );
            for val in &notif.notification.list_of_values {
                println!("    {:?}", val.property_identifier);
            }
        }
    });

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

    // Unsubscribe
    client.unsubscribe_cov(&server_mac, 1, ai_oid).await?;
    println!("\nUnsubscribed from COV");

    listener.abort();
    client.stop().await?;
    server.stop().await?;

    Ok(())
}
