//! The root README's quickstart read (#2105). Everything below this header is
//! the README's Rust read block, byte for byte: website/tests/python/
//! test_site_samples.py checks it, and CI's clippy --all-targets compiles it.
#![allow(clippy::print_stdout)] // the README's program prints its result

use bacnet_client::client::BACnetClient;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use std::net::Ipv4Addr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)?;
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .build()
        .await?;

    // A BACnet/IP address: four IPv4 octets, then the UDP port (0xBAC0 = 47808).
    let address = [127, 0, 0, 1, 0xBA, 0xC0];
    let response = client
        .read_property(&address, oid, PropertyIdentifier::PRESENT_VALUE, None)
        .await;
    client.stop().await?;

    let ack = response?;
    let (value, _) = decode_application_value(&ack.property_value, 0)?;
    println!("{value:?}");
    Ok(())
}
