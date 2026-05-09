use super::*;

// ---------------------------------------------------------------------------
// Address parsing
// ---------------------------------------------------------------------------

/// Parse an address string to a MAC byte vector.
///
/// Supported formats:
/// - IPv4: `"192.168.1.100:47808"` → 6-byte MAC (4-byte IP + 2-byte port BE)
/// - IPv6: `"[::1]:47808"` → 18-byte MAC (16-byte IPv6 + 2-byte port BE)
/// - Hex:  `"01:02:03:04:05:06"` → raw bytes (for SC VMAC or Ethernet MAC)
pub fn parse_address(address: &str) -> PyResult<Vec<u8>> {
    // IPv6 bracket notation: [addr]:port
    if address.starts_with('[') {
        let close = address
            .find(']')
            .ok_or_else(|| PyValueError::new_err("IPv6 address missing closing bracket"))?;
        let ip_str = &address[1..close];
        let ip: std::net::Ipv6Addr = ip_str
            .parse()
            .map_err(|e| PyValueError::new_err(format!("invalid IPv6 address: {e}")))?;
        let rest = &address[close + 1..];
        let port_str = rest
            .strip_prefix(':')
            .ok_or_else(|| PyValueError::new_err("expected ':port' after IPv6 address"))?;
        let port: u16 = port_str
            .parse()
            .map_err(|e| PyValueError::new_err(format!("invalid port: {e}")))?;
        let mut mac = Vec::with_capacity(18);
        mac.extend_from_slice(&ip.octets());
        mac.extend_from_slice(&port.to_be_bytes());
        return Ok(mac);
    }

    // Hex colon notation: aa:bb:cc:dd:ee:ff (6 or more hex pairs)
    if address.contains(':')
        && address
            .split(':')
            .all(|s| s.len() == 2 && s.chars().all(|c| c.is_ascii_hexdigit()))
    {
        let bytes: Result<Vec<u8>, _> = address
            .split(':')
            .map(|s| u8::from_str_radix(s, 16))
            .collect();
        return bytes.map_err(|e| PyValueError::new_err(format!("invalid hex address: {e}")));
    }

    // IPv4: ip:port
    let (ip_str, port_str) = address.rsplit_once(':').ok_or_else(|| {
        PyValueError::new_err("address must be 'ip:port', '[ipv6]:port', or 'aa:bb:...' hex")
    })?;
    let ip: Ipv4Addr = ip_str
        .parse()
        .map_err(|e| PyValueError::new_err(format!("invalid IP address: {e}")))?;
    let port: u16 = port_str
        .parse()
        .map_err(|e| PyValueError::new_err(format!("invalid port: {e}")))?;
    let mut mac = Vec::with_capacity(6);
    mac.extend_from_slice(&ip.octets());
    mac.extend_from_slice(&port.to_be_bytes());
    Ok(mac)
}
