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
    pub sc_vmac: Option<[u8; 6]>,
    pub sc_device_uuid: Option<[u8; 16]>,
    pub ipv6: bool,
    pub ipv6_interface: Option<Ipv6Addr>,
    pub device_instance: Option<u32>,
}

fn parse_fixed_hex_array<const N: usize>(
    value: &str,
    label: &str,
    separators: &[char],
) -> Result<[u8; N], String> {
    let compact: String = value
        .trim()
        .chars()
        .filter(|ch| !separators.contains(ch))
        .collect();
    let expected_len = N * 2;
    if compact.len() != expected_len {
        return Err(format!("{label} must contain exactly {N} hex bytes"));
    }
    if !compact.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err(format!("{label} contains non-hex characters"));
    }

    let mut bytes = [0u8; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let start = index * 2;
        *byte = u8::from_str_radix(&compact[start..start + 2], 16)
            .map_err(|e| format!("failed to parse {label}: {e}"))?;
    }
    Ok(bytes)
}

/// Parse a BACnet/SC VMAC CLI argument.
pub fn parse_sc_vmac_arg(value: &str) -> Result<[u8; 6], String> {
    let vmac = parse_fixed_hex_array::<6>(value, "SC VMAC", &[':', '-'])?;
    match vmac {
        bacnet_transport::sc_frame::UNKNOWN_VMAC => {
            Err("--sc-vmac must not be the reserved unknown VMAC".into())
        }
        bacnet_transport::sc_frame::BROADCAST_VMAC => {
            Err("--sc-vmac must not be the reserved broadcast VMAC".into())
        }
        _ => Ok(vmac),
    }
}

/// Parse a BACnet/SC device UUID CLI argument.
pub fn parse_sc_device_uuid_arg(value: &str) -> Result<[u8; 16], String> {
    let uuid = parse_fixed_hex_array::<16>(value, "SC device UUID", &['-'])?;
    if uuid == [0; 16] {
        return Err("--sc-device-uuid must not be all zero".into());
    }
    Ok(uuid)
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
    let sc_vmac = args
        .sc_vmac
        .ok_or_else(|| Error::Encoding("--sc-vmac is required for BACnet/SC".into()))?;
    let sc_device_uuid = args
        .sc_device_uuid
        .ok_or_else(|| Error::Encoding("--sc-device-uuid is required for BACnet/SC".into()))?;

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
        .vmac(sc_vmac)
        .device_uuid(sc_device_uuid)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "sc-tls")]
    fn sc_args() -> TransportArgs {
        TransportArgs {
            interface: Ipv4Addr::UNSPECIFIED,
            port: 0xBAC0,
            broadcast: Ipv4Addr::BROADCAST,
            timeout_ms: 6000,
            sc: true,
            sc_url: Some("wss://hub.example.com/bacnet".into()),
            sc_cert: Some(PathBuf::from("cert.pem")),
            sc_key: Some(PathBuf::from("key.pem")),
            sc_vmac: Some([0x22, 0x01, 0x02, 0x03, 0x04, 0x05]),
            sc_device_uuid: Some([0xAB; 16]),
            ipv6: false,
            ipv6_interface: None,
            device_instance: None,
        }
    }

    #[test]
    fn parse_sc_vmac_arg_accepts_compact_and_separated_hex() {
        assert_eq!(
            parse_sc_vmac_arg("220102030405").unwrap(),
            [0x22, 0x01, 0x02, 0x03, 0x04, 0x05]
        );
        assert_eq!(
            parse_sc_vmac_arg("22:01:02:03:04:05").unwrap(),
            [0x22, 0x01, 0x02, 0x03, 0x04, 0x05]
        );
    }

    #[test]
    fn parse_sc_vmac_arg_rejects_reserved_values() {
        assert!(parse_sc_vmac_arg("00:00:00:00:00:00").is_err());
        assert!(parse_sc_vmac_arg("ff:ff:ff:ff:ff:ff").is_err());
    }

    #[test]
    fn parse_sc_device_uuid_arg_accepts_hyphenated_uuid() {
        assert_eq!(
            parse_sc_device_uuid_arg("00112233-4455-6677-8899-aabbccddeeff").unwrap(),
            [
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD,
                0xEE, 0xFF,
            ]
        );
    }

    #[test]
    fn parse_sc_device_uuid_arg_rejects_zero_uuid() {
        assert!(parse_sc_device_uuid_arg("00000000-0000-0000-0000-000000000000").is_err());
    }

    #[cfg(feature = "sc-tls")]
    #[tokio::test]
    async fn build_sc_client_requires_vmac_before_loading_tls_files() {
        let mut args = sc_args();
        args.sc_vmac = None;

        let result = build_sc_client(&args).await;
        assert!(matches!(
            result,
            Err(Error::Encoding(message)) if message.contains("--sc-vmac is required")
        ));
    }

    #[cfg(feature = "sc-tls")]
    #[tokio::test]
    async fn build_sc_client_requires_device_uuid_before_loading_tls_files() {
        let mut args = sc_args();
        args.sc_device_uuid = None;

        let result = build_sc_client(&args).await;
        assert!(matches!(
            result,
            Err(Error::Encoding(message)) if message.contains("--sc-device-uuid is required")
        ));
    }
}
