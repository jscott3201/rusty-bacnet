use std::net::{Ipv4Addr, Ipv6Addr};

/// A parsed target for a BACnet command.
pub enum Target {
    /// Direct MAC address (already resolved).
    Mac(Vec<u8>),
    /// Device instance number (needs discovery lookup).
    Instance(u32),
    /// Routed device: (dnet, device_instance).
    Routed(u16, u32),
}

/// Encode an IPv4 address and port as a 6-byte BIP MAC address.
///
/// The format is 4 bytes of IPv4 address followed by 2 bytes of port in
/// big-endian byte order.
pub fn ip_to_bip_mac(ip: Ipv4Addr, port: u16) -> Vec<u8> {
    let mut mac = Vec::with_capacity(6);
    mac.extend_from_slice(&ip.octets());
    mac.extend_from_slice(&port.to_be_bytes());
    mac
}

/// Parse a target string into a [`Target`].
///
/// Supported formats:
/// - `dnet:instance` — routed device (e.g., `2:1234`)
/// - `ip` or `ip:port` — direct BIP MAC (e.g., `192.168.1.100` or `192.168.1.100:47808`)
/// - `instance` — device instance number (e.g., `1234`)
pub fn parse_target(s: &str) -> Result<Target, String> {
    // Try IPv6 bracket notation: [addr]:port or [addr]
    if s.starts_with('[') {
        if let Some(bracket_end) = s.find(']') {
            let ip_str = &s[1..bracket_end];
            let ip: Ipv6Addr = ip_str
                .parse()
                .map_err(|_| format!("invalid IPv6 address in '{s}'"))?;
            let port = if s.len() > bracket_end + 1 && s.as_bytes()[bracket_end + 1] == b':' {
                s[bracket_end + 2..]
                    .parse::<u16>()
                    .map_err(|_| format!("invalid port in target '{s}'"))?
            } else {
                0xBAC0
            };
            let mac = bacnet_transport::bip6::encode_bip6_mac(ip, port);
            return Ok(Target::Mac(mac.to_vec()));
        }
    }

    // Try parsing as dnet:instance (both parts must be numeric).
    if let Some((left, right)) = s.split_once(':') {
        if let (Ok(dnet), Ok(instance)) = (left.parse::<u16>(), right.parse::<u32>()) {
            return Ok(Target::Routed(dnet, instance));
        }
        // If only the left side fails as u16, it might be an IP:port.
    }

    // Try parsing as IP address with optional port.
    if let Some((ip_str, port_str)) = s.split_once(':') {
        if let Ok(ip) = ip_str.parse::<Ipv4Addr>() {
            let port = port_str
                .parse::<u16>()
                .map_err(|_| format!("invalid port in target '{s}'"))?;
            return Ok(Target::Mac(ip_to_bip_mac(ip, port)));
        }
    }

    // Try parsing as bare IP address (no port).
    if let Ok(ip) = s.parse::<Ipv4Addr>() {
        return Ok(Target::Mac(ip_to_bip_mac(ip, 0xBAC0)));
    }

    // Try parsing as plain device instance number.
    if let Ok(instance) = s.parse::<u32>() {
        return Ok(Target::Instance(instance));
    }

    Err(format!(
        "invalid target '{s}': expected IP[:port], device instance, or dnet:instance"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ip_to_bip_mac() {
        let mac = ip_to_bip_mac(Ipv4Addr::new(192, 168, 1, 100), 47808);
        assert_eq!(mac, vec![192, 168, 1, 100, 0xBA, 0xC0]);
    }

    #[test]
    fn test_parse_target_ip_no_port() {
        let target = parse_target("192.168.1.100").unwrap();
        match target {
            Target::Mac(mac) => assert_eq!(mac, vec![192, 168, 1, 100, 0xBA, 0xC0]),
            _ => panic!("expected Mac target"),
        }
    }

    #[test]
    fn test_parse_target_ip_with_port() {
        let target = parse_target("10.0.1.100:47809").unwrap();
        match target {
            Target::Mac(mac) => assert_eq!(mac, vec![10, 0, 1, 100, 0xBA, 0xC1]),
            _ => panic!("expected Mac target"),
        }
    }

    #[test]
    fn test_parse_target_instance() {
        let target = parse_target("1234").unwrap();
        match target {
            Target::Instance(id) => assert_eq!(id, 1234),
            _ => panic!("expected Instance target"),
        }
    }

    #[test]
    fn test_parse_target_routed() {
        let target = parse_target("2:1234").unwrap();
        match target {
            Target::Routed(dnet, id) => {
                assert_eq!(dnet, 2);
                assert_eq!(id, 1234);
            }
            _ => panic!("expected Routed target"),
        }
    }

    #[test]
    fn test_parse_target_invalid() {
        assert!(parse_target("not-a-target").is_err());
    }

    #[test]
    fn test_parse_target_ipv6_with_port() {
        let target = parse_target("[::1]:47808").unwrap();
        match target {
            Target::Mac(mac) => assert_eq!(mac.len(), 18),
            _ => panic!("expected Mac target"),
        }
    }

    #[test]
    fn test_parse_target_ipv6_no_port() {
        let target = parse_target("[fe80::1]").unwrap();
        match target {
            Target::Mac(mac) => {
                assert_eq!(mac.len(), 18);
                // Default port should be 0xBAC0
                assert_eq!(mac[16], 0xBA);
                assert_eq!(mac[17], 0xC0);
            }
            _ => panic!("expected Mac target"),
        }
    }
}
