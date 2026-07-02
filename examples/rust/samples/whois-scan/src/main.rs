//! BACnet Who-Is scanner — send discovery, list I-Am responses, exit.
//!
//! Binds UDP/47808 on the chosen NIC so subnet I-Am broadcasts are received
//! (required for rusty-bacnet devices that reply with broadcast I-Am).

use std::net::Ipv4Addr;
use std::process;
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_services::who_is::WhoIsRequest;
use bacnet_transport::bip::DEFAULT_BACNET_PORT;
use bacnet_types::enums::UnconfirmedServiceChoice;
use bytes::BytesMut;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "whois-scan",
    about = "BACnet Who-Is discovery scan (rusty-bacnet)"
)]
struct Args {
    /// Local NIC IPv4 to bind (auto-detects enp3s0 if omitted)
    #[arg(long, short = 'i')]
    interface: Option<Ipv4Addr>,

    /// Subnet directed broadcast (default: /24 from --interface)
    #[arg(long, short = 'b')]
    broadcast: Option<Ipv4Addr>,

    /// UDP port to bind (default 47808 — needed to hear broadcast I-Am)
    #[arg(long, default_value_t = DEFAULT_BACNET_PORT)]
    port: u16,

    /// Seconds to wait for I-Am after sending Who-Is
    #[arg(long, short = 't', default_value_t = 3)]
    timeout: u64,

    /// Optional device instance range low limit
    #[arg(long)]
    low: Option<u32>,

    /// Optional device instance range high limit
    #[arg(long)]
    high: Option<u32>,

    /// Use ephemeral UDP port if 47808 is busy (broadcast I-Am may be missed)
    #[arg(long)]
    ephemeral: bool,
}

fn detect_enp3s0_address() -> Option<Ipv4Addr> {
    let output = process::Command::new("ip")
        .args(["-4", "addr", "show", "dev", "enp3s0"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("inet ") {
            if let Some(ip_str) = rest.split_whitespace().next() {
                if let Some(base) = ip_str.split('/').next() {
                    if let Ok(ip) = base.parse::<Ipv4Addr>() {
                        return Some(ip);
                    }
                }
            }
        }
    }
    None
}

fn subnet_broadcast(ip: Ipv4Addr) -> Ipv4Addr {
    let o = ip.octets();
    Ipv4Addr::new(o[0], o[1], o[2], 255)
}

fn default_broadcast(interface: Ipv4Addr) -> Ipv4Addr {
    if interface.is_unspecified() {
        Ipv4Addr::BROADCAST
    } else {
        subnet_broadcast(interface)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_broadcast_uses_global_broadcast_for_unspecified_interface() {
        assert_eq!(
            default_broadcast(Ipv4Addr::UNSPECIFIED),
            Ipv4Addr::BROADCAST
        );
    }

    #[test]
    fn default_broadcast_uses_slash_24_for_bound_interface() {
        assert_eq!(
            default_broadcast(Ipv4Addr::new(192, 168, 204, 55)),
            Ipv4Addr::new(192, 168, 204, 255)
        );
    }
}

fn resolve_interface(args: &Args) -> Ipv4Addr {
    if let Some(ip) = args.interface {
        return ip;
    }
    if let Some(ip) = detect_enp3s0_address() {
        eprintln!("auto-detected enp3s0: {ip}");
        return ip;
    }
    eprintln!("no --interface and enp3s0 not found; using 0.0.0.0");
    Ipv4Addr::UNSPECIFIED
}

fn format_bip_mac(mac: &[u8]) -> String {
    if mac.len() == 6 {
        format!(
            "{}.{}.{}.{}:{}",
            mac[0],
            mac[1],
            mac[2],
            mac[3],
            u16::from_be_bytes([mac[4], mac[5]])
        )
    } else {
        mac.iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":")
    }
}

async fn build_client(
    interface: Ipv4Addr,
    broadcast: Ipv4Addr,
    port: u16,
) -> Result<BACnetClient<bacnet_transport::bip::BipTransport>, bacnet_types::error::Error> {
    BACnetClient::bip_builder()
        .interface(interface)
        .port(port)
        .broadcast_address(broadcast)
        .build()
        .await
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let interface = resolve_interface(&args);
    let broadcast = args
        .broadcast
        .unwrap_or_else(|| default_broadcast(interface));

    if args.low.is_some() ^ args.high.is_some() {
        eprintln!("ERROR: --low and --high must be used together");
        process::exit(1);
    }

    let bind_port = if args.ephemeral {
        eprintln!("WARNING: --ephemeral skips UDP/47808; broadcast I-Am may not be received");
        0
    } else {
        args.port
    };

    eprintln!(
        "Who-Is scan: bind={interface}:{bind_port} broadcast={broadcast} timeout={}s",
        args.timeout
    );

    let mut client = match build_client(interface, broadcast, bind_port).await {
        Ok(c) => c,
        Err(e) if bind_port == DEFAULT_BACNET_PORT => {
            eprintln!("ERROR: cannot bind UDP {interface}:{bind_port} ({e})");
            eprintln!("Hint: stop mini-device-revisited or other BACnet process on :47808");
            eprintln!("      or retry with --ephemeral (may miss broadcast I-Am replies)");
            process::exit(1);
        }
        Err(e) => {
            eprintln!("ERROR: BACnet client failed: {e}");
            process::exit(1);
        }
    };

    let whois = WhoIsRequest {
        low_limit: args.low,
        high_limit: args.high,
    };
    let mut whois_buf = BytesMut::new();
    whois.encode(&mut whois_buf);

    eprintln!("Sending local-subnet Who-Is...");
    if let Err(e) = client
        .broadcast_unconfirmed(UnconfirmedServiceChoice::WHO_IS, &whois_buf)
        .await
    {
        eprintln!("ERROR: local Who-Is failed: {e}");
        process::exit(1);
    }

    eprintln!("Sending global Who-Is (DNET=0xFFFF)...");
    if let Err(e) = client.who_is(args.low, args.high).await {
        eprintln!("ERROR: global Who-Is failed: {e}");
        process::exit(1);
    }

    tokio::time::sleep(Duration::from_secs(args.timeout)).await;

    let devices = client.discovered_devices().await;
    let _ = client.stop().await;

    if devices.is_empty() {
        eprintln!("\nNo devices found.");
        eprintln!("Checks:");
        eprintln!("  - Scanner and devices on same subnet ({broadcast})");
        eprintln!("  - Target device running and not using --log-packets (steals UDP)");
        eprintln!("  - UDP/47808 free on this host (or run scan from another machine)");
        process::exit(1);
    }

    println!("\nFound {} device(s):\n", devices.len());
    for d in devices {
        let instance = d.object_identifier.instance_number();
        let addr = format_bip_mac(d.mac_address.as_slice());
        println!(
            "  device {instance:>6}  addr {addr:<21}  vendor {}  max_apdu {}",
            d.vendor_id, d.max_apdu_length
        );
    }
}
