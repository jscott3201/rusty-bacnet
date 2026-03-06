//! Output formatting for the BACnet CLI.
//!
//! Supports table (human-readable) and JSON output formats with automatic
//! TTY detection.

use bacnet_types::primitives::PropertyValue;
use serde::Serialize;
use std::io::IsTerminal;

/// Check whether stdout is connected to a terminal.
#[allow(dead_code)]
pub fn is_tty() -> bool {
    std::io::stdout().is_terminal()
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
}

/// Print a list of discovered devices.
pub fn print_devices(devices: &[DeviceInfo], format: OutputFormat) {
    match format {
        OutputFormat::Table => {
            if devices.is_empty() {
                println!("No devices found.");
                return;
            }
            let mut table = comfy_table::Table::new();
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
            println!("{table}");
        }
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(devices).expect("serialize devices");
            println!("{json}");
        }
    }
}

/// Format a `PropertyValue` as a human-readable string.
pub fn format_property_value(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Null => "null".to_string(),
        PropertyValue::Boolean(b) => b.to_string(),
        PropertyValue::Unsigned(v) => v.to_string(),
        PropertyValue::Signed(v) => v.to_string(),
        PropertyValue::Real(v) => format!("{v}"),
        PropertyValue::Double(v) => format!("{v}"),
        PropertyValue::OctetString(bytes) => {
            let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
            format!("[{}]", hex.join(" "))
        }
        PropertyValue::CharacterString(s) => format!("\"{s}\""),
        PropertyValue::BitString { unused_bits, data } => {
            let total_bits = (data.len() * 8).saturating_sub(*unused_bits as usize);
            let mut bits = String::with_capacity(total_bits);
            for i in 0..total_bits {
                let byte_idx = i / 8;
                let bit_idx = 7 - (i % 8);
                if data[byte_idx] & (1 << bit_idx) != 0 {
                    bits.push('1');
                } else {
                    bits.push('0');
                }
            }
            format!("bits({bits})")
        }
        PropertyValue::Enumerated(v) => format!("enumerated({v})"),
        PropertyValue::Date(d) => {
            let year_str = if d.year == 0xFF {
                "*".to_string()
            } else {
                format!("{}", d.year as u16 + 1900)
            };
            let month_str = if d.month == 0xFF {
                "*".to_string()
            } else {
                format!("{:02}", d.month)
            };
            let day_str = if d.day == 0xFF {
                "*".to_string()
            } else {
                format!("{:02}", d.day)
            };
            format!("{year_str}-{month_str}-{day_str}")
        }
        PropertyValue::Time(t) => {
            let hour_str = if t.hour == 0xFF {
                "*".to_string()
            } else {
                format!("{:02}", t.hour)
            };
            let minute_str = if t.minute == 0xFF {
                "*".to_string()
            } else {
                format!("{:02}", t.minute)
            };
            let second_str = if t.second == 0xFF {
                "*".to_string()
            } else {
                format!("{:02}", t.second)
            };
            let hundredths_str = if t.hundredths == 0xFF {
                "*".to_string()
            } else {
                format!("{:02}", t.hundredths)
            };
            format!("{hour_str}:{minute_str}:{second_str}.{hundredths_str}")
        }
        PropertyValue::ObjectIdentifier(oid) => {
            format!("{}:{}", oid.object_type(), oid.instance_number())
        }
        PropertyValue::List(items) => {
            let formatted: Vec<String> = items.iter().map(format_property_value).collect();
            format!("[{}]", formatted.join(", "))
        }
    }
}

/// Print a single read result.
pub fn print_read_result(
    object: &str,
    property: &str,
    array_index: Option<u32>,
    value: &str,
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
            table.add_row(vec![object, &prop_display, value]);
            println!("{table}");
        }
        OutputFormat::Json => {
            let result = serde_json::json!({
                "object": object,
                "property": prop_display,
                "value": value,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&result).expect("serialize read result")
            );
        }
    }
}

/// Print RPM results as a table.
///
/// Each entry is (object_str, property_str, array_index, value_str).
pub fn print_rpm_table(entries: &[(String, String, Option<u32>, String)], format: OutputFormat) {
    match format {
        OutputFormat::Table => {
            let mut table = comfy_table::Table::new();
            table.set_header(vec!["Object", "Property", "Value"]);
            for (obj, prop, idx, val) in entries {
                let prop_display = match idx {
                    Some(i) => format!("{prop}[{i}]"),
                    None => prop.clone(),
                };
                table.add_row(vec![obj.as_str(), &prop_display, val.as_str()]);
            }
            println!("{table}");
        }
        OutputFormat::Json => {
            let json_results: Vec<serde_json::Value> = entries
                .iter()
                .map(|(obj, prop, idx, val)| {
                    let prop_display = match idx {
                        Some(i) => format!("{prop}[{i}]"),
                        None => prop.clone(),
                    };
                    serde_json::json!({
                        "object": obj,
                        "property": prop_display,
                        "value": val,
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json_results).expect("serialize rpm results")
            );
        }
    }
}

/// Print a single generic value.
#[allow(dead_code)]
pub fn print_value(value: &str, format: OutputFormat) {
    match format {
        OutputFormat::Table => {
            println!("{value}");
        }
        OutputFormat::Json => {
            let result = serde_json::json!({ "value": value });
            println!(
                "{}",
                serde_json::to_string_pretty(&result).expect("serialize value")
            );
        }
    }
}

/// Print a success message to stderr.
#[allow(dead_code)]
pub fn print_ok(msg: &str) {
    eprintln!("OK: {msg}");
}

/// Print a success/status message.
pub fn print_success(msg: &str, format: OutputFormat) {
    match format {
        OutputFormat::Table => println!("{msg}"),
        OutputFormat::Json => {
            let result = serde_json::json!({ "status": "ok", "message": msg });
            println!(
                "{}",
                serde_json::to_string_pretty(&result).expect("serialize success")
            );
        }
    }
}

/// Print an error message to stderr.
pub fn print_error(msg: &str) {
    eprintln!("ERROR: {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_null() {
        assert_eq!(format_property_value(&PropertyValue::Null), "null");
    }

    #[test]
    fn format_boolean() {
        assert_eq!(format_property_value(&PropertyValue::Boolean(true)), "true");
    }

    #[test]
    fn format_unsigned() {
        assert_eq!(format_property_value(&PropertyValue::Unsigned(42)), "42");
    }

    #[test]
    fn format_string() {
        assert_eq!(
            format_property_value(&PropertyValue::CharacterString("hello".to_string())),
            "\"hello\""
        );
    }

    #[test]
    fn format_enumerated() {
        assert_eq!(
            format_property_value(&PropertyValue::Enumerated(3)),
            "enumerated(3)"
        );
    }

    #[test]
    fn output_format_equality() {
        assert_eq!(OutputFormat::Table, OutputFormat::Table);
        assert_ne!(OutputFormat::Table, OutputFormat::Json);
    }

    #[test]
    fn format_bitstring_empty_data() {
        // B4: empty data with unused_bits > 0 should not panic.
        let val = PropertyValue::BitString {
            unused_bits: 3,
            data: vec![],
        };
        assert_eq!(format_property_value(&val), "bits()");
    }

    #[test]
    fn format_date_human_readable() {
        use bacnet_types::primitives::Date;
        let d = Date {
            year: 124,
            month: 3,
            day: 15,
            day_of_week: 5,
        };
        assert_eq!(format_property_value(&PropertyValue::Date(d)), "2024-03-15");
    }

    #[test]
    fn format_date_unspecified() {
        use bacnet_types::primitives::Date;
        let d = Date {
            year: 0xFF,
            month: 12,
            day: 0xFF,
            day_of_week: 0xFF,
        };
        assert_eq!(format_property_value(&PropertyValue::Date(d)), "*-12-*");
    }

    #[test]
    fn format_time_human_readable() {
        use bacnet_types::primitives::Time;
        let t = Time {
            hour: 14,
            minute: 30,
            second: 0,
            hundredths: 0,
        };
        assert_eq!(
            format_property_value(&PropertyValue::Time(t)),
            "14:30:00.00"
        );
    }

    #[test]
    fn device_info_serializes() {
        let info = DeviceInfo {
            instance: 1234,
            address: "192.168.1.10:47808".to_string(),
            vendor_id: 42,
            max_apdu: 1476,
            segmentation: "both".to_string(),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("1234"));
        assert!(json.contains("192.168.1.10"));
    }
}
