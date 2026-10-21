use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;

use bacnet_client::client::BACnetClient;
use bacnet_transport::bip::{BipMode, BipMulticast, BipTransport, ForeignDeviceConfig};
use bacnet_transport::bip6::Bip6Transport;
use bacnet_types::error::Error;

/// CLI-level transport arguments for constructing a BACnet client.
#[allow(dead_code)]
#[derive(Clone)]
pub struct TransportArgs {
    pub interface: Ipv4Addr,
    pub port: u16,
    pub broadcast: Ipv4Addr,
    pub timeout_ms: u64,
    pub min_interval_ms: u64,
    pub sc: bool,
    pub sc_url: Option<String>,
    /// The failover hub (#962): from a profile, or `--sc-failover-url`
    /// (#1831). It needs `sc_reconnect`.
    pub sc_failover_url: Option<String>,
    /// How a lost hub connection is redialled: from a profile (on by
    /// default), or `--sc-reconnect` / `--sc-failover-url`. `None` leaves a
    /// lost connection down.
    pub sc_reconnect: Option<bacnet_transport::sc::ScReconnectConfig>,
    /// The bound on each hub dial and handshake; `None` keeps the client's
    /// default.
    pub sc_connect_timeout_ms: Option<u64>,
    pub sc_ca: Option<PathBuf>,
    pub sc_cert: Option<PathBuf>,
    pub sc_key: Option<PathBuf>,
    pub sc_vmac: Option<[u8; 6]>,
    pub sc_device_uuid: Option<[u8; 16]>,
    pub ipv6: bool,
    pub ipv6_interface: Option<Ipv6Addr>,
    pub device_instance: Option<u32>,
    /// `--foreign-bbmd`, with `--ttl` and `--renewal` (#938).
    pub foreign_device: Option<ForeignDeviceConfig>,
    /// `--bip-multicast` and `--bip-multicast-ttl` (#941).
    pub multicast: Option<BipMulticast>,
    /// `--mstp` and its settings (#969).
    pub mstp: Option<crate::mstp::MstpArgs>,
}

impl TransportArgs {
    /// Collect the global transport flags, with the BACnet/IP interface and
    /// broadcast address already chosen by the caller.
    pub(crate) fn from_cli(
        cli: &crate::args::Cli,
        interface: Ipv4Addr,
        broadcast: Ipv4Addr,
    ) -> Result<Self, String> {
        let ipv6_interface = cli
            .ipv6_interface
            .as_deref()
            .map(|s| {
                s.parse::<Ipv6Addr>()
                    .map_err(|e| format!("invalid --ipv6-interface address '{s}': {e}"))
            })
            .transpose()?;
        if cli.foreign_bbmd.is_some() && (cli.sc || cli.ipv6) {
            return Err(
                "--foreign-bbmd requires BACnet/IP transport (do not use --sc or --ipv6)".into(),
            );
        }
        let multicast = crate::multicast::from_cli(cli, interface)?;
        if let (Some(failover), Some(primary)) = (&cli.sc_failover_url, &cli.sc_url) {
            if same_hub_url(failover, primary) {
                return Err("--sc-failover-url must differ from --sc-url".into());
            }
        }
        Ok(Self {
            interface,
            port: cli.port,
            broadcast,
            timeout_ms: cli.timeout,
            min_interval_ms: cli.min_interval_ms,
            sc: cli.sc,
            sc_url: cli.sc_url.clone(),
            sc_failover_url: cli.sc_failover_url.clone(),
            // A failover hub is only reached by reconnecting, so it brings
            // the default policy along.
            sc_reconnect: (cli.sc_reconnect || cli.sc_failover_url.is_some())
                .then(bacnet_transport::sc::ScReconnectConfig::default),
            sc_connect_timeout_ms: cli.sc_connect_timeout,
            sc_ca: cli.sc_ca.clone(),
            sc_cert: cli.sc_cert.clone(),
            sc_key: cli.sc_key.clone(),
            sc_vmac: cli.sc_vmac,
            sc_device_uuid: cli.sc_device_uuid,
            ipv6: cli.ipv6,
            ipv6_interface,
            device_instance: cli.device_instance,
            foreign_device: crate::foreign::config(cli.foreign_bbmd, cli.ttl, cli.renewal),
            multicast,
            mstp: crate::mstp::MstpArgs::from_cli(cli)?,
        })
    }
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
        .min_request_interval_ms(args.min_interval_ms)
        .mode(
            args.foreign_device
                .clone()
                .map_or(BipMode::Normal, BipMode::Foreign),
        )
        .multicast(args.multicast)
        .build()
        .await
}

/// Build a BACnet/SC client from CLI transport arguments.
///
/// Loads TLS certificates and private key from PEM files, constructs a TLS
/// configuration trusting only the explicit site CA PEM, and builds the SC client.
#[cfg(feature = "sc-tls")]
pub async fn build_sc_client(
    args: &TransportArgs,
) -> Result<
    BACnetClient<bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>>,
    Error,
> {
    sc_client_builder(args)?.build().await
}

/// The BACnet/SC client builder for CLI transport arguments, before it
/// dials: the TLS files loaded and linted, the hubs and identity set.
#[cfg(feature = "sc-tls")]
pub fn sc_client_builder(
    args: &TransportArgs,
) -> Result<bacnet_client::client::ScClientBuilder, Error> {
    use bacnet_transport::sc_tls::pem;
    use bacnet_transport::sc_tls::ScNodeTlsConfig;
    use rustls::RootCertStore;
    use rustls_pki_types::pem::Error as PemError;
    use std::fs::File;

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
    let ca_path = args
        .sc_ca
        .as_ref()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| {
            Error::Encoding("--sc-ca <FILE> is required for BACnet/SC; supply a nonempty site CA PEM path (no system-root fallback)".into())
        })?;

    let mut root_store = RootCertStore::empty();
    let ca_file = File::open(ca_path)
        .map_err(|e| Error::Encoding(format!("failed to read --sc-ca PEM: {}", PemError::Io(e))))?;
    let ca_iter = pem::certificate_reader(ca_file);
    let mut ca_certs = Vec::new();
    // Preserve CA error precedence before identity file I/O. The factory below
    // owns the actual TLS policy; this store only validates the loaded input.
    for cert in ca_iter {
        let cert =
            cert.map_err(|e| Error::Encoding(format!("failed to parse --sc-ca PEM: {e}")))?;
        root_store
            .add(cert.clone())
            .map_err(|e| Error::Encoding(format!("unusable certificate in --sc-ca PEM: {e}")))?;
        ca_certs.push(cert);
    }
    if root_store.is_empty() {
        return Err(Error::Encoding(
            "--sc-ca PEM contains no certificates; supply the trusted site CA certificate(s)"
                .into(),
        ));
    }

    let cert_file = File::open(cert_path)
        .map_err(|e| Error::Encoding(format!("failed to read cert PEM: {}", PemError::Io(e))))?;
    let certs = pem::certificate_reader(cert_file)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::Encoding(format!("failed to parse cert PEM: {e}")))?;
    let key = File::open(key_path)
        .map_err(PemError::Io)
        .and_then(pem::private_key_reader)
        .map_err(|e| Error::Encoding(format!("failed to read key PEM: {e}")))?;

    // The pre-dial lint (#968), for one-shots and the TUI alike:
    // a hard failure stops here, before any socket. Warnings are logged (to
    // stderr in one-shots) and never block; the TUI confirms them earlier, so
    // a warning that first appears between that check and this one is only
    // logged (accepted by design). Inputs past the inspection limits are a
    // warning, not a refusal: the loaders above already accepted them.
    let lint = crate::sc_check::lint::Lint::run(cert_path, Some(ca_path), Some(key_path), None)
        .map_err(Error::Encoding)?;
    if let Some(refusal) = lint.refusal() {
        return Err(Error::Encoding(refusal));
    }
    for warning in lint.of(crate::sc_check::lint::Weight::Warning) {
        tracing::warn!("{}", warning.text);
    }

    let tls_config = ScNodeTlsConfig::from_der(ca_certs, certs, key)
        .map_err(|e| Error::Encoding(format!("TLS config error: {e}")))?;

    let mut builder = BACnetClient::sc_builder()
        .hub_url(hub_url)
        .tls_config(tls_config)
        .vmac(sc_vmac)
        .device_uuid(sc_device_uuid)
        .apdu_timeout_ms(args.timeout_ms)
        .min_request_interval_ms(args.min_interval_ms);
    builder = builder.connect_timeout_ms(
        args.sc_connect_timeout_ms
            .unwrap_or(DEFAULT_SC_CONNECT_TIMEOUT_MS),
    );
    if let Some(policy) = args.sc_reconnect.clone() {
        builder = builder.reconnect(policy);
    }
    if let Some(failover) = &args.sc_failover_url {
        // The builder refuses a failover hub without a reconnect policy.
        builder = builder.failover_hub_url(failover);
    }
    Ok(builder)
}

/// The connect timeout the CLI gives the SC client builder when none is
/// set: 10 s, the builder's own default (`ScClientBuilder::connect_timeout_ms`),
/// and a profile's.
#[cfg_attr(not(any(feature = "tui", feature = "sc-tls")), allow(dead_code))]
pub(crate) const DEFAULT_SC_CONNECT_TIMEOUT_MS: u64 = 10_000;

/// Whether two hub URLs name the same hub: compared with the scheme and
/// host lowercased and a trailing slash ignored.
pub(crate) fn same_hub_url(a: &str, b: &str) -> bool {
    fn normalized(url: &str) -> String {
        let url = url.trim();
        let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        let tail = tail.strip_suffix('/').unwrap_or(tail);
        format!(
            "{}://{}{tail}",
            scheme.to_ascii_lowercase(),
            authority.to_ascii_lowercase()
        )
    }
    normalized(a) == normalized(b)
}

/// The longest connect timeout a profile or `--sc-connect-timeout` takes.
pub(crate) const MAX_SC_CONNECT_TIMEOUT_MS: u64 = 600_000;

/// Why an SC connect timeout is out of range, if it is: zero would fail
/// every dial at once.
pub(crate) fn check_sc_connect_timeout(ms: u64) -> Result<(), String> {
    if (1..=MAX_SC_CONNECT_TIMEOUT_MS).contains(&ms) {
        Ok(())
    } else {
        Err(format!(
            "must be 1 to {MAX_SC_CONNECT_TIMEOUT_MS} ms, got {ms}"
        ))
    }
}

/// `--sc-connect-timeout`: milliseconds, 1 to [`MAX_SC_CONNECT_TIMEOUT_MS`].
pub(crate) fn parse_sc_connect_timeout_arg(s: &str) -> Result<u64, String> {
    let ms = s
        .parse::<u64>()
        .map_err(|_| format!("'{s}' is not a number of milliseconds"))?;
    check_sc_connect_timeout(ms)?;
    Ok(ms)
}

/// Build a BACnet/IPv6 (BIP6) client from CLI transport arguments.
pub async fn build_bip6_client(args: &TransportArgs) -> Result<BACnetClient<Bip6Transport>, Error> {
    let ipv6_addr = args.ipv6_interface.unwrap_or(Ipv6Addr::UNSPECIFIED);

    let mut builder = BACnetClient::bip6_builder()
        .interface(ipv6_addr)
        .port(args.port)
        .apdu_timeout_ms(args.timeout_ms)
        .min_request_interval_ms(args.min_interval_ms);

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
            min_interval_ms: 0,
            sc: true,
            sc_url: Some("wss://hub.example.com/bacnet".into()),
            sc_failover_url: None,
            sc_reconnect: None,
            sc_connect_timeout_ms: None,
            sc_ca: Some(PathBuf::from("site-ca.pem")),
            sc_cert: Some(PathBuf::from("cert.pem")),
            sc_key: Some(PathBuf::from("key.pem")),
            sc_vmac: Some([0x22, 0x01, 0x02, 0x03, 0x04, 0x05]),
            sc_device_uuid: Some([0xAB; 16]),
            ipv6: false,
            ipv6_interface: None,
            device_instance: None,
            foreign_device: None,
            multicast: None,
            mstp: None,
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
