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
/// - MS/TP: `"7"` or `"mstp:7"` → 1-byte unicast peer MAC (0..=254)
pub fn parse_address(address: &str) -> PyResult<Vec<u8>> {
    // MS/TP unicast peer: "mstp:N" or bare decimal 0..=254.
    if let Some(rest) = address.strip_prefix("mstp:") {
        return parse_mstp_peer_address(rest);
    }
    let is_negative_decimal = address
        .strip_prefix('-')
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()));
    if address.bytes().all(|b| b.is_ascii_digit()) || is_negative_decimal {
        return parse_mstp_peer_address(address);
    }

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
        PyValueError::new_err(
            "address must be 'ip:port', '[ipv6]:port', 'aa:bb:...' hex, or MS/TP 'N' / 'mstp:N'",
        )
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

fn parse_mstp_peer_address(address: &str) -> PyResult<Vec<u8>> {
    if address.starts_with('-') {
        return Err(PyValueError::new_err(
            "MS/TP peer address must be in 0..=254",
        ));
    }
    if address.is_empty() || !address.bytes().all(|b| b.is_ascii_digit()) {
        return Err(PyValueError::new_err(
            "MS/TP peer address must be a decimal integer in 0..=254",
        ));
    }
    let mac: u32 = address
        .parse()
        .map_err(|_| PyValueError::new_err("MS/TP peer address must be in 0..=254"))?;
    match mac {
        0..=254 => Ok(vec![mac as u8]),
        255 => Err(PyValueError::new_err(
            "MS/TP peer address 255 is broadcast, not a unicast peer",
        )),
        _ => Err(PyValueError::new_err(
            "MS/TP peer address must be in 0..=254",
        )),
    }
}
