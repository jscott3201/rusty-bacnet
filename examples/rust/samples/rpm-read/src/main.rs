//! Who-Is a BACnet device, ReadPropertyMultiple on a batch of points, print, exit.
//!
//! Default demo (device 5007): RPM read OA-T, STAT ZN-T, and DUCT-T in one request.

use std::net::Ipv4Addr;
use std::process;
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use bacnet_services::who_is::WhoIsRequest;
use bacnet_transport::bip::DEFAULT_BACNET_PORT;
use bacnet_transport::bvll::encode_bip_mac;
use bacnet_types::enums::{ObjectType, PropertyIdentifier, UnconfirmedServiceChoice};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "rpm-read", about = "BACnet ReadPropertyMultiple demo")]
struct Args {
    #[arg(long, short = 'd', default_value_t = 5007)]
    device: u32,

    /// Comma-separated points as type:instance (default: three bench sensors)
    #[arg(
        long,
        default_value = "analog-input:1173,analog-input:10014,analog-input:1192"
    )]
    points: String,

    #[arg(long, short = 'a')]
    address: Option<String>,

    #[arg(long, short = 'i')]
    interface: Option<Ipv4Addr>,

    #[arg(long, short = 'b')]
    broadcast: Option<Ipv4Addr>,

    #[arg(long, short = 't', default_value_t = 3)]
    timeout: u64,

    #[arg(long, default_value_t = DEFAULT_BACNET_PORT)]
    port: u16,

    #[arg(long)]
    ephemeral: bool,
}

fn detect_enp3s0_address() -> Option<Ipv4Addr> {
    let output = process::Command::new("ip")
        .args(["-4", "addr", "show", "dev", "enp3s0"])
        .output()
        .ok()?;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
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

fn parse_point(s: &str) -> Result<ObjectIdentifier, String> {
    let (ty, inst) = s
        .trim()
        .split_once(':')
        .ok_or_else(|| format!("expected type:instance, got {s:?}"))?;
    let inst: u32 = inst
        .parse()
        .map_err(|_| format!("bad instance number in {s:?}"))?;
    let object_type = match ty.to_ascii_lowercase().replace('-', "_").as_str() {
        "analog_input" | "ai" => ObjectType::ANALOG_INPUT,
        "analog_output" | "ao" => ObjectType::ANALOG_OUTPUT,
        "analog_value" | "av" => ObjectType::ANALOG_VALUE,
        "binary_input" | "bi" => ObjectType::BINARY_INPUT,
        "binary_output" | "bo" => ObjectType::BINARY_OUTPUT,
        "binary_value" | "bv" => ObjectType::BINARY_VALUE,
        other => {
            if let Ok(code) = other.parse::<u32>() {
                ObjectType::from_raw(code)
            } else {
                return Err(format!("unknown object type {ty:?}"));
            }
        }
    };
    ObjectIdentifier::new(object_type, inst).map_err(|e| e.to_string())
}

fn format_oid(oid: &ObjectIdentifier) -> String {
    format!("{}:{}", oid.object_type(), oid.instance_number())
}

fn present_value_hint(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Real(v) => format!("{v}"),
        PropertyValue::Enumerated(v) => format!("{v}"),
        PropertyValue::Boolean(v) => format!("{v}"),
        PropertyValue::Unsigned(v) => format!("{v}"),
        PropertyValue::Signed(v) => format!("{v}"),
        PropertyValue::CharacterString(s) => s.clone(),
        PropertyValue::Null => "null".into(),
        other => format!("{other:?}"),
    }
}

fn decode_prop(bytes: &[u8]) -> Option<PropertyValue> {
    decode_application_value(bytes, 0).ok().map(|(v, _)| v)
}

fn parse_device_endpoint(s: &str) -> Result<(Ipv4Addr, u16), String> {
    if let Some((host, port_str)) = s.rsplit_once(':') {
        if let Ok(port) = port_str.parse::<u16>() {
            if let Ok(ip) = host.parse::<Ipv4Addr>() {
                return Ok((ip, port));
            }
        }
    }
    s.parse::<Ipv4Addr>()
        .map(|ip| (ip, DEFAULT_BACNET_PORT))
        .map_err(|e| format!("invalid device address {s:?}: {e}"))
}

async fn discover_device(
    client: &mut BACnetClient<bacnet_transport::bip::BipTransport>,
    args: &Args,
) {
    if let Some(ref addr) = args.address {
        let (device_ip, device_port) = match parse_device_endpoint(addr) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("ERROR: {e}");
                process::exit(1);
            }
        };
        let mac = encode_bip_mac(device_ip.octets(), device_port).to_vec();
        eprintln!("Using fixed address {device_ip}:{device_port} (skip Who-Is)");
        if let Err(e) = client.add_device(args.device, &mac).await {
            eprintln!("ERROR: add_device failed: {e}");
            process::exit(1);
        }
        return;
    }

    let whois = WhoIsRequest {
        low_limit: Some(args.device),
        high_limit: Some(args.device),
    };
    let mut whois_buf = BytesMut::new();
    whois.encode(&mut whois_buf);

    eprintln!("Sending Who-Is for device {}...", args.device);
    if let Err(e) = client
        .broadcast_unconfirmed(UnconfirmedServiceChoice::WHO_IS, &whois_buf)
        .await
    {
        eprintln!("ERROR: local Who-Is failed: {e}");
        process::exit(1);
    }
    if let Err(e) = client.who_is(Some(args.device), Some(args.device)).await {
        eprintln!("ERROR: global Who-Is failed: {e}");
        process::exit(1);
    }

    tokio::time::sleep(Duration::from_secs(args.timeout)).await;

    if client.get_device(args.device).await.is_none() {
        eprintln!(
            "ERROR: device {} not found after {}s Who-Is",
            args.device, args.timeout
        );
        let _ = client.stop().await;
        process::exit(1);
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let interface = resolve_interface(&args);
    let broadcast = args
        .broadcast
        .unwrap_or_else(|| default_broadcast(interface));
    let bind_port = if args.ephemeral { 0 } else { args.port };

    let point_oids: Vec<ObjectIdentifier> = match args
        .points
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_point)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(v) if !v.is_empty() => v,
        Ok(_) => {
            eprintln!("ERROR: no points specified");
            process::exit(1);
        }
        Err(e) => {
            eprintln!("ERROR: {e}");
            process::exit(1);
        }
    };

    eprintln!(
        "RPM demo: device={} points={} bind={interface}:{bind_port}",
        args.device,
        point_oids.len()
    );

    let mut client = match BACnetClient::bip_builder()
        .interface(interface)
        .port(bind_port)
        .broadcast_address(broadcast)
        .apdu_timeout_ms(8000)
        .build()
        .await
    {
        Ok(c) => c,
        Err(e) if bind_port == DEFAULT_BACNET_PORT => {
            eprintln!("ERROR: cannot bind UDP {interface}:{bind_port} ({e})");
            eprintln!("Hint: stop other BACnet process on :47808 or use --ephemeral");
            process::exit(1);
        }
        Err(e) => {
            eprintln!("ERROR: BACnet client failed: {e}");
            process::exit(1);
        }
    };

    discover_device(&mut client, &args).await;

    let specs: Vec<ReadAccessSpecification> = point_oids
        .iter()
        .map(|oid| ReadAccessSpecification {
            object_identifier: *oid,
            list_of_property_references: vec![
                PropertyReference {
                    property_identifier: PropertyIdentifier::OBJECT_NAME,
                    property_array_index: None,
                },
                PropertyReference {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                },
                PropertyReference {
                    property_identifier: PropertyIdentifier::UNITS,
                    property_array_index: None,
                },
            ],
        })
        .collect();

    eprintln!(
        "Sending ReadPropertyMultiple ({} object(s))...",
        specs.len()
    );

    let rpm = match client
        .read_property_multiple_from_device(args.device, specs)
        .await
    {
        Ok(ack) => ack,
        Err(e) => {
            eprintln!("ERROR: RPM failed: {e}");
            let _ = client.stop().await;
            process::exit(1);
        }
    };

    println!("\nReadPropertyMultiple results:\n");
    for result in rpm.list_of_read_access_results {
        let oid = result.object_identifier;
        let mut name = String::from("?");
        let mut pv = String::new();
        let mut units = String::new();

        for prop in result.list_of_results {
            if let Some(ref err) = prop.error {
                eprintln!(
                    "WARN: {} {:?} failed: {:?}",
                    format_oid(&oid),
                    prop.property_identifier,
                    err
                );
                continue;
            }
            if let Some(ref bytes) = prop.property_value {
                if let Some(val) = decode_prop(bytes) {
                    match prop.property_identifier {
                        PropertyIdentifier::OBJECT_NAME => {
                            if let PropertyValue::CharacterString(s) = val {
                                name = s;
                            }
                        }
                        PropertyIdentifier::PRESENT_VALUE => {
                            pv = present_value_hint(&val);
                        }
                        PropertyIdentifier::UNITS => {
                            units = present_value_hint(&val);
                        }
                        _ => {}
                    }
                }
            }
        }

        if units.is_empty() {
            println!("  {:<28}  {:<20}  pv={pv}", format_oid(&oid), name);
        } else {
            println!(
                "  {:<28}  {:<20}  pv={pv}  units={units}",
                format_oid(&oid),
                name
            );
        }
    }

    let _ = client.stop().await;
    println!("\nDone.");
}
