use std::io::{self, Write as _};
use std::net::Ipv4Addr;

use owo_colors::OwoColorize;

/// An IPv4 network interface with its address and broadcast.
struct Ipv4Interface {
    name: String,
    ip: Ipv4Addr,
    broadcast: Ipv4Addr,
}

/// List available IPv4 network interfaces, excluding loopback.
fn list_ipv4_interfaces() -> Vec<Ipv4Interface> {
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

/// Prompt the user to select a network interface. Returns (ip, broadcast).
pub(crate) fn pick_interface() -> Result<(Ipv4Addr, Ipv4Addr), Box<dyn std::error::Error>> {
    let ifaces = list_ipv4_interfaces();
    if ifaces.is_empty() {
        eprintln!("No network interfaces found, binding to 0.0.0.0");
        return Ok((Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST));
    }
    if ifaces.len() == 1 {
        let iface = &ifaces[0];
        eprintln!(
            "Using interface {} ({}, broadcast {})",
            iface.name.bold(),
            iface.ip,
            iface.broadcast
        );
        return Ok((iface.ip, iface.broadcast));
    }

    eprintln!("{}", "Select a network interface:".bold());
    for (i, iface) in ifaces.iter().enumerate() {
        eprintln!(
            "  {}) {} — {} (broadcast {})",
            (i + 1).bold(),
            iface.name.bold(),
            iface.ip,
            iface.broadcast.dimmed()
        );
    }
    eprint!("Enter selection [1-{}]: ", ifaces.len());
    io::stderr().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let choice: usize = input
        .trim()
        .parse()
        .map_err(|_| format!("invalid selection: '{}'", input.trim()))?;
    if choice < 1 || choice > ifaces.len() {
        return Err(format!("selection out of range: {choice}").into());
    }
    let iface = &ifaces[choice - 1];
    eprintln!("Using interface {} ({})", iface.name.bold(), iface.ip);
    Ok((iface.ip, iface.broadcast))
}
