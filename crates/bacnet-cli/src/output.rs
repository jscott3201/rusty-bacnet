//! Output formatting for the BACnet CLI.
//!
//! Supports table (human-readable) and JSON output formats with automatic
//! TTY detection.

use bacnet_client::discovery::DiscoveredDevice;
use owo_colors::OwoColorize;
use serde::Serialize;

use bacnet_ops::format::{Datum, StateTexts};
use bacnet_ops::target::{AddressStyle, routed_station};

/// No state texts: a binary object's state shows as `active` or
/// `inactive` and a multi-state one as its number. `read` names states
/// with texts it reads (#2132); the other one-shot commands use this.
pub static NO_STATE_TEXTS: StateTexts = StateTexts {
    active: None,
    inactive: None,
    states: Vec::new(),
};

/// Text that may hold device strings (a value, a name, a message) as the
/// table and text output prints it (#2144): each control character and
/// line or paragraph separator as a space and each invisible format
/// character dropped ([`clean_text`](bacnet_ops::format::clean_text)), as
/// the TUI shows them, so a device can't move the cursor, break a row,
/// send an escape sequence or reorder what follows. Applied as the text is
/// printed, before any colour; JSON output keeps the device's text exactly.
pub fn screen(text: &str) -> String {
    bacnet_ops::format::clean_text(text)
}

/// Output format for CLI results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Human-readable table layout.
    Table,
    /// Machine-readable JSON.
    Json,
}

/// Device information for display purposes.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    /// BACnet device instance number.
    pub instance: u32,
    /// Network address (e.g., "192.168.1.10:47808").
    pub address: String,
    /// BACnet vendor identifier.
    pub vendor_id: u16,
    /// Maximum APDU length accepted.
    pub max_apdu: u32,
    /// Segmentation support description.
    pub segmentation: String,
    /// Remote BACnet network number (if behind a router).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<u16>,
    /// The routed station (if behind a router), in the target syntax the
    /// CLI reads back: `2:0x05`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
}

/// Convert a `DiscoveredDevice` into a `DeviceInfo` for display, its
/// address in `style` (a BACnet/SC VMAC as hex octets, not as `ip:port`).
pub fn device_info(d: &DiscoveredDevice, style: AddressStyle) -> DeviceInfo {
    DeviceInfo {
        instance: d.object_identifier.instance_number(),
        address: style.format(d.mac_address.as_slice()),
        vendor_id: d.vendor_id,
        max_apdu: d.max_apdu_length,
        segmentation: format!("{}", d.segmentation_supported),
        network: d.source_network,
        station: d
            .source_network
            .zip(d.source_address.as_ref())
            .and_then(|(network, mac)| routed_station(network, mac)),
    }
}

/// The table `whois` prints. With any routed device, the link address is
/// the router's, and the station column says where the device is.
fn devices_table(devices: &[DeviceInfo]) -> comfy_table::Table {
    let has_routing = devices.iter().any(|d| d.network.is_some());
    let mut table = comfy_table::Table::new();
    if has_routing {
        table.set_header(vec![
            "Instance",
            "Router",
            "DNET",
            "Station",
            "Vendor",
            "Max APDU",
            "Segmentation",
        ]);
        for d in devices {
            table.add_row(vec![
                d.instance.to_string(),
                d.address.clone(),
                d.network
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "-".into()),
                d.station.clone().unwrap_or_else(|| "-".into()),
                d.vendor_id.to_string(),
                d.max_apdu.to_string(),
                d.segmentation.clone(),
            ]);
        }
    } else {
        table.set_header(vec![
            "Instance",
            "Address",
            "Vendor",
            "Max APDU",
            "Segmentation",
        ]);
        for d in devices {
            table.add_row(vec![
                d.instance.to_string(),
                d.address.clone(),
                d.vendor_id.to_string(),
                d.max_apdu.to_string(),
                d.segmentation.clone(),
            ]);
        }
    }
    table
}

/// Print a list of discovered devices.
pub fn print_devices(devices: &[DeviceInfo], format: OutputFormat) {
    match format {
        OutputFormat::Table => {
            if devices.is_empty() {
                println!("{}", "No devices found.".yellow());
                return;
            }
            println!("{}", devices_table(devices));
            println!("{}", format!("Found {} device(s)", devices.len()).green());
        }
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(devices).expect("serialize devices");
            println!("{json}");
        }
    }
}

/// Print a single read result: the value as text, and in JSON its typed
/// form under `decoded` as well.
pub fn print_read_result(
    object: &str,
    property: &str,
    array_index: Option<u32>,
    value: &str,
    decoded: &Datum,
    format: OutputFormat,
) {
    let prop_display = match array_index {
        Some(idx) => format!("{property}[{idx}]"),
        None => property.to_string(),
    };
    match format {
        OutputFormat::Table => {
            let mut table = comfy_table::Table::new();
            table.set_header(vec!["Object", "Property", "Value"]);
            table.add_row(vec![
                object,
                &prop_display,
                &format!("{}", screen(value).green()),
            ]);
            println!("{table}");
        }
        OutputFormat::Json => {
            let result = serde_json::json!({
                "object": object,
                "property": prop_display,
                "value": value,
                "decoded": decoded,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&result).expect("serialize read result")
            );
        }
    }
}

/// One property of a ReadPropertyMultiple result.
#[derive(Debug, Clone)]
pub struct RpmEntry {
    /// The object, `ANALOG_INPUT:1`.
    pub object: String,
    /// The property, `PRESENT_VALUE`.
    pub property: String,
    /// The array index read, if any.
    pub index: Option<u32>,
    /// The value as text, or `ERROR: CLASS:CODE`.
    pub value: String,
    /// The value as typed data; `None` for an error.
    pub data: Option<Datum>,
}

/// Print RPM results: a table, or in JSON one object per property with
/// its text `value` and, for a value, its typed form under `decoded`.
pub fn print_rpm_table(entries: &[RpmEntry], format: OutputFormat) {
    let property = |entry: &RpmEntry| match entry.index {
        Some(i) => format!("{}[{i}]", entry.property),
        None => entry.property.clone(),
    };
    match format {
        OutputFormat::Table => {
            let mut table = comfy_table::Table::new();
            table.set_header(vec!["Object", "Property", "Value"]);
            for entry in entries {
                let value = screen(&entry.value);
                let colored_val = if entry.data.is_none() {
                    format!("{}", value.red())
                } else {
                    format!("{}", value.green())
                };
                table.add_row(vec![entry.object.as_str(), &property(entry), &colored_val]);
            }
            println!("{table}");
        }
        OutputFormat::Json => {
            let json_results: Vec<serde_json::Value> = entries
                .iter()
                .map(|entry| {
                    let mut json = serde_json::json!({
                        "object": entry.object,
                        "property": property(entry),
                        "value": entry.value,
                    });
                    if let Some(data) = &entry.data {
                        json["decoded"] = serde_json::to_value(data).unwrap_or_default();
                    }
                    json
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json_results).expect("serialize rpm results")
            );
        }
    }
}

/// Print a success/status message.
pub fn print_success(msg: &str, format: OutputFormat) {
    match format {
        OutputFormat::Table => println!("{}", msg.green()),
        OutputFormat::Json => {
            let result = serde_json::json!({ "status": "ok", "message": msg });
            println!(
                "{}",
                serde_json::to_string_pretty(&result).expect("serialize success")
            );
        }
    }
}

/// Print a waiting/progress message to stderr.
pub fn print_waiting(secs: u64) {
    eprintln!("{}", format!("Waiting {secs}s for responses...").dimmed());
}

/// Print an error message to stderr.
pub fn print_error(msg: &str) {
    eprintln!("{} {}", "ERROR:".red().bold(), msg.red());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_format_equality() {
        assert_eq!(OutputFormat::Table, OutputFormat::Table);
        assert_ne!(OutputFormat::Table, OutputFormat::Json);
    }

    #[test]
    fn device_info_serializes() {
        let info = DeviceInfo {
            instance: 1234,
            address: "192.168.1.10:47808".to_string(),
            vendor_id: 42,
            max_apdu: 1476,
            segmentation: "both".to_string(),
            network: None,
            station: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("1234"));
        assert!(json.contains("192.168.1.10"));
    }

    /// A routed device found by `whois` prints its station in the target
    /// syntax, in the table and in JSON, and that text parses back to the
    /// same station (#1916).
    #[test]
    fn whois_prints_routed_stations_as_targets() {
        use bacnet_types::MacAddr;
        use bacnet_types::enums::{ObjectType, Segmentation};
        use bacnet_types::primitives::ObjectIdentifier;

        use bacnet_ops::target::{Target, parse_target};

        let found = |instance: u32, routed: Option<(u16, &[u8])>| DiscoveredDevice {
            object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
            mac_address: MacAddr::from_slice(&[10, 0, 1, 1, 0xBA, 0xC0]),
            max_apdu_length: 480,
            segmentation_supported: Segmentation::NONE,
            max_segments_accepted: None,
            vendor_id: 24,
            last_seen: std::time::Instant::now(),
            source_network: routed.map(|(network, _)| network),
            source_address: routed.map(|(_, mac)| MacAddr::from_slice(mac)),
        };
        let stations: [(u16, &[u8], &str); 2] = [
            (2, &[5], "2:0x05"),
            (3, &[10, 0, 2, 9, 0xBA, 0xC0], "3:0x0A000209BAC0"),
        ];
        let mut infos = Vec::new();
        for (instance, (network, mac, text)) in (2001..).zip(stations) {
            let info = device_info(&found(instance, Some((network, mac))), AddressStyle::Bip);
            assert_eq!(info.address, "10.0.1.1:47808", "the router");
            assert_eq!(info.station.as_deref(), Some(text));
            let json = serde_json::to_value(&info).unwrap();
            assert_eq!(json["station"], text);
            assert_eq!(json["network"], network);
            assert_eq!(
                parse_target(text),
                Ok(Target::RoutedMac {
                    dnet: network,
                    mac: mac.to_vec()
                })
            );
            infos.push(info);
        }
        let local = device_info(&found(100, None), AddressStyle::Bip);
        assert_eq!(local.station, None);
        let json = serde_json::to_value(&local).unwrap();
        assert!(json.get("station").is_none());
        infos.push(local);

        let table = devices_table(&infos).to_string();
        assert!(table.contains("Station"), "{table}");
        assert!(!table.contains("Remote MAC"), "{table}");
        let station_cells: Vec<&str> = table
            .lines()
            .filter_map(|line| line.split('|').map(str::trim).nth(4))
            .filter(|cell| cell.contains(":0x"))
            .collect();
        assert_eq!(station_cells, ["2:0x05", "3:0x0A000209BAC0"], "{table}");
    }
}
