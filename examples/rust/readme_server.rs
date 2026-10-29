//! The root README's Rust server (#2105). Everything below this header is the
//! README's server block, byte for byte: website/tests/python/
//! test_site_samples.py checks it, and CI's clippy --all-targets compiles it.
#![allow(clippy::print_stdout)] // the README's program prints its address

use bacnet_objects::DeviceIdentity;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_types::enums::EngineeringUnits;
use std::net::Ipv4Addr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Vendor 0 is a placeholder: use the identifier ASHRAE assigned you.
    let identity = DeviceIdentity::new(1001, 0)?.with_name("Example air handler");
    let mut database = ObjectDatabase::new();
    database.add(Box::new(DeviceObject::new(DeviceConfig {
        identity: identity.clone(),
        ..DeviceConfig::default()
    })?))?;
    let mut supply = AnalogInputObject::new(1, "Supply air", EngineeringUnits::DEGREES_CELSIUS)?;
    supply.set_present_value(21.5);
    database.add(Box::new(supply))?;

    let mut server = BACnetServer::bip_builder()
        .device(identity)
        .interface(Ipv4Addr::LOCALHOST)
        .port(47809)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(database)
        .build()
        .await?;
    println!("Serving device 1001 on 127.0.0.1:47809; press Ctrl+C to stop");
    tokio::signal::ctrl_c().await?;
    server.stop().await?;
    Ok(())
}
