//! COV (Change of Value) subscription example.
//!
//! Demonstrates:
//! - Subscribing to COV on an analog input
//! - Receiving notifications via broadcast channel
//! - Simulating value changes from the server side
//!
//! Run with: cargo run --example cov_subscriptions

use bacnet_client::client::BACnetClient;
use bacnet_objects::analog_input::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use std::net::Ipv4Addr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Build server with an analog input
    let mut db = ObjectDatabase::new();
    db.add(Box::new(DeviceObject::new(DeviceConfig {
        instance: 1000,
        name: "Sensor Controller".into(),
        ..DeviceConfig::default()
    })?))?;
    let mut ai = AnalogInputObject::new(1, "Zone Temp", 62)?;
    ai.set_present_value(72.0);
    db.add(Box::new(ai))?;

    let server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(db)
        .build()
        .await?;

    let server_mac = server.local_mac().to_vec();

    let client = BACnetClient::bip_builder()
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
                notif.monitored_object_identifier,
                notif.list_of_values.len()
            );
            for val in &notif.list_of_values {
                println!("    {:?}", val.property_identifier);
            }
        }
    });

    // Simulate value changes
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)?;
    for temp in [73.0f32, 74.5, 71.0, 75.0] {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let db = server.database();
        let mut db = db.write().await;
        if let Some(obj) = db.get_mut(&oid) {
            obj.write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(temp),
                None,
            )?;
        }
        println!("Server wrote: {}", temp);
    }

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Unsubscribe
    client.unsubscribe_cov(&server_mac, 1, ai_oid).await?;
    println!("\nUnsubscribed from COV");

    listener.abort();
    client.stop().await;
    server.stop().await;

    Ok(())
}
