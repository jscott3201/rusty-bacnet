//! BACnet/IP client and server example.
//!
//! Demonstrates:
//! - Building a BACnet/IP server with objects
//! - Building a client and reading/writing properties
//! - ReadPropertyMultiple for efficient bulk reads
//! - Device discovery via WhoIs/IAm
//!
//! Run with: cargo run --example bip_client_server

use bacnet_client::client::BACnetClient;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_objects::analog_input::AnalogInputObject;
use bacnet_objects::analog_output::AnalogOutputObject;
use bacnet_objects::binary_value::BinaryValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;
use std::net::Ipv4Addr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- Build the object database ---
    let mut db = ObjectDatabase::new();

    let device = DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "Example HVAC Controller".into(),
        vendor_name: "Rusty BACnet".into(),
        vendor_id: 555,
        ..DeviceConfig::default()
    })?;
    db.add(Box::new(device))?;

    let mut ai = AnalogInputObject::new(1, "Zone Temp", 62)?;
    ai.set_present_value(72.5);
    db.add(Box::new(ai))?;

    let ao = AnalogOutputObject::new(1, "Damper Position", 98)?;
    db.add(Box::new(ao))?;

    let bv = BinaryValueObject::new(1, "Override Mode")?;
    db.add(Box::new(bv))?;

    // --- Start the server ---
    let server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0) // auto-assign
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(db)
        .build()
        .await?;

    let server_mac = server.local_mac().to_vec();
    println!("Server started, MAC: {:?}", server_mac);

    // --- Start the client ---
    let client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .apdu_timeout_ms(3000)
        .build()
        .await?;

    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)?;
    let ao_oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1)?;

    // --- Read a single property ---
    let ack = client
        .read_property(&server_mac, ai_oid, PropertyIdentifier::PRESENT_VALUE, None)
        .await?;
    let (value, _) = decode_application_value(&ack.property_value, 0)?;
    println!("Zone Temp: {:?}", value);

    // --- Write a property ---
    let mut buf = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut buf, &PropertyValue::Real(65.0))?;
    client
        .write_property(
            &server_mac,
            ao_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            buf.freeze(),
            Some(8),
        )
        .await?;
    println!("Wrote damper position: 65.0 @ priority 8");

    // --- ReadPropertyMultiple ---
    let specs = vec![
        ReadAccessSpecification {
            object_identifier: ai_oid,
            list_of_property_references: vec![
                PropertyReference {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                },
                PropertyReference {
                    property_identifier: PropertyIdentifier::OBJECT_NAME,
                    property_array_index: None,
                },
            ],
        },
        ReadAccessSpecification {
            object_identifier: ao_oid,
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
            }],
        },
    ];

    let rpm_ack = client.read_property_multiple(&server_mac, specs).await?;
    println!("\nReadPropertyMultiple results:");
    for result in &rpm_ack.list_of_read_access_results {
        println!("  Object: {:?}", result.object_identifier);
        for prop_result in &result.list_of_results {
            if let Some(ref value_bytes) = prop_result.read_result {
                if let Ok((val, _)) = decode_application_value(value_bytes, 0) {
                    println!("    {:?}: {:?}", prop_result.property_identifier, val);
                }
            }
        }
    }

    // --- WhoIs discovery ---
    client.who_is(None, None).await?;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let devices = client.discovered_devices().await;
    println!("\nDiscovered {} device(s):", devices.len());
    for dev in &devices {
        println!(
            "  Device {} vendor={}",
            dev.device_instance, dev.vendor_id
        );
    }

    // Cleanup
    client.stop().await;
    server.stop().await;
    println!("\nDone.");

    Ok(())
}
