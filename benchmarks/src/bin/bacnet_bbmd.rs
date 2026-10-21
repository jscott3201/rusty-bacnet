//! BBMD with configurable BDT for Docker stress topology.
//!
//! Starts a BIP transport with BBMD enabled, serving as a BDT peer and
//! accepting foreign device registrations. Also runs a minimal BACnet device.
#![allow(clippy::print_stdout, clippy::print_stderr)] // benchmark binaries report progress and results on the console
#![forbid(unsafe_code)]

use std::net::Ipv4Addr;

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::{BACnetServer, DiscoveryPolicy};
use bacnet_transport::bbmd::{BdtEntry, ForeignDevicePolicy, NatTraversal};
use bacnet_transport::bip::BipTransport;
use clap::Parser;

#[derive(Parser)]
#[command(name = "bacnet-bbmd", about = "BACnet BBMD for stress testing")]
struct Args {
    /// Local interface IP
    #[arg(long, default_value = "0.0.0.0")]
    interface: String,

    /// UDP port
    #[arg(long, default_value_t = 47808)]
    port: u16,

    /// Broadcast address
    #[arg(long, default_value = "255.255.255.255")]
    broadcast: String,

    /// Comma-separated BDT peers (ip:port)
    #[arg(long)]
    bdt: Option<String>,

    /// Device instance number for the BBMD's device object
    #[arg(long, default_value_t = 9000)]
    device_instance: u32,

    /// Run behind a NAT router that port-forwards this public ip:port to
    /// the BBMD (Annex J.7.5). The --bdt rows must then list it, with every
    /// site's public address, and no own row is added.
    #[arg(long)]
    nat_global: Option<std::net::SocketAddrV4>,

    /// The BBMD is the only device of its B/IP network on its subnet.
    #[arg(long)]
    alone_on_subnet: bool,

    /// Answer a Who-Is with a broadcast I-Am, which the BBMDs carry to
    /// every foreign device, not a unicast one to the asker. When the
    /// asker sits behind a router with no port forward, only its own BBMD
    /// can get datagrams to it, so a unicast I-Am from another site is
    /// dropped on the way.
    #[arg(long)]
    broadcast_i_am: bool,
}

fn parse_bdt_entry(spec: &str) -> Result<BdtEntry, Box<dyn std::error::Error>> {
    let parts: Vec<&str> = spec.split(':').collect();
    let ip: Ipv4Addr = parts[0].parse()?;
    let port: u16 = parts.get(1).unwrap_or(&"47808").parse()?;
    Ok(BdtEntry {
        ip: ip.octets(),
        port,
        broadcast_mask: [255, 255, 255, 255],
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let interface: Ipv4Addr = args.interface.parse()?;
    let broadcast: Ipv4Addr = args.broadcast.parse()?;

    // Parse BDT entries
    let mut bdt = Vec::new();
    if let Some(ref bdt_str) = args.bdt {
        for entry_str in bdt_str.split(',') {
            bdt.push(parse_bdt_entry(entry_str.trim())?);
        }
    }
    // Add our own row. A 0.0.0.0 row is invalid, so a wildcard-bound BBMD
    // instead takes its own address from a --bdt row at one of the host's
    // addresses, or else from the default route, and adds its row itself.
    if !interface.is_unspecified() && args.nat_global.is_none() {
        bdt.push(BdtEntry {
            ip: interface.octets(),
            port: args.port,
            broadcast_mask: [255, 255, 255, 255],
        });
    }

    let mut transport = BipTransport::new(interface, args.port, broadcast);
    transport.enable_bbmd(bdt);
    transport.enable_foreign_device_registration(ForeignDevicePolicy::default());
    transport.set_bbmd_alone_on_subnet(args.alone_on_subnet);
    transport.set_bbmd_nat_traversal(args.nat_global.map(NatTraversal::new));

    // Minimal device database
    let mut db = ObjectDatabase::new();
    let device = DeviceObject::new(DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(args.device_instance, 555)
            .expect("valid Device identity")
            .with_name(format!("BBMD-{}", args.device_instance)),
        vendor_name: "Rusty BACnet".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(device)).unwrap();

    let mut server = BACnetServer::generic_builder()
        .device(bacnet_objects::DeviceIdentity::new(
            args.device_instance,
            555,
        )?)
        .transport(transport)
        .database(db)
        .discovery_policy(DiscoveryPolicy {
            prefer_directed_responses: !args.broadcast_i_am,
            ..DiscoveryPolicy::default()
        })
        .build()
        .await?;

    eprintln!(
        "BBMD listening on {}:{} (instance={})",
        args.interface, args.port, args.device_instance
    );

    tokio::signal::ctrl_c().await?;
    server.stop().await?;

    Ok(())
}
