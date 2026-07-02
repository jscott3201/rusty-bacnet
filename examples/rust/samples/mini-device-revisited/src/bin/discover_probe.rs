//! One-shot probe for a running mini-device-revisited server.
use std::net::Ipv4Addr;
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_transport::bip::DEFAULT_BACNET_PORT;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bind: Ipv4Addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "192.168.204.55".into())
        .parse()?;
    let broadcast: Ipv4Addr = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "192.168.204.255".into())
        .parse()?;
    let device_instance: u32 = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "3456".into())
        .parse()?;

    let server_mac: Vec<u8> = {
        let o = bind.octets();
        vec![o[0], o[1], o[2], o[3], 0xBA, 0xC0]
    };

    let mut client = BACnetClient::bip_builder()
        .interface(bind)
        .port(DEFAULT_BACNET_PORT + 1)
        .broadcast_address(broadcast)
        .build()
        .await?;

    // Unicast sanity check (works even when subnet broadcast is filtered)
    let device_oid = ObjectIdentifier::new(ObjectType::DEVICE, device_instance)?;
    let ack = client
        .read_property(
            &server_mac,
            device_oid,
            PropertyIdentifier::OBJECT_NAME,
            None,
        )
        .await?;
    let (value, _) = decode_application_value(&ack.property_value, 0)?;
    println!("unicast object-name: {value:?}");

    client.who_is(None, None).await?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let devices = client.discovered_devices().await;
    for d in &devices {
        println!(
            "discovered device {} mac={:?}",
            d.object_identifier.instance_number(),
            d.mac_address
        );
    }
    println!("discovered_total={}", devices.len());
    let _ = client.stop().await;
    Ok(())
}
