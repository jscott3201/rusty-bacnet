//! A BACnet/IP server whose objects come from a point list (#2445).
//!
//! Demonstrates:
//! - Describing a device's points as plain data, a `PointList`
//! - Building the server's object database from it with `build_database`
//! - Giving an input its first reading before the server starts
//! - Reading the points back over BACnet/IP
//!
//! Run from the workspace root with:
//! `cargo run -p bacnet-integration-tests --example point_list_server`

#![allow(clippy::print_stdout)] // an example demonstrates itself by printing

use bacnet_client::client::BACnetClient;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_objects::DeviceIdentity;
use bacnet_objects::points::{
    Access, Analog, Binary, DeviceInfo, MultiState, Point, PointList, build_database,
};
use bacnet_objects::traits::LocalInputValue;
use bacnet_server::server::BACnetServer;
use bacnet_types::enums::{BinaryPV, EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use std::net::Ipv4Addr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- Describe the points ---
    // Vendor 0 is a placeholder: use the identifier ASHRAE assigned you.
    let identity = DeviceIdentity::new(1001, 0)?;
    let supply = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)?;
    let fan = ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1)?;
    let mode = ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, 1)?;
    let list = PointList::new(
        DeviceInfo::new(1001)
            .with_name("AHU-1")
            .with_description("Air handler, level 2"),
        vec![
            Point::new(
                supply,
                "ahu-1.supply-air-temp",
                Analog::new(EngineeringUnits::DEGREES_CELSIUS, Access::ReadOnly)
                    .with_cov_increment(0.2),
            )
            .with_description("Supply air temperature"),
            Point::new(
                fan,
                "ahu-1.supply-fan",
                Binary::new(Access::Commandable {
                    relinquish_default: Some(BinaryPV::INACTIVE),
                })
                .with_active_text("On")
                .with_inactive_text("Off"),
            ),
            Point::new(
                mode,
                "ahu-1.mode",
                MultiState::new(3, Access::Writable).with_state_texts(["Off", "Heat", "Cool"]),
            ),
        ],
    );

    // --- Build the database; nothing serves it yet ---
    let mut database = build_database(list, &identity)?;
    database
        .get_mut(&supply)
        .and_then(|sensor| sensor.local_input())
        .ok_or("an Analog Input takes its Present_Value from the application")?
        .apply_local_input(LocalInputValue::present_value(PropertyValue::Real(13.5)))?;

    // --- Serve it ---
    let mut server = BACnetServer::bip_builder()
        .device(identity)
        .interface(Ipv4Addr::LOCALHOST)
        .port(0) // auto-assign
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(database)
        .build()
        .await?;
    let server_mac = server.local_mac().to_vec();

    // --- Read the points back ---
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .apdu_timeout_ms(3000)
        .build()
        .await?;
    let read = async |object: ObjectIdentifier, property: PropertyIdentifier| {
        let ack = client
            .read_property(&server_mac, object, property, None)
            .await?;
        let (value, _) = decode_application_value(&ack.property_value, 0)?;
        Ok::<_, Box<dyn std::error::Error>>(value)
    };
    for object in [supply, fan, mode] {
        let name = read(object, PropertyIdentifier::OBJECT_NAME).await?;
        println!("{object}: {name:?}");
    }
    let value = read(supply, PropertyIdentifier::PRESENT_VALUE).await?;
    assert_eq!(value, PropertyValue::Real(13.5));
    println!("Supply air: {value:?}");

    client.stop().await?;
    server.stop().await?;
    println!("Done.");
    Ok(())
}
