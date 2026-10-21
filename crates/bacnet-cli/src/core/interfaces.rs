//! IPv4 interface listing for picking the BACnet/IP bind address.
//!
//! The TUI's interface dialog chooses from this list.

use std::net::Ipv4Addr;

/// An IPv4 network interface with its address and broadcast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ipv4Interface {
    /// Operating-system interface name, such as `en0` or `eth0`.
    pub(crate) name: String,
    /// The interface's IPv4 address.
    pub(crate) ip: Ipv4Addr,
    /// Its directed broadcast address.
    pub(crate) broadcast: Ipv4Addr,
}

/// List available IPv4 network interfaces, excluding loopback.
pub(crate) fn list_ipv4_interfaces() -> Vec<Ipv4Interface> {
    let Ok(ifaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for iface in ifaces {
        if iface.is_loopback() {
            continue;
        }
        if let if_addrs::IfAddr::V4(v4) = &iface.addr {
            let broadcast = v4.broadcast.unwrap_or_else(|| {
                // Compute broadcast from IP and netmask.
                let ip_bits = u32::from(v4.ip);
                let mask_bits = u32::from(v4.netmask);
                Ipv4Addr::from(ip_bits | !mask_bits)
            });
            result.push(Ipv4Interface {
                name: iface.name.clone(),
                ip: v4.ip,
                broadcast,
            });
        }
    }
    result
}
