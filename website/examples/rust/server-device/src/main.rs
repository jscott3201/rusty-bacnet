//! Serve a small device over BACnet/IP: an Analog Input whose value the
//! application supplies, and a Binary Output that peers command.
#![forbid(unsafe_code)]

use bacnet_objects::DeviceIdentity;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryOutputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bvll::decode_bip_mac;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use std::io::{self, Write};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;
use tokio::io::AsyncBufReadExt;

const DEVICE_INSTANCE: u32 = 1001;
// Vendor 0 is a placeholder: use the vendor identifier ASHRAE assigned you.
const VENDOR_ID: u16 = 0;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os().len() != 1 {
        return Err("usage: server-device (press Enter or send EOF to stop)".into());
    }
    let identity =
        DeviceIdentity::new(DEVICE_INSTANCE, VENDOR_ID)?.with_name("Example air handler");
    let mut database = ObjectDatabase::new();
    database.add(Box::new(DeviceObject::new(DeviceConfig {
        identity: identity.clone(),
        ..DeviceConfig::default()
    })?))?;
    database.add(Box::new(AnalogInputObject::new(
        1,
        "Supply air temperature",
        EngineeringUnits::DEGREES_CELSIUS,
    )?))?;
    database.add(Box::new(BinaryOutputObject::new(1, "Supply fan")?))?;
    let temperature = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)?;
    let fan = ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1)?;

    // The server answers only requests sent to 127.0.0.1. For a real
    // network, give the interface's address and its subnet broadcast.
    let mut server = BACnetServer::bip_builder()
        .device(identity)
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(database)
        .build()
        .await?;

    // Keep every error on the path that stops the server.
    let served = async {
        // The application owns the input's value.
        server
            .set_present_value_local(&temperature, PropertyValue::Real(21.5))
            .await?;
        let (ip, port) = decode_bip_mac(server.local_mac())?;
        let address = SocketAddrV4::new(Ipv4Addr::from(ip), port);
        writeln!(
            io::stdout(),
            "Serving device {DEVICE_INSTANCE} at {address}"
        )?;
        writeln!(io::stdout(), "Press Enter to stop (or send EOF).")?;
        io::stdout().flush()?;

        let mut input = String::new();
        let mut stdin = tokio::io::BufReader::new(tokio::io::stdin());
        let stop = stdin.read_line(&mut input);
        tokio::pin!(stop);
        let mut ticks = tokio::time::interval(Duration::from_millis(100));
        let mut fan_on = false;
        loop {
            tokio::select! {
                read = &mut stop => {
                    read?;
                    break;
                }
                _ = ticks.tick() => {
                    // Peers command the fan; the application acts on the
                    // value the priority array settles on.
                    let command = server
                        .read_local(&fan, PropertyIdentifier::PRESENT_VALUE, None)
                        .await?;
                    let on = command == PropertyValue::Enumerated(1);
                    if on != fan_on {
                        fan_on = on;
                        let reading = if on { 19.0 } else { 21.5 };
                        server
                            .set_present_value_local(&temperature, PropertyValue::Real(reading))
                            .await?;
                        writeln!(io::stdout(), "Supply fan: {}", if on { "on" } else { "off" })?;
                        io::stdout().flush()?;
                    }
                }
            }
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    }
    .await;
    server.stop().await?;
    writeln!(io::stdout(), "Stopped")?;
    served
}
