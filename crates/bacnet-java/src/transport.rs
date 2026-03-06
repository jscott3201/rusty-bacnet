use crate::errors::BacnetError;
use crate::types::TransportConfig;

use std::net::Ipv4Addr;

use bacnet_transport::any::AnyTransport;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::mstp::NoSerial;

/// Parse an address string to MAC bytes for the client.
pub(crate) fn parse_address(address: &str) -> Result<Vec<u8>, BacnetError> {
    // Try IPv4 "ip:port" format
    if let Some((ip_str, port_str)) = address.rsplit_once(':') {
        if let (Ok(ip), Ok(port)) = (ip_str.parse::<Ipv4Addr>(), port_str.parse::<u16>()) {
            let octets = ip.octets();
            let port_bytes = port.to_be_bytes();
            return Ok(vec![
                octets[0],
                octets[1],
                octets[2],
                octets[3],
                port_bytes[0],
                port_bytes[1],
            ]);
        }
    }

    // Try hex MAC "aa:bb:cc:..." format
    let parts: Result<Vec<u8>, _> = address
        .split(':')
        .map(|s| u8::from_str_radix(s, 16))
        .collect();
    if let Ok(mac) = parts {
        if !mac.is_empty() {
            return Ok(mac);
        }
    }

    Err(BacnetError::InvalidArgument {
        msg: format!("cannot parse address: {address}"),
    })
}

/// Build an AnyTransport from a TransportConfig (BIP only for synchronous creation).
///
/// SC requires async TLS setup, handled separately in client/server connect.
pub(crate) fn build_transport(
    config: &TransportConfig,
) -> Result<AnyTransport<NoSerial>, BacnetError> {
    match config {
        TransportConfig::Bip {
            address,
            port,
            broadcast_address,
        } => {
            let interface: Ipv4Addr =
                address.parse().map_err(|e| BacnetError::InvalidArgument {
                    msg: format!("invalid interface address: {e}"),
                })?;
            let broadcast: Ipv4Addr =
                broadcast_address
                    .parse()
                    .map_err(|e| BacnetError::InvalidArgument {
                        msg: format!("invalid broadcast address: {e}"),
                    })?;
            Ok(AnyTransport::Bip(BipTransport::new(
                interface, *port, broadcast,
            )))
        }
        TransportConfig::BipIpv6 { address, port } => {
            let interface: std::net::Ipv6Addr =
                address.parse().map_err(|e| BacnetError::InvalidArgument {
                    msg: format!("invalid IPv6 address: {e}"),
                })?;
            Ok(AnyTransport::Bip6(
                bacnet_transport::bip6::Bip6Transport::new(interface, *port, None),
            ))
        }
        TransportConfig::Sc { .. } => Err(BacnetError::InvalidArgument {
            msg: "SC transport requires async setup — use connect() directly".into(),
        }),
        TransportConfig::Mstp { .. } => Err(BacnetError::InvalidArgument {
            msg: "MS/TP transport is not supported on this platform".into(),
        }),
    }
}

/// Validate a TransportConfig and return an informational string.
#[allow(dead_code)]
pub(crate) fn validate_config(config: &TransportConfig) -> Result<String, BacnetError> {
    match config {
        TransportConfig::Bip {
            address,
            port,
            broadcast_address,
        } => {
            address
                .parse::<Ipv4Addr>()
                .map_err(|e| BacnetError::InvalidArgument {
                    msg: format!("invalid BIP address: {e}"),
                })?;
            broadcast_address
                .parse::<Ipv4Addr>()
                .map_err(|e| BacnetError::InvalidArgument {
                    msg: format!("invalid broadcast address: {e}"),
                })?;
            Ok(format!("BIP {address}:{port}"))
        }
        TransportConfig::BipIpv6 { address, port } => Ok(format!("BIP/IPv6 [{address}]:{port}")),
        TransportConfig::Sc { hub_url, .. } => {
            if !hub_url.starts_with("wss://") && !hub_url.starts_with("ws://") {
                return Err(BacnetError::InvalidArgument {
                    msg: "SC hub URL must start with ws:// or wss://".into(),
                });
            }
            Ok(format!("SC {hub_url}"))
        }
        TransportConfig::Mstp {
            serial_port,
            baud_rate,
            mac_address,
        } => {
            if *mac_address > 127 {
                return Err(BacnetError::InvalidArgument {
                    msg: "MS/TP MAC address must be 0-127".into(),
                });
            }
            Ok(format!(
                "MS/TP {serial_port} @ {baud_rate} baud, MAC {mac_address}"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ipv4_address() {
        let mac = parse_address("192.168.1.100:47808").unwrap();
        assert_eq!(mac, vec![192, 168, 1, 100, 0xBA, 0xC0]);
    }

    #[test]
    fn test_parse_hex_mac() {
        let mac = parse_address("0a:0b:0c:0d:ba:c0").unwrap();
        assert_eq!(mac, vec![0x0a, 0x0b, 0x0c, 0x0d, 0xba, 0xc0]);
    }

    #[test]
    fn test_parse_invalid_address() {
        let result = parse_address("not-valid");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_bip_config() {
        let cfg = TransportConfig::Bip {
            address: "0.0.0.0".into(),
            port: 47808,
            broadcast_address: "255.255.255.255".into(),
        };
        assert!(validate_config(&cfg).is_ok());
    }

    #[test]
    fn test_validate_bip_bad_address() {
        let cfg = TransportConfig::Bip {
            address: "not-ip".into(),
            port: 47808,
            broadcast_address: "255.255.255.255".into(),
        };
        assert!(validate_config(&cfg).is_err());
    }

    #[test]
    fn test_validate_sc_config() {
        let cfg = TransportConfig::Sc {
            hub_url: "wss://hub.example.com".into(),
            ca_cert: None,
            client_cert: None,
            client_key: None,
            heartbeat_interval_ms: None,
            heartbeat_timeout_ms: None,
        };
        assert!(validate_config(&cfg).is_ok());
    }

    #[test]
    fn test_validate_sc_bad_url() {
        let cfg = TransportConfig::Sc {
            hub_url: "http://wrong".into(),
            ca_cert: None,
            client_cert: None,
            client_key: None,
            heartbeat_interval_ms: None,
            heartbeat_timeout_ms: None,
        };
        assert!(validate_config(&cfg).is_err());
    }

    #[test]
    fn test_validate_mstp_bad_mac() {
        let cfg = TransportConfig::Mstp {
            serial_port: "/dev/ttyUSB0".into(),
            baud_rate: 9600,
            mac_address: 200,
        };
        assert!(validate_config(&cfg).is_err());
    }
}
