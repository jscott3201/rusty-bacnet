//! Discover a BACnet device (default instance 5007), read its object-list,
//! fetch point names and present-values, scan priority arrays on commandable
//! points, print, exit.

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

mod net_defaults;

use net_defaults::default_broadcast;

#[derive(Parser, Debug)]
#[command(
    name = "point-discover",
    about = "Who-Is a device, enumerate object-list points, exit"
)]
struct Args {
    /// BACnet device instance to find (default: 5007 on this bench)
    #[arg(long, short = 'd', default_value_t = 5007)]
    device: u32,

    /// Skip Who-Is and use device IPv4 or ip:port (e.g. 192.168.204.200:47808)
    #[arg(long, short = 'a')]
    address: Option<String>,

    /// Local NIC IPv4 to bind (auto-detects enp3s0 if omitted)
    #[arg(long, short = 'i')]
    interface: Option<Ipv4Addr>,

    /// Subnet directed broadcast (default: /24 from --interface)
    #[arg(long, short = 'b')]
    broadcast: Option<Ipv4Addr>,

    /// Seconds to wait for I-Am after Who-Is
    #[arg(long, short = 't', default_value_t = 3)]
    timeout: u64,

    /// Client UDP bind port (not the remote device port)
    #[arg(long, default_value_t = DEFAULT_BACNET_PORT)]
    port: u16,

    /// Bind ephemeral port instead of 47808
    #[arg(long)]
    ephemeral: bool,

    /// Skip priority-array scan for commandable points
    #[arg(long)]
    skip_priority: bool,
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

fn format_oid(oid: &ObjectIdentifier) -> String {
    format!("{}:{}", oid.object_type(), oid.instance_number())
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

fn decode_object_identifier_list(bytes: &[u8]) -> Vec<ObjectIdentifier> {
    let mut oids = Vec::new();
    let mut offset = 0usize;
    while offset < bytes.len() {
        match decode_application_value(bytes, offset) {
            Ok((PropertyValue::ObjectIdentifier(oid), next)) if next > offset => {
                oids.push(oid);
                offset = next;
            }
            Ok((_, next)) if next > offset => offset = next,
            _ => break,
        }
    }
    oids
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_encoding::primitives::encode_property_value;
    use bytes::BytesMut;

    #[test]
    fn decode_full_object_list_sequence() {
        let device = ObjectIdentifier::new(ObjectType::DEVICE, 5007).unwrap();
        let ai = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1173).unwrap();
        let mut buf = BytesMut::new();
        encode_property_value(&mut buf, &PropertyValue::ObjectIdentifier(device)).unwrap();
        encode_property_value(&mut buf, &PropertyValue::ObjectIdentifier(ai)).unwrap();

        let oids = decode_object_identifier_list(&buf);
        assert_eq!(oids.len(), 2);
        assert_eq!(oids[0], device);
        assert_eq!(oids[1], ai);
    }
}

async fn read_object_list(
    client: &BACnetClient<bacnet_transport::bip::BipTransport>,
    device_instance: u32,
    device_oid: ObjectIdentifier,
) -> Result<Vec<ObjectIdentifier>, bacnet_types::error::Error> {
    async fn read_prop(
        client: &BACnetClient<bacnet_transport::bip::BipTransport>,
        device_instance: u32,
        device_oid: ObjectIdentifier,
        index: Option<u32>,
    ) -> Result<bacnet_services::read_property::ReadPropertyACK, bacnet_types::error::Error> {
        client
            .read_property_from_device(
                device_instance,
                device_oid,
                PropertyIdentifier::OBJECT_LIST,
                index,
            )
            .await
    }

    // Many field devices only expose object-list via array index (not as a whole list).
    if let Ok(ack0) = read_prop(client, device_instance, device_oid, Some(0)).await {
        if let Some(PropertyValue::Unsigned(count)) = decode_prop(&ack0.property_value) {
            let count = count as u32;
            let mut oids = Vec::with_capacity(count as usize);
            for idx in 1..=count {
                let ack = read_prop(client, device_instance, device_oid, Some(idx)).await?;
                match decode_prop(&ack.property_value) {
                    Some(PropertyValue::ObjectIdentifier(oid)) => oids.push(oid),
                    other => {
                        eprintln!("WARN: object-list[{idx}] unexpected value: {other:?}");
                    }
                }
            }
            if !oids.is_empty() {
                return Ok(oids);
            }
        }
    }

    // Fallback: single ReadProperty for the full list (sequential object identifiers).
    let ack = read_prop(client, device_instance, device_oid, None).await?;
    let oids = decode_object_identifier_list(&ack.property_value);
    if !oids.is_empty() {
        return Ok(oids);
    }

    Err(bacnet_types::error::Error::Encoding(
        "object-list empty or unsupported encoding".into(),
    ))
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

fn is_commandable_candidate(oid: &ObjectIdentifier) -> bool {
    matches!(
        oid.object_type(),
        ObjectType::ANALOG_OUTPUT
            | ObjectType::BINARY_OUTPUT
            | ObjectType::MULTI_STATE_OUTPUT
            | ObjectType::ANALOG_VALUE
            | ObjectType::BINARY_VALUE
            | ObjectType::MULTI_STATE_VALUE
    )
}

fn format_priority_slot(value: &PropertyValue) -> String {
    present_value_hint(value)
}

async fn read_current_command_priority(
    client: &BACnetClient<bacnet_transport::bip::BipTransport>,
    device_instance: u32,
    oid: ObjectIdentifier,
) -> Option<u8> {
    let ack = client
        .read_property_from_device(
            device_instance,
            oid,
            PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
            None,
        )
        .await
        .ok()?;
    match decode_prop(&ack.property_value)? {
        PropertyValue::Unsigned(v) if (1..=16).contains(&(v as u8)) => Some(v as u8),
        PropertyValue::Null => None,
        _ => None,
    }
}

async fn has_priority_array(
    client: &BACnetClient<bacnet_transport::bip::BipTransport>,
    device_instance: u32,
    oid: ObjectIdentifier,
) -> bool {
    client
        .read_property_from_device(
            device_instance,
            oid,
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(0),
        )
        .await
        .is_ok()
        || client
            .read_property_from_device(
                device_instance,
                oid,
                PropertyIdentifier::PRIORITY_ARRAY,
                Some(1),
            )
            .await
            .is_ok()
}

async fn read_priority_array_slots(
    client: &BACnetClient<bacnet_transport::bip::BipTransport>,
    device_instance: u32,
    oid: ObjectIdentifier,
) -> Vec<(u8, PropertyValue)> {
    if let Ok(ack) = client
        .read_property_from_device(
            device_instance,
            oid,
            PropertyIdentifier::PRIORITY_ARRAY,
            None,
        )
        .await
    {
        if let Some(PropertyValue::List(items)) = decode_prop(&ack.property_value) {
            return items
                .into_iter()
                .enumerate()
                .filter_map(|(idx, value)| {
                    let priority = (idx + 1) as u8;
                    if priority > 16 || matches!(value, PropertyValue::Null) {
                        None
                    } else {
                        Some((priority, value))
                    }
                })
                .collect();
        }
    }

    let mut active = Vec::new();
    for chunk_start in (1u32..=16).step_by(8) {
        let chunk_end = (chunk_start + 7).min(16);
        let specs = vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: (chunk_start..=chunk_end)
                .map(|idx| PropertyReference {
                    property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
                    property_array_index: Some(idx),
                })
                .collect(),
        }];

        let rpm = match client
            .read_property_multiple_from_device(device_instance, specs)
            .await
        {
            Ok(r) => r,
            Err(_) => continue,
        };

        for (offset, result) in rpm.list_of_read_access_results[0]
            .list_of_results
            .iter()
            .enumerate()
        {
            let priority = (chunk_start + offset as u32) as u8;
            if let Some(ref bytes) = result.property_value {
                if let Some(val) = decode_prop(bytes) {
                    if !matches!(val, PropertyValue::Null) {
                        active.push((priority, val));
                    }
                }
            }
        }
    }

    if active.is_empty() {
        for priority in 1u8..=16 {
            if let Ok(ack) = client
                .read_property_from_device(
                    device_instance,
                    oid,
                    PropertyIdentifier::PRIORITY_ARRAY,
                    Some(priority as u32),
                )
                .await
            {
                if let Some(val) = decode_prop(&ack.property_value) {
                    if !matches!(val, PropertyValue::Null) {
                        active.push((priority, val));
                    }
                }
            }
        }
    }

    active
}

struct CommandablePoint {
    oid: ObjectIdentifier,
    name: String,
    pv: String,
    current_priority: Option<u8>,
    priority_slots: Vec<(u8, PropertyValue)>,
}

async fn scan_priority_arrays(
    client: &BACnetClient<bacnet_transport::bip::BipTransport>,
    device_instance: u32,
    rows: &[(ObjectIdentifier, String, String)],
) -> Vec<CommandablePoint> {
    let candidates: Vec<_> = rows
        .iter()
        .filter(|(oid, _, _)| is_commandable_candidate(oid))
        .collect();

    if candidates.is_empty() {
        return Vec::new();
    }

    eprintln!(
        "Scanning priority arrays on {} commandable candidate(s)...",
        candidates.len()
    );

    let mut results = Vec::new();
    for (oid, name, pv) in candidates {
        if !has_priority_array(client, device_instance, *oid).await {
            continue;
        }

        let current_priority = read_current_command_priority(client, device_instance, *oid).await;
        let priority_slots = read_priority_array_slots(client, device_instance, *oid).await;

        results.push(CommandablePoint {
            oid: *oid,
            name: name.clone(),
            pv: pv.clone(),
            current_priority,
            priority_slots,
        });
    }

    results
}

fn effective_command_priority(current: Option<u8>, slots: &[(u8, PropertyValue)]) -> Option<u8> {
    current.or_else(|| slots.iter().map(|(p, _)| *p).min())
}

fn should_print_priority_point(point: &CommandablePoint) -> bool {
    if !point.priority_slots.is_empty() {
        return true;
    }
    matches!(
        point.oid.object_type(),
        ObjectType::ANALOG_OUTPUT | ObjectType::BINARY_OUTPUT | ObjectType::MULTI_STATE_OUTPUT
    )
}

fn print_priority_scan(results: &[CommandablePoint]) {
    let visible: Vec<_> = results
        .iter()
        .filter(|p| should_print_priority_point(p))
        .collect();

    if visible.is_empty() {
        println!("\nNo commandable points with active priority-array entries found.\n");
        return;
    }

    let active_count = visible
        .iter()
        .filter(|p| !p.priority_slots.is_empty())
        .count();

    println!(
        "\nCommandable points — priority arrays ({} shown, {} with active slot(s)):\n",
        visible.len(),
        active_count
    );

    for point in visible {
        let cmd = effective_command_priority(point.current_priority, &point.priority_slots)
            .map(|p| format!("cmd@P{p}"))
            .unwrap_or_else(|| "cmd@—".into());

        if point.pv.is_empty() {
            println!(
                "  {:<28}  {:<32}  {cmd}",
                format_oid(&point.oid),
                point.name
            );
        } else {
            println!(
                "  {:<28}  {:<32}  pv={}  {cmd}",
                format_oid(&point.oid),
                point.name,
                point.pv
            );
        }

        if point.priority_slots.is_empty() {
            println!("    (16 slots scanned — all null / relinquished)");
        } else {
            for (priority, value) in &point.priority_slots {
                println!("    P{priority:<2}  {}", format_priority_slot(value));
            }
        }
        println!();
    }
}

fn rpm_specs_for_points(points: &[ObjectIdentifier]) -> Vec<ReadAccessSpecification> {
    points
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
            ],
        })
        .collect()
}

fn print_point_line(oid: &ObjectIdentifier, name: &str, pv: &str) {
    if pv.is_empty() {
        println!("  {:<28}  {}", format_oid(oid), name);
    } else {
        println!("  {:<28}  {:<32}  pv={pv}", format_oid(oid), name);
    }
}

async fn read_point_name(
    client: &BACnetClient<bacnet_transport::bip::BipTransport>,
    device_instance: u32,
    oid: ObjectIdentifier,
) -> String {
    match client
        .read_property_from_device(device_instance, oid, PropertyIdentifier::OBJECT_NAME, None)
        .await
    {
        Ok(ack) => decode_prop(&ack.property_value)
            .and_then(|v| match v {
                PropertyValue::CharacterString(s) => Some(s),
                _ => None,
            })
            .unwrap_or_else(|| "?".into()),
        Err(_) => "?".into(),
    }
}

async fn read_point_present_value(
    client: &BACnetClient<bacnet_transport::bip::BipTransport>,
    device_instance: u32,
    oid: ObjectIdentifier,
) -> String {
    match client
        .read_property_from_device(
            device_instance,
            oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
        )
        .await
    {
        Ok(ack) => decode_prop(&ack.property_value)
            .map(|v| present_value_hint(&v))
            .unwrap_or_default(),
        Err(_) => String::new(),
    }
}

async fn read_points_batched(
    client: &BACnetClient<bacnet_transport::bip::BipTransport>,
    device_instance: u32,
    points: &[ObjectIdentifier],
    batch_size: usize,
) -> Vec<(ObjectIdentifier, String, String)> {
    let mut rows = Vec::with_capacity(points.len());

    for chunk in points.chunks(batch_size.max(1)) {
        let specs = rpm_specs_for_points(chunk);
        match client
            .read_property_multiple_from_device(device_instance, specs)
            .await
        {
            Ok(rpm) => {
                for result in rpm.list_of_read_access_results {
                    let oid = result.object_identifier;
                    let mut name = String::from("?");
                    let mut pv = String::new();

                    for prop in result.list_of_results {
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
                                    _ => {}
                                }
                            }
                        }
                    }
                    rows.push((oid, name, pv));
                }
            }
            Err(e) => {
                eprintln!(
                    "WARN: RPM batch failed ({e}), falling back for {} point(s)",
                    chunk.len()
                );
                for oid in chunk {
                    let name = read_point_name(client, device_instance, *oid).await;
                    let pv = read_point_present_value(client, device_instance, *oid).await;
                    rows.push((*oid, name, pv));
                }
            }
        }
    }

    rows
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let interface = resolve_interface(&args);
    let broadcast = args
        .broadcast
        .unwrap_or_else(|| default_broadcast(interface));
    let bind_port = if args.ephemeral { 0 } else { args.port };

    eprintln!(
        "Point discover: device={} bind={interface}:{bind_port} broadcast={broadcast}",
        args.device
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
            eprintln!("Hint: stop mini-device on :47808 or use --ephemeral");
            process::exit(1);
        }
        Err(e) => {
            eprintln!("ERROR: BACnet client failed: {e}");
            process::exit(1);
        }
    };

    let device_oid = match ObjectIdentifier::new(ObjectType::DEVICE, args.device) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("ERROR: bad device instance {}: {e}", args.device);
            process::exit(1);
        }
    };

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
    } else {
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
            eprintln!("Try: --address <device-ip> if you know it (e.g. 192.168.204.200)");
            let _ = client.stop().await;
            process::exit(1);
        }
    }

    let discovered = client
        .get_device(args.device)
        .await
        .expect("device table entry");
    let mac = discovered.mac_address.as_slice();
    let addr = format_bip_mac(mac);

    // Device object-name
    let device_name = match client
        .read_property_from_device(
            args.device,
            device_oid,
            PropertyIdentifier::OBJECT_NAME,
            None,
        )
        .await
    {
        Ok(ack) => decode_prop(&ack.property_value)
            .and_then(|v| match v {
                PropertyValue::CharacterString(s) => Some(s),
                _ => None,
            })
            .unwrap_or_else(|| "?".into()),
        Err(e) => {
            eprintln!("WARN: device object-name read failed: {e}");
            "?".into()
        }
    };

    println!(
        "\nDevice {} at {addr}  name \"{device_name}\"\n",
        args.device
    );

    let object_list = match read_object_list(&client, args.device, device_oid).await {
        Ok(list) => list,
        Err(e) => {
            eprintln!("ERROR: object-list read failed: {e}");
            let _ = client.stop().await;
            process::exit(1);
        }
    };

    let points: Vec<ObjectIdentifier> = object_list
        .into_iter()
        .filter(|oid| oid.object_type() != ObjectType::DEVICE)
        .collect();

    if points.is_empty() {
        println!("No points in object-list (device only).");
        let _ = client.stop().await;
        return;
    }

    eprintln!("Reading {} point(s)...", points.len());

    let rows = read_points_batched(&client, args.device, &points, 10).await;

    println!("Points ({}):\n", points.len());
    for (oid, name, pv) in &rows {
        print_point_line(oid, name, pv);
    }

    if !args.skip_priority {
        let priority_results = scan_priority_arrays(&client, args.device, &rows).await;
        print_priority_scan(&priority_results);
    }

    let _ = client.stop().await;
}
