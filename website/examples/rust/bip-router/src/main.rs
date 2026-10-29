//! Route between two BACnet/IP networks on one host: a router joins
//! networks 1 and 5, and a client on network 1 finds the router, then
//! finds and reads a device on network 5 through it.
#![forbid(unsafe_code)]

use bacnet_client::client::BACnetClient;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use bacnet_objects::DeviceIdentity;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::bbmd::ForeignDevicePolicy;
use bacnet_transport::bip::{BbmdOptions, BipMode, BipTransport, ForeignDeviceConfig};
use bacnet_transport::bvll::decode_bip_mac;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use std::io::{self, Write};
use std::net::Ipv4Addr;
use std::time::Duration;

type Error = Box<dyn std::error::Error>;

const NEAR: u16 = 1;
const FAR: u16 = 5;
const DEVICE_INSTANCE: u32 = 1005;
const WAIT: Duration = Duration::from_secs(2);

/// One router port on 127.0.0.1, on a port the OS picks. Every node here
/// shares that address, so a B/IP broadcast would reach nobody else: each
/// port is a BBMD, and the other nodes register with it as foreign devices
/// to send and receive broadcasts.
fn router_port(network_number: u16) -> RouterPort<BipTransport> {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    transport.set_mode(BipMode::Bbmd(BbmdOptions {
        foreign_device_policy: Some(ForeignDevicePolicy::default()),
        alone_on_subnet: true,
        ..BbmdOptions::default()
    }));
    RouterPort {
        transport,
        network_number,
    }
}

/// Register as a foreign device with the router port whose MAC is `bbmd`.
fn foreign_device_of(bbmd: &[u8]) -> Result<BipMode, Error> {
    let (ip, port) = decode_bip_mac(bbmd)?;
    Ok(BipMode::Foreign(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::from(ip),
        bbmd_port: port,
        ttl: 60,
        renewal_interval: None,
    }))
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let StartedRouter { mut router, .. } = BACnetRouter::start(
        vec![router_port(NEAR), router_port(FAR)],
        RouterOptions::new(),
    )
    .await?;
    let macs: Vec<Vec<u8>> = router
        .ports()
        .iter()
        .map(|port| port.local_mac().to_vec())
        .collect();
    writeln!(io::stdout(), "Router started: networks {NEAR} and {FAR}")?;
    let outcome = serve_and_read(&macs[0], &macs[1]).await;
    // Only a successful stop means the router's sockets are closed.
    let stopped = router.stop().await;
    outcome?;
    stopped?;
    writeln!(io::stdout(), "Stopped")?;
    Ok(())
}

/// A device on network 5 and a client on network 1, each stopped
/// whatever happens in between.
async fn serve_and_read(near: &[u8], far: &[u8]) -> Result<(), Error> {
    let identity = DeviceIdentity::new(DEVICE_INSTANCE, 0)?.with_name("Rooftop unit");
    let mut database = ObjectDatabase::new();
    database.add(Box::new(DeviceObject::new(DeviceConfig {
        identity: identity.clone(),
        ..DeviceConfig::default()
    })?))?;
    let mut temperature = AnalogInputObject::new(
        1,
        "Outdoor air temperature",
        EngineeringUnits::DEGREES_CELSIUS,
    )?;
    temperature.set_present_value(8.5);
    database.add(Box::new(temperature))?;
    let mut server = BACnetServer::bip_builder()
        .device(identity)
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .mode(foreign_device_of(far)?)
        .database(database)
        .build()
        .await?;
    let outcome = async {
        let mut client = BACnetClient::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .broadcast_address(Ipv4Addr::LOCALHOST)
            .mode(foreign_device_of(near)?)
            .build()
            .await?;
        let outcome = async {
            // Broadcasts flow once both registrations are accepted.
            let deadline = tokio::time::Instant::now() + WAIT;
            while !(server.foreign_registration()?.registered
                && client.foreign_registration()?.registered)
            {
                if tokio::time::Instant::now() > deadline {
                    return Err("no foreign-device registration".into());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            read_through_the_router(&client, near).await
        }
        .await;
        client.stop().await?;
        outcome
    }
    .await;
    server.stop().await?;
    outcome
}

async fn read_through_the_router(
    client: &BACnetClient<BipTransport>,
    near: &[u8],
) -> Result<(), Error> {
    // Who-Is-Router-To-Network: which router reaches network 5?
    let routers = client.discover_routers(Some(FAR), WAIT).await?;
    let [router] = routers.as_slice() else {
        return Err(format!("expected one router, heard {}", routers.len()).into());
    };
    if router.mac.as_slice() != near {
        return Err("the answer came from another router".into());
    }
    writeln!(
        io::stdout(),
        "Router to network {FAR} found, reaching networks {:?}",
        router.networks
    )?;

    // A global Who-Is crosses the router; the I-Am names the device's network.
    client
        .who_is(Some(DeviceInstanceRange::new(
            DEVICE_INSTANCE,
            DEVICE_INSTANCE,
        )?))
        .await?;
    let deadline = tokio::time::Instant::now() + WAIT;
    let device = loop {
        if let Some(device) = client.get_device(DEVICE_INSTANCE).await {
            break device;
        }
        if tokio::time::Instant::now() > deadline {
            return Err(format!("device {DEVICE_INSTANCE} did not answer").into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    writeln!(
        io::stdout(),
        "Device {DEVICE_INSTANCE} found on network {}",
        device
            .source_network
            .ok_or("the device answered from this network")?
    )?;

    // Reads by instance go through the router that answered for it.
    let temperature = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)?;
    let ack = client
        .read_property_from_device(
            DEVICE_INSTANCE,
            temperature,
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )
        .await?;
    let (value, _) = decode_application_value(&ack.property_value, 0)?;
    let PropertyValue::Real(value) = value else {
        return Err(format!("unexpected value {value:?}").into());
    };
    writeln!(io::stdout(), "Read through the router: {value}")?;
    Ok(())
}
