//! Router and BBMD commands.

use std::net::Ipv4Addr;

use bacnet_client::client::{BACnetClient, DiscoveredRouter, RoutingTable, RoutingTableWait};
use bacnet_ops::target::AddressStyle;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::port::TransportPort;
use serde::Serialize;

use crate::output::{self, OutputFormat};

/// A router `whois-router` heard from, as it prints it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RouterInfo {
    /// The router's address on the local network, in the form a target
    /// takes.
    pub router: String,
    /// The networks it reaches, ascending.
    pub networks: Vec<u16>,
}

impl RouterInfo {
    fn of(router: &DiscoveredRouter, style: AddressStyle) -> Self {
        Self {
            router: style.format(&router.mac),
            networks: router.networks.clone(),
        }
    }
}

/// Send a Who-Is-Router-To-Network, to `via` or by local broadcast, and
/// print the routers that answer within `wait_secs`: a JSON array of
/// `{router, networks}`, empty when none answered.
pub async fn whois_router_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    dnet: Option<u16>,
    via: Option<&str>,
    wait_secs: u64,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let destination = match via {
        Some(address) => Some(
            bacnet_ops::target::address_mac(
                client.transport(),
                address,
                "--via needs a local router address",
            )
            .await
            .map_err(crate::destination::with_link_flag)?,
        ),
        None => None,
    };
    output::print_waiting(wait_secs);
    let routers = client
        .discover_routers(
            dnet,
            destination.as_deref(),
            std::time::Duration::from_secs(wait_secs),
        )
        .await?;
    let style = AddressStyle::of(client.transport());
    let routers: Vec<RouterInfo> = routers.iter().map(|r| RouterInfo::of(r, style)).collect();
    match format {
        OutputFormat::Table if routers.is_empty() => println!("No routers found."),
        OutputFormat::Table => {
            let mut table = comfy_table::Table::new();
            table.set_header(vec!["Router", "Networks"]);
            for r in &routers {
                let networks: Vec<String> = r.networks.iter().map(u16::to_string).collect();
                table.add_row(vec![r.router.clone(), networks.join(", ")]);
            }
            println!("{table}");
        }
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&routers).expect("serialize routers")
        ),
    }
    Ok(())
}

/// A router's table as `routing-table` prints it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoutingTableInfo {
    /// The router's address on the local network, in the form a target
    /// takes.
    pub router: String,
    /// What it listed, in its order.
    pub entries: Vec<RoutingEntryInfo>,
    /// Whether a bound cut the answer short, so the list may be incomplete.
    pub truncated: bool,
}

/// One entry of a router's table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoutingEntryInfo {
    /// The network.
    pub network: u16,
    /// The router's identifier for the port that reaches it.
    pub port_id: u8,
    /// The port information octets (the standard defines only an octet
    /// string, typically dial-up details) as hex, the one form they have
    /// here; `None` when the entry has none.
    pub port_info: Option<String>,
}

impl RoutingTableInfo {
    fn of(router: String, table: &RoutingTable) -> Self {
        Self {
            router,
            entries: table
                .entries
                .iter()
                .map(|entry| RoutingEntryInfo {
                    network: entry.network,
                    port_id: entry.port_id,
                    port_info: (!entry.port_info.is_empty()).then(|| {
                        entry
                            .port_info
                            .iter()
                            .map(|octet| format!("{octet:02x}"))
                            .collect()
                    }),
                })
                .collect(),
            truncated: table.truncated,
        }
    }
}

/// Read the routing table of the router at `mac` on the local network
/// without changing it, waiting `wait_secs` for its first answer, and
/// print it: a JSON object with the router, its `entries` and `truncated`.
/// A router that does not answer, or rejects the query, is an error.
pub async fn routing_table_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
    wait_secs: u64,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    output::print_waiting(wait_secs);
    let first_answer = std::time::Duration::from_secs(wait_secs);
    let default = RoutingTableWait::default();
    // `--wait` is the first answer's wait, so the total grows to cover it
    // plus the default total's room for the rest of a long table.
    let wait = RoutingTableWait {
        first_answer,
        total: first_answer + default.total,
        ..default
    };
    let table = client.routing_table(mac, wait).await?;
    let style = AddressStyle::of(client.transport());
    let info = RoutingTableInfo::of(style.format(mac), &table);
    match format {
        OutputFormat::Table => {
            if info.entries.is_empty() {
                println!("The routing table is empty.");
            } else {
                let mut table = comfy_table::Table::new();
                table.set_header(vec!["Network", "Port", "Port info"]);
                for entry in &info.entries {
                    table.add_row(vec![
                        entry.network.to_string(),
                        entry.port_id.to_string(),
                        entry.port_info.clone().unwrap_or_default(),
                    ]);
                }
                println!("{table}");
            }
            if info.truncated {
                println!("The answer was cut short: the table may be incomplete.");
            }
        }
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&info).expect("serialize routing table")
        ),
    }
    Ok(())
}

/// Read the Broadcast Distribution Table from a BBMD.
pub async fn bdt_cmd(
    client: &BACnetClient<BipTransport>,
    mac: &[u8],
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let entries = client.read_bdt(mac).await?;
    match format {
        OutputFormat::Table => {
            if entries.is_empty() {
                println!("BDT is empty.");
            } else {
                let mut table = comfy_table::Table::new();
                table.set_header(vec!["IP", "Port", "Broadcast Mask"]);
                for e in &entries {
                    table.add_row(vec![
                        format!("{}", Ipv4Addr::from(e.ip)),
                        format!("{}", e.port),
                        format!("{}", Ipv4Addr::from(e.broadcast_mask)),
                    ]);
                }
                println!("{table}");
            }
        }
        OutputFormat::Json => {
            let json_entries: Vec<_> = entries
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "ip": format!("{}", Ipv4Addr::from(e.ip)),
                        "port": e.port,
                        "broadcast_mask": format!("{}", Ipv4Addr::from(e.broadcast_mask)),
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json_entries).unwrap_or_default()
            );
        }
    }
    Ok(())
}

/// Read the Foreign Device Table from a BBMD.
pub async fn fdt_cmd(
    client: &BACnetClient<BipTransport>,
    mac: &[u8],
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let entries = client.read_fdt(mac).await?;
    match format {
        OutputFormat::Table => {
            if entries.is_empty() {
                println!("FDT is empty.");
            } else {
                let mut table = comfy_table::Table::new();
                table.set_header(vec!["IP", "Port", "TTL", "Remaining"]);
                for e in &entries {
                    table.add_row(vec![
                        format!("{}", Ipv4Addr::from(e.ip)),
                        format!("{}", e.port),
                        format!("{}s", e.ttl),
                        format!("{}s", e.seconds_remaining),
                    ]);
                }
                println!("{table}");
            }
        }
        OutputFormat::Json => {
            let json_entries: Vec<_> = entries
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "ip": format!("{}", Ipv4Addr::from(e.ip)),
                        "port": e.port,
                        "ttl": e.ttl,
                        "seconds_remaining": e.seconds_remaining,
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json_entries).unwrap_or_default()
            );
        }
    }
    Ok(())
}

/// Register as a foreign device with a BBMD.
pub async fn register_cmd(
    client: &BACnetClient<BipTransport>,
    mac: &[u8],
    ttl: u16,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    client.register_foreign_device_bvlc(mac, ttl).await?;
    output::print_success("Register-Foreign-Device accepted", format);
    Ok(())
}

/// Unregister from a BBMD (delete our own FDT entry).
pub async fn unregister_cmd(
    client: &BACnetClient<BipTransport>,
    mac: &[u8],
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    // To unregister, we delete our own entry from the BBMD's FDT.
    // Our local MAC contains our IP and port.
    let local_mac = client.local_mac();
    if local_mac.len() != 6 {
        return Err("Cannot determine local BIP address".into());
    }
    let ip = [local_mac[0], local_mac[1], local_mac[2], local_mac[3]];
    let port = u16::from_be_bytes([local_mac[4], local_mac[5]]);
    client.delete_fdt_entry(mac, ip, port).await?;
    output::print_success("Delete-Foreign-Device-Table-Entry accepted", format);
    Ok(())
}
