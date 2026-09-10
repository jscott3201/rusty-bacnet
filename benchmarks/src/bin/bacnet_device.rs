//! Configurable BACnet device for Docker stress topology.
//!
//! Supports BIP and SC transports. Creates a server with N AnalogInput objects.

use std::net::Ipv4Addr;

#[path = "sc/credentials.rs"]
mod credentials;
#[path = "sc/device.rs"]
mod sc_device;

use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject};
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::{BipTransport, ForeignDeviceConfig};
use bacnet_transport::sc::ScTransport;
use bacnet_transport::sc_tls::TlsWebSocket;
use clap::Parser;

#[derive(Parser)]
#[command(
    name = "bacnet-device",
    version,
    about = "BACnet device for stress testing"
)]
struct Args {
    /// Transport type: bip or sc
    #[arg(long, default_value = "bip")]
    transport: String,

    /// Local interface IP (BIP only)
    #[arg(long, default_value = "0.0.0.0")]
    interface: String,

    /// UDP port (BIP only)
    #[arg(long, default_value_t = 47808)]
    port: u16,

    /// Broadcast address (BIP only)
    #[arg(long, default_value = "255.255.255.255")]
    broadcast: String,

    /// Device instance number
    #[arg(long, default_value_t = 1000)]
    device_instance: u32,

    /// Number of AnalogInput objects to create
    #[arg(long, default_value_t = 100)]
    objects: u32,

    /// Required wss:// hub URL (SC only; TLS 1.3 mutual authentication)
    #[arg(long)]
    sc_hub: Option<String>,

    // Parse only to reject with migration instructions before any I/O.
    #[arg(long, hide = true)]
    sc_no_verify: bool,

    /// Required trusted hub CA PEM file (SC only; no system-root fallback)
    #[arg(long, value_name = "FILE")]
    sc_ca: Option<String>,

    /// Required client certificate chain PEM file, leaf first (SC only)
    #[arg(long, value_name = "FILE")]
    sc_cert: Option<String>,

    /// Required matching client private key PEM file (SC only)
    #[arg(long, value_name = "FILE")]
    sc_key: Option<String>,

    /// Required unique SC VMAC: 12 hex digits, not all zero or all ff
    #[arg(long, value_name = "HEX")]
    sc_vmac: Option<String>,

    /// Required unique nonzero SC Device UUID: 32 hex digits
    #[arg(long, value_name = "HEX")]
    sc_device_uuid: Option<String>,

    /// Register as foreign device at this BBMD address (ip:port)
    #[arg(long)]
    foreign_bbmd: Option<String>,

    /// Foreign device TTL in seconds
    #[arg(long, default_value_t = 300)]
    foreign_ttl: u16,
}

fn make_db(device_instance: u32, object_count: u32) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();

    let device = DeviceObject::new(DeviceConfig {
        instance: device_instance,
        name: format!("Device-{device_instance}"),
        vendor_name: "Rusty BACnet".into(),
        vendor_id: 555,
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(device)).unwrap();

    for i in 1..=object_count {
        let mut ai = AnalogInputObject::new(i, format!("AI-{i}"), 62).unwrap();
        ai.set_present_value(20.0 + (i as f32 * 0.1));
        db.add(Box::new(ai)).unwrap();
    }

    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    db.add(Box::new(ao)).unwrap();

    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    db.add(Box::new(bv)).unwrap();

    db
}

fn format_mac(mac: &[u8]) -> String {
    mac.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.sc_no_verify {
        return Err("--sc-no-verify has been removed; supply --sc-ca, --sc-cert, --sc-key, --sc-hub, --sc-vmac and --sc-device-uuid (see examples/docker/README.md)".into());
    }
    let sc = if args.transport == "sc" {
        Some(sc_device::ScConfig::load(&args)?)
    } else {
        None
    };
    if sc.is_some() {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .init();
    } else {
        tracing_subscriber::fmt::init();
    }
    let db = make_db(args.device_instance, args.objects);

    match args.transport.as_str() {
        "bip" => {
            let interface: Ipv4Addr = args.interface.parse()?;
            let broadcast: Ipv4Addr = args.broadcast.parse()?;

            let mut server = if let Some(ref bbmd_addr) = args.foreign_bbmd {
                let parts: Vec<&str> = bbmd_addr.split(':').collect();
                let bbmd_ip: Ipv4Addr = parts[0].parse()?;
                let bbmd_port: u16 = parts.get(1).unwrap_or(&"47808").parse()?;

                let mut transport = BipTransport::new(interface, args.port, broadcast);
                transport.register_as_foreign_device(ForeignDeviceConfig {
                    bbmd_ip,
                    bbmd_port,
                    ttl: args.foreign_ttl,
                });

                BACnetServer::generic_builder()
                    .transport(transport)
                    .database(db)
                    .build()
                    .await?
            } else {
                BACnetServer::bip_builder()
                    .interface(interface)
                    .port(args.port)
                    .broadcast_address(broadcast)
                    .database(db)
                    .build()
                    .await?
            };

            eprintln!(
                "BIP device {} listening (instance={}, objects={})",
                format_mac(server.local_mac()),
                args.device_instance,
                args.objects
            );

            tokio::signal::ctrl_c().await?;
            server.stop().await?;
        }
        "sc" => {
            let sc = sc.unwrap();
            let hub_url = sc.url;
            let vmac = sc.vmac;
            let ws = TlsWebSocket::connect(hub_url, sc.tls).await?;
            let transport = ScTransport::new(ws, vmac).with_device_uuid(sc.uuid);
            let mut server = BACnetServer::generic_builder()
                .transport(transport)
                .database(db)
                .build()
                .await?;

            eprintln!(
                "SC device connected to {} (instance={}, vmac={}, objects={})",
                hub_url,
                args.device_instance,
                format_mac(&vmac),
                args.objects
            );

            tokio::signal::ctrl_c().await?;
            server.stop().await?;
        }
        other => {
            eprintln!("Unknown transport: {other}. Use 'bip' or 'sc'.");
            std::process::exit(1);
        }
    }

    Ok(())
}
