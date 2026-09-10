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
    let material = Credentials::load(ca, cert, key, ["--ca", "--cert", "--key"])?;
    let tls = ScHubTlsConfig::from_der(material.ca, material.chain, material.key)?;

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    // Retain the standalone hub's existing identity and handshake budgets.
    let mut hub = ScHub::start_with_tls_config(
        &args.listen,
        tls,
        [0, 0, 0, 0, 0, 1],
        [0; 16],
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
