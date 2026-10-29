//! The website's PICS task page, integrate/pics (#1496). Everything below this
//! header is the Rust block in the page's "Generate one in Rust" section,
//! byte for byte: website/tests/python/test_site_samples.py checks it, and
//! CI's clippy --all-targets compiles it.
#![allow(clippy::print_stdout)] // the page's program prints the PICS

use bacnet_objects::DeviceIdentity;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryOutputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::pics::{DataLinkSupport, DeviceProfile, PicsConfig};
use bacnet_server::server::BACnetServer;
use bacnet_types::enums::EngineeringUnits;
use std::net::Ipv4Addr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Your product's details, on the Device object; the PICS reads them there.
    let version = "2.1.0";
    // Vendor 0 is a placeholder: use the identifier ASHRAE assigned you.
    let identity = DeviceIdentity::new(1001, 0)?.with_name("Example air handler");
    let mut database = ObjectDatabase::new();
    database.add(Box::new(DeviceObject::new(DeviceConfig {
        identity: identity.clone(),
        vendor_name: "Example Controls".into(),
        model_name: "AHU-100".into(),
        firmware_revision: version.into(),
        application_software_version: version.into(),
        ..DeviceConfig::default()
    })?))?;
    let celsius = EngineeringUnits::DEGREES_CELSIUS;
    database.add(Box::new(AnalogInputObject::new(1, "Supply air", celsius)?))?;
    database.add(Box::new(BinaryOutputObject::new(1, "Supply fan")?))?;

    let mut server = BACnetServer::bip_builder()
        .device(identity)
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(database)
        .build()
        .await?;

    // What the server can't report about itself: the profile you claim,
    // and the data links and network roles you ship.
    let pics_config = PicsConfig {
        // Claim an Annex L profile only if the product meets it.
        device_profile: DeviceProfile::Custom("None claimed".into()),
        data_link_layers: vec![DataLinkSupport::BipV4],
        ..PicsConfig::default()
    };
    let pics = server.generate_pics(&pics_config).await;
    server.stop().await?;
    println!("{}", pics.generate_markdown());
    Ok(())
}
