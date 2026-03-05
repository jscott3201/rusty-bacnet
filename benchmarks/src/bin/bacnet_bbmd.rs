//! BBMD with configurable BDT for Docker stress topology.
//!
//! Starts a BIP transport with BBMD enabled, serving as a BDT peer and
//! accepting foreign device registrations. Also runs a minimal BACnet device.

use std::net::Ipv4Addr;

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bbmd::BdtEntry;
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
    // Add ourselves to BDT
    bdt.push(BdtEntry {
        ip: interface.octets(),
        port: args.port,
        broadcast_mask: [255, 255, 255, 255],
    });

    let mut transport = BipTransport::new(interface, args.port, broadcast);
    transport.enable_bbmd(bdt);

    // Minimal device database
    let mut db = ObjectDatabase::new();
    let device = DeviceObject::new(DeviceConfig {
        instance: args.device_instance,
        name: format!("BBMD-{}", args.device_instance),
        vendor_name: "Rusty BACnet".into(),
        vendor_id: 555,
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(device)).unwrap();

    let mut server = BACnetServer::generic_builder()
        .transport(transport)
        .database(db)
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
