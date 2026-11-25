use std::net::{Ipv4Addr, SocketAddrV4};

use bacnet_client::client::BACnetClient;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target: SocketAddrV4 = std::env::args()
        .nth(1)
        .ok_or("usage: first-read <IPv4:port>")?
        .parse()?;
    let client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::UNSPECIFIED)
        .port(0)
        .broadcast_address(Ipv4Addr::BROADCAST)
        .build()
        .await?;

    let [a, b, c, d] = target.ip().octets();
    let [hi, lo] = target.port().to_be_bytes();
    let address = [a, b, c, d, hi, lo];
    let object = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)?;
    let ack = client
        .read_property(&address, object, PropertyIdentifier::PRESENT_VALUE, None)
        .await?;
    let (value, _) = decode_application_value(&ack.property_value, 0)?;
    println!("{value:?}");
    Ok(())
}
