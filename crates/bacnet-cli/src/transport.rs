use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;

use bacnet_client::client::BACnetClient;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::bip6::Bip6Transport;
use bacnet_types::error::Error;

/// CLI-level transport arguments for constructing a BACnet client.
#[allow(dead_code)]
pub struct TransportArgs {
    pub interface: Ipv4Addr,
    pub port: u16,
    pub broadcast: Ipv4Addr,
    pub timeout_ms: u64,
    pub sc: bool,
    pub sc_url: Option<String>,
    pub sc_cert: Option<PathBuf>,
    pub sc_key: Option<PathBuf>,
    pub ipv6: bool,
    pub ipv6_interface: Option<Ipv6Addr>,
    pub device_instance: Option<u32>,
}

/// Build a BACnet/IP (BIP) client from CLI transport arguments.
pub async fn build_bip_client(args: &TransportArgs) -> Result<BACnetClient<BipTransport>, Error> {
    BACnetClient::bip_builder()
        .interface(args.interface)
        .port(args.port)
        .broadcast_address(args.broadcast)
        .apdu_timeout_ms(args.timeout_ms)
        .build()
        .await
}

/// Build a BACnet/SC client from CLI transport arguments.
///
/// Loads TLS certificates and private key from PEM files, constructs a TLS
/// configuration using native root certificates, and builds the SC client.
#[cfg(feature = "sc-tls")]
pub async fn build_sc_client(
    args: &TransportArgs,
) -> Result<
    BACnetClient<bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>>,
    Error,
> {
    use std::sync::Arc;

    use rustls::RootCertStore;
    use rustls_pki_types::pem::PemObject;
    use rustls_pki_types::{CertificateDer, PrivateKeyDer};

    let cert_path = args
        .sc_cert
        .as_ref()
        .ok_or_else(|| Error::Encoding("--sc-cert is required for BACnet/SC".into()))?;
    let key_path = args
        .sc_key
        .as_ref()
        .ok_or_else(|| Error::Encoding("--sc-key is required for BACnet/SC".into()))?;
    let hub_url = args
        .sc_url
        .as_deref()
        .ok_or_else(|| Error::Encoding("--sc-url is required for BACnet/SC".into()))?;

    let certs = CertificateDer::pem_file_iter(cert_path)
        .map_err(|e| Error::Encoding(format!("failed to read cert PEM: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::Encoding(format!("failed to parse cert PEM: {e}")))?;
    let key = PrivateKeyDer::from_pem_file(key_path)
        .map_err(|e| Error::Encoding(format!("failed to read key PEM: {e}")))?;

    let mut root_store = RootCertStore::empty();
    let native_certs = rustls_native_certs::load_native_certs();
    for cert in native_certs.certs {
        root_store
            .add(cert)
            .map_err(|e| Error::Encoding(format!("failed to add native root cert: {e}")))?;
    }
    if root_store.is_empty() {
        return Err(Error::Encoding(
            "no native root certificates found — TLS connections will fail".into(),
        ));
    }

    let tls_config =
        rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .with_root_certificates(root_store)
            .with_client_auth_cert(certs, key)
            .map_err(|e| Error::Encoding(format!("TLS config error: {e}")))?;

    BACnetClient::sc_builder()
        .hub_url(hub_url)
        .tls_config(Arc::new(tls_config))
        .apdu_timeout_ms(args.timeout_ms)
        .build()
        .await
}

/// Build a BACnet/IPv6 (BIP6) client from CLI transport arguments.
pub async fn build_bip6_client(args: &TransportArgs) -> Result<BACnetClient<Bip6Transport>, Error> {
    let ipv6_addr = args.ipv6_interface.unwrap_or(Ipv6Addr::UNSPECIFIED);

    let mut builder = BACnetClient::bip6_builder()
        .interface(ipv6_addr)
        .port(args.port)
        .apdu_timeout_ms(args.timeout_ms);

    if let Some(instance) = args.device_instance {
        builder = builder.device_instance(instance);
    }

    builder.build().await
}
