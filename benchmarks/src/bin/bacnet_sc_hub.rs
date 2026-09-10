//! BACnet/SC hub for Docker stress topology with required caller-provided mTLS.

#[path = "sc/credentials.rs"]
mod credentials;

use bacnet_transport::sc_hub::{ScHub, ScHubHandshakeTimeouts, ScHubTlsConfig};
use clap::Parser;
use credentials::{required, Credentials};

#[derive(Parser)]
#[command(
    name = "bacnet-sc-hub",
    version,
    about = "BACnet/SC mutual-TLS 1.3 hub for stress testing"
)]
struct Args {
    /// Listen address (ip:port)
    #[arg(long, default_value = "0.0.0.0:47809")]
    listen: String,

    // Parse only for an actionable migration error, never an insecure mode.
    #[arg(long, hide = true)]
    self_signed: bool,

    /// Required server certificate chain PEM file (leaf first)
    #[arg(long, value_name = "FILE")]
    cert: Option<String>,

    /// Required matching server private key PEM file
    #[arg(long, value_name = "FILE")]
    key: Option<String>,

    /// Required trusted client CA certificate PEM file (no system-root fallback)
    #[arg(long, value_name = "FILE")]
    ca: Option<String>,

    /// Required hosting device UUID: 32 ASCII hex digits, nonzero, no separators.
    /// Provision before deployment and durably reuse for the device's lifetime.
    #[arg(long, value_name = "HEX")]
    device_uuid: Option<String>,

    /// Hosting port VMAC: 12 ASCII hex digits; not all zero or all ff
    #[arg(long, default_value = "000000000001", value_name = "HEX")]
    vmac: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.self_signed {
        return Err("--self-signed has been removed; supply --ca, --cert and --key PEM files (see examples/docker/README.md)".into());
    }
    let ca = required(args.ca.as_deref(), "--ca")?;
    let cert = required(args.cert.as_deref(), "--cert")?;
    let key = required(args.key.as_deref(), "--key")?;
    let uuid = hex(
        required(args.device_uuid.as_deref(), "--device-uuid")?,
        "--device-uuid",
    )?;
    if uuid == [0; 16] {
        return Err("--device-uuid must be nonzero; provision and durably reuse the hosting device's lifetime UUID (see examples/docker/README.md)".into());
    }
    let vmac = hex(&args.vmac, "--vmac")?;
    if vmac == [0; 6] || vmac == [0xff; 6] {
        return Err("--vmac must not be unknown (all zero) or broadcast (all ff)".into());
    }
    let material = Credentials::load(ca, cert, key, ["--ca", "--cert", "--key"])?;
    let tls = ScHubTlsConfig::from_der(material.ca, material.chain, material.key)?;

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    // Retain the existing handshake budgets; identity is caller-provisioned.
    let mut hub = ScHub::start_with_tls_config(
        &args.listen,
        tls,
        vmac,
        uuid,
        ScHubHandshakeTimeouts::default(),
    )
    .await?;
    let addr = hub.local_addr().unwrap();
    eprintln!("BACnet/SC hub listening on {addr}");

    let signal = tokio::signal::ctrl_c().await;
    eprintln!("SC hub shutting down");
    hub.stop().await;
    signal?;
    Ok(())
}

fn hex<const N: usize>(text: &str, flag: &str) -> Result<[u8; N], String> {
    // Same input grammar as the standalone device; do not slice arbitrary UTF-8.
    if text.len() != N * 2 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!(
            "{flag}: expected {} hexadecimal digits without separators",
            N * 2
        ));
    }
    let mut bytes = [0; N];
    for (byte, pair) in bytes.iter_mut().zip(text.as_bytes().chunks_exact(2)) {
        let digit = |b: u8| {
            if b.is_ascii_digit() {
                b - b'0'
            } else {
                b.to_ascii_lowercase() - b'a' + 10
            }
        };
        *byte = digit(pair[0]) * 16 + digit(pair[1]);
    }
    Ok(bytes)
}
