//! Multi-port BACnet router for Docker stress topology.
//!
//! Parses a comma-separated port spec and starts a router connecting them.
//! Format: `bip:<ip>:<port>:<broadcast>:<network>,bip:<ip>:<port>:<broadcast>:<network>`

use std::net::Ipv4Addr;

use bacnet_network::router::{BACnetRouter, RouterPort};
use bacnet_transport::bip::BipTransport;
use clap::Parser;

#[derive(Parser)]
#[command(name = "bacnet-router", about = "BACnet multi-port router")]
struct Args {
    /// Comma-separated port specs: bip:<ip>:<port>:<broadcast>:<network>
    #[arg(long)]
    ports: String,
}

struct PortSpec {
    interface: Ipv4Addr,
    port: u16,
    broadcast: Ipv4Addr,
    network: u16,
}

fn parse_port_spec(spec: &str) -> Result<PortSpec, Box<dyn std::error::Error>> {
    let parts: Vec<&str> = spec.split(':').collect();
    if parts.len() != 5 || parts[0] != "bip" {
        return Err(format!(
            "Invalid port spec: {spec}. Expected bip:<ip>:<port>:<broadcast>:<network>"
        )
        .into());
    }
    Ok(PortSpec {
        interface: parts[1].parse()?,
        port: parts[2].parse()?,
        broadcast: parts[3].parse()?,
        network: parts[4].parse()?,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let specs: Vec<PortSpec> = args
        .ports
        .split(',')
        .map(|s| parse_port_spec(s.trim()))
        .collect::<Result<Vec<_>, _>>()?;

    if specs.len() < 2 {
        return Err("Router requires at least 2 ports".into());
    }

    let mut router_ports = Vec::new();
    for spec in &specs {
        let transport = BipTransport::new(spec.interface, spec.port, spec.broadcast);
        router_ports.push(RouterPort {
            transport,
            network_number: spec.network,
        });
    }

    let networks: Vec<u16> = specs.iter().map(|s| s.network).collect();
    eprintln!(
        "Router starting with {} ports: networks {:?}",
        specs.len(),
        networks
    );

    let (_router, mut local_rx) = BACnetRouter::start(router_ports).await?;

    eprintln!("Router running");

    // Drain local APDUs (router shouldn't receive many targeted to itself)
    loop {
        tokio::select! {
            apdu = local_rx.recv() => {
                if apdu.is_none() {
                    break;
                }
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("Router shutting down");
                break;
            }
        }
    }

    Ok(())
}
