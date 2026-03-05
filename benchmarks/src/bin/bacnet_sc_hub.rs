//! BACnet/SC Hub for Docker stress topology.
//!
//! Starts a TLS WebSocket hub that relays messages between SC nodes.
//! Supports self-signed certificates for testing or PEM files for production.

use std::sync::Arc;

use bacnet_transport::sc_hub::ScHub;
use clap::Parser;
use rcgen::{CertificateParams, Issuer, KeyPair};
use tokio_rustls::rustls;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::TlsAcceptor;

#[derive(Parser)]
#[command(name = "bacnet-sc-hub", about = "BACnet/SC hub for stress testing")]
struct Args {
    /// Listen address (ip:port)
    #[arg(long, default_value = "0.0.0.0:47809")]
    listen: String,

    /// Generate self-signed certificates (for testing)
    #[arg(long)]
    self_signed: bool,

    /// Path to server certificate PEM file
    #[arg(long)]
    cert: Option<String>,

    /// Path to server private key PEM file
    #[arg(long)]
    key: Option<String>,

    /// Path to CA certificate PEM file (for client auth)
    #[arg(long)]
    ca: Option<String>,
}

fn make_self_signed_acceptor() -> TlsAcceptor {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let mut ca_params =
        CertificateParams::new(Vec::<String>::new()).expect("empty SANs should not fail");
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = KeyPair::generate().unwrap();
    let _ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let ca_issuer = Issuer::from_params(&ca_params, &ca_key);

    let server_params = CertificateParams::new(vec!["localhost".into()]).unwrap();
    let server_key = KeyPair::generate().unwrap();
    let server_cert = server_params.signed_by(&server_key, &ca_issuer).unwrap();

    let cert_chain: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(server_cert.pem().as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
    let key = PrivateKeyDer::from_pem_slice(server_key.serialize_pem().as_bytes()).unwrap();

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)
        .unwrap();

    TlsAcceptor::from(Arc::new(config))
}

fn make_file_acceptor(
    cert_path: &str,
    key_path: &str,
    _ca_path: Option<&str>,
) -> Result<TlsAcceptor, Box<dyn std::error::Error>> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let cert_pem = std::fs::read(cert_path)?;
    let key_pem = std::fs::read(key_path)?;

    let cert_chain: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(&cert_pem).collect::<Result<Vec<_>, _>>()?;
    let key = PrivateKeyDer::from_pem_slice(&key_pem)?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)?;

    Ok(TlsAcceptor::from(Arc::new(config)))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let acceptor = if args.self_signed {
        eprintln!("Generating self-signed certificates");
        make_self_signed_acceptor()
    } else {
        let cert = args
            .cert
            .ok_or("--cert required when not using --self-signed")?;
        let key = args
            .key
            .ok_or("--key required when not using --self-signed")?;
        make_file_acceptor(&cert, &key, args.ca.as_deref())?
    };

    let hub_vmac: [u8; 6] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x01];

    let hub = ScHub::start(&args.listen, acceptor, hub_vmac).await?;
    let addr = hub.local_addr().unwrap();
    eprintln!("BACnet/SC hub listening on {addr}");

    tokio::signal::ctrl_c().await?;
    eprintln!("SC hub shutting down");

    Ok(())
}
