//! Configurable BACnet device for Docker stress topology.
//!
//! Supports BIP and SC transports. Creates a server with N AnalogInput objects.

use std::net::Ipv4Addr;
use std::sync::Arc;

use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject};
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::{BipTransport, ForeignDeviceConfig};
use clap::Parser;

#[derive(Parser)]
#[command(name = "bacnet-device", about = "BACnet device for stress testing")]
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

    /// SC hub URL (SC only)
    #[arg(long)]
    sc_hub: Option<String>,

    /// Skip TLS certificate verification for SC (testing only)
    #[arg(long)]
    sc_no_verify: bool,

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
    tracing_subscriber::fmt::init();
    let args = Args::parse();
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
            let hub_url = args.sc_hub.ok_or("--sc-hub is required for SC transport")?;

            let tls_config = if args.sc_no_verify {
                let config = tokio_rustls::rustls::ClientConfig::builder()
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(NoVerify))
                    .with_no_client_auth();
                Arc::new(config)
            } else {
                let root_store = tokio_rustls::rustls::RootCertStore::empty();
                let config = tokio_rustls::rustls::ClientConfig::builder()
                    .with_root_certificates(root_store)
                    .with_no_client_auth();
                Arc::new(config)
            };

            let vmac: [u8; 6] = rand::random();
            let mut server = BACnetServer::sc_builder()
                .hub_url(&hub_url)
                .tls_config(tls_config)
                .vmac(vmac)
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

/// No-op TLS verifier for testing (skips certificate validation).
#[derive(Debug)]
struct NoVerify;

impl tokio_rustls::rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[tokio_rustls::rustls::pki_types::CertificateDer<'_>],
        _server_name: &tokio_rustls::rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: tokio_rustls::rustls::pki_types::UnixTime,
    ) -> Result<tokio_rustls::rustls::client::danger::ServerCertVerified, tokio_rustls::rustls::Error>
    {
        Ok(tokio_rustls::rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<
        tokio_rustls::rustls::client::danger::HandshakeSignatureValid,
        tokio_rustls::rustls::Error,
    > {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
        _dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<
        tokio_rustls::rustls::client::danger::HandshakeSignatureValid,
        tokio_rustls::rustls::Error,
    > {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
        tokio_rustls::rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
