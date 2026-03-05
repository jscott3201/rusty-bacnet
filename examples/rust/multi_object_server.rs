//! Server with multiple object types example.
//!
//! Demonstrates building a realistic BACnet device with many object types,
//! then querying it with RPM.
//!
//! Run with: cargo run --example multi_object_server

use bacnet_client::client::BACnetClient;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_objects::analog_input::AnalogInputObject;
use bacnet_objects::analog_output::AnalogOutputObject;
use bacnet_objects::analog_value::AnalogValueObject;
use bacnet_objects::binary_input::BinaryInputObject;
use bacnet_objects::binary_output::BinaryOutputObject;
use bacnet_objects::binary_value::BinaryValueObject;
use bacnet_objects::calendar::CalendarObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::multistate_value::MultiStateValueObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::schedule::ScheduleObject;
use bacnet_objects::trend_log::TrendLogObject;
use bacnet_server::server::BACnetServer;
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use std::net::Ipv4Addr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut db = ObjectDatabase::new();

    // Device object
    db.add(Box::new(DeviceObject::new(DeviceConfig {
        instance: 5000,
        name: "HVAC AHU-1".into(),
        vendor_name: "Rusty BACnet".into(),
        vendor_id: 555,
        model_name: "AHU Controller".into(),
        ..DeviceConfig::default()
    })?))?;

    // HVAC point objects
    let mut ai1 = AnalogInputObject::new(1, "Supply Air Temp", 62)?;
    ai1.set_present_value(55.0);
    db.add(Box::new(ai1))?;

    let mut ai2 = AnalogInputObject::new(2, "Return Air Temp", 62)?;
    ai2.set_present_value(72.0);
    db.add(Box::new(ai2))?;

    let mut ai3 = AnalogInputObject::new(3, "Outside Air Temp", 62)?;
    ai3.set_present_value(85.0);
    db.add(Box::new(ai3))?;

    db.add(Box::new(AnalogOutputObject::new(1, "Cooling Valve", 98)?))?;
    db.add(Box::new(AnalogOutputObject::new(2, "Heating Valve", 98)?))?;
    db.add(Box::new(AnalogValueObject::new(1, "Cooling Setpoint", 62)?))?;
    db.add(Box::new(BinaryInputObject::new(1, "Filter Status")?))?;
    db.add(Box::new(BinaryOutputObject::new(1, "Supply Fan")?))?;
    db.add(Box::new(BinaryValueObject::new(1, "Auto/Manual")?))?;
    db.add(Box::new(MultiStateValueObject::new(
        1,
        "Operating Mode",
        4,
    )?))?;
    db.add(Box::new(CalendarObject::new(1, "Holiday Calendar")?))?;
    db.add(Box::new(ScheduleObject::new(
        1,
        "Occupancy Schedule",
        PropertyValue::Unsigned(1),
    )?))?;
    db.add(Box::new(NotificationClass::new(1, "HVAC Alarms")?))?;
    db.add(Box::new(TrendLogObject::new(1, "Temp History", 1000)?))?;

    println!("Created {} objects", db.len());

    // Start server
    let server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(db)
        .build()
        .await?;

    let mac = server.local_mac().to_vec();

    // Start client and query all analog inputs
    let client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .build()
        .await?;

    // Read all 3 analog inputs at once with RPM
    let specs: Vec<_> = (1..=3)
        .map(|i| ReadAccessSpecification {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, i).unwrap(),
            list_of_property_references: vec![
                PropertyReference {
                    property_identifier: PropertyIdentifier::OBJECT_NAME,
                    property_array_index: None,
                },
                PropertyReference {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                },
                PropertyReference {
                    property_identifier: PropertyIdentifier::UNITS,
                    property_array_index: None,
                },
            ],
        })
        .collect();

    let rpm_ack = client.read_property_multiple(&mac, specs).await?;
    println!("\nAll analog inputs:");
    for result in &rpm_ack.list_of_read_access_results {
        print!("  {:?}: ", result.object_identifier);
        for prop in &result.list_of_results {
            if let Some(ref bytes) = prop.read_result {
                if let Ok((val, _)) = decode_application_value(bytes, 0) {
                    print!("{:?}={:?}  ", prop.property_identifier, val);
                }
            }
        }
        println!();
    }

    client.stop().await;
    server.stop().await;

    Ok(())
}
