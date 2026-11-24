//! Output formatting for the BACnet CLI.
//!
//! Supports table (human-readable) and JSON output formats with automatic
//! TTY detection.

use std::io::{self, Write};

use bacnet_client::discovery::DiscoveredDevice;
use owo_colors::OwoColorize;
use serde::Serialize;

use bacnet_ops::Value;
use bacnet_ops::format::StateTexts;
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

/// Output format for CLI results. Its derived `ValueEnum` lists its
/// variants for [`crate::formats`], which maps each to the `--format` value
/// it prints: they are the formats a command printing through it takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
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
    read: &Value,
    format: OutputFormat,
) {
    let (value, decoded) = (&read.text, &read.data);
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
    /// The value, or why there is none as text: `ERROR: CLASS:CODE`.
    pub value: Result<Value, String>,
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
                let colored_val = match &entry.value {
                    Ok(value) => format!("{}", screen(&value.text).green()),
                    Err(error) => format!("{}", screen(error).red()),
                };
                table.add_row(vec![entry.object.as_str(), &property(entry), &colored_val]);
            }
            println!("{table}");
        }
        OutputFormat::Json => {
            let json_results: Vec<serde_json::Value> = entries
                .iter()
                .map(|entry| {
                    let text = match &entry.value {
                        Ok(value) => &value.text,
                        Err(error) => error,
                    };
                    let mut json = serde_json::json!({
                        "object": entry.object,
                        "property": property(entry),
                        "value": text,
                    });
                    if let Ok(value) = &entry.value {
                        json["decoded"] = serde_json::to_value(&value.data).unwrap_or_default();
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

/// Print a success/status message: the status line of the commands that
/// change a device, and of the file and BBMD commands.
pub fn print_success(msg: &str, format: OutputFormat) {
    println!("{}", success_text(msg, format));
}

/// [`print_success`]'s text: the message, or `{"status": "ok", "message"}`.
pub(crate) fn success_text(msg: &str, format: OutputFormat) -> String {
    match format {
        OutputFormat::Table => msg.green().to_string(),
        OutputFormat::Json => {
            let result = serde_json::json!({ "status": "ok", "message": msg });
            serde_json::to_string_pretty(&result).expect("serialize success")
        }
    }
}

/// What became of a line [`print_line`] wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Printed {
    /// It went out.
    Line,
    /// Stdout's reader has gone (a closed pipe, as `| head -1` leaves it):
    /// the stream stops quietly, a success.
    ReaderGone,
}

/// Print `text` as a line of a long-running command's output (`subscribe`,
/// `capture`). With JSON these lines are JSON Lines (#2783): each one
/// compact value ([`json_line`]) naming its kind in `type`, and nothing
/// else on stdout. A reader that has gone is [`Printed::ReaderGone`], for
/// the command to stop on, cleaning up as it does at Ctrl+C (`println!`
/// would panic); any other failed write (a full disk) is an error.
pub fn print_line(text: &str) -> Result<Printed, Box<dyn std::error::Error>> {
    print_line_to(&mut io::stdout().lock(), text)
}

/// [`print_line`] to `out`: stdout, or a buffer for a test of a stream's
/// loop.
pub(crate) fn print_line_to(
    out: &mut impl Write,
    text: &str,
) -> Result<Printed, Box<dyn std::error::Error>> {
    written(write_line(out, text))
}

/// [`print_line`]'s outcome from its write's. The reader having gone is
/// told by the error's kind, which std gives a closed pipe on each OS: EPIPE
/// on Unix, ERROR_BROKEN_PIPE and ERROR_NO_DATA on Windows.
fn written(result: io::Result<()>) -> Result<Printed, Box<dyn std::error::Error>> {
    match result {
        Ok(()) => Ok(Printed::Line),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(Printed::ReaderGone),
        Err(error) => Err(format!("writing to stdout failed: {error}").into()),
    }
}

/// [`print_line`] to `out`: `text` and its newline in one write, then a
/// flush, so a consumer reading a pipe sees each line when it happens
/// rather than when a buffer fills.
fn write_line(out: &mut impl Write, text: &str) -> io::Result<()> {
    let mut line = String::with_capacity(text.len() + 1);
    line.push_str(text);
    line.push('\n');
    out.write_all(line.as_bytes())?;
    out.flush()
}

/// `value` as a line of a JSON Lines stream: compact.
pub fn json_line(value: &impl Serialize) -> String {
    serde_json::to_string(value).expect("serialize a stream line")
}

/// End a stream that fails once it is running with its last line: in JSON
/// `{"type": "error", "message"}`, the message as stderr gives it after
/// `Error: ` (`main` prints that, with the exit status); with a table,
/// nothing, as stderr says it. A failed write is let go: the message still
/// reaches stderr.
pub fn end_stream(error: &(dyn std::error::Error + 'static), format: OutputFormat) {
    if format == OutputFormat::Json {
        let _ = print_line(&error_line(&crate::error_text(error)));
    }
}

/// [`end_stream`]'s JSON line.
fn error_line(message: &str) -> String {
    json_line(&serde_json::json!({ "type": "error", "message": message }))
}

/// A stream's gap (#2787): `count` items (notifications, packets) that never
/// reached it, as a note on stderr in either format and, in JSON, the line
/// `{"type": "missed", "count"}` for the stream where the gap is; `None`
/// for a table, whose stdout stays as it was.
pub(crate) fn missed(note: &str, count: u64, format: OutputFormat) -> Option<String> {
    print_error(note);
    (format == OutputFormat::Json)
        .then(|| json_line(&serde_json::json!({ "type": "missed", "count": count })))
}

/// Print a waiting/progress message to stderr.
pub fn print_waiting(secs: u64) {
    eprintln!("{}", format!("Waiting {secs}s for responses...").dimmed());
}

/// Log lines for a one-shot command at `-v`'s level (`warn` without it), on
/// stderr in every link mode: stdout carries only the command's output, so a
/// warning, an SC connection close or a lapsed registration can't corrupt a
/// JSON result (#2667). Colour only when stderr is a terminal, as `bacnet
/// mcp`'s.
///
/// Without `-v`, the B/IP start warning for a loopback interface with the
/// limited broadcast address stays quiet: a one-shot unicast command is fine
/// that way, and a broadcast it then tries fails with an error naming
/// `-b/--broadcast` (#2691).
pub fn setup_tracing(verbosity: u8) {
    use std::io::IsTerminal;
    use tracing_subscriber::EnvFilter;
    let filter = match verbosity {
        0 => "warn,bacnet_transport::bip::loopback_guard=error",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_target(false)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
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

    /// The status line each command without a table of its own prints
    /// (write, writem, control, reinit, ack-alarm, delete-object,
    /// time-sync, the file and BBMD commands): the message as a line, or
    /// as JSON (#2778).
    #[test]
    fn a_status_prints_as_a_line_or_as_json() {
        let line = success_text("Alarm acknowledged", OutputFormat::Table);
        assert!(line.contains("Alarm acknowledged"), "{line}");
        assert!(serde_json::from_str::<serde_json::Value>(&line).is_err());
        let json: serde_json::Value =
            serde_json::from_str(&success_text("Alarm acknowledged", OutputFormat::Json)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"status": "ok", "message": "Alarm acknowledged"})
        );
    }

    /// A stream line goes out whole, newline and all, and is flushed after
    /// it (#2783), so a piped reader sees it when it happens.
    #[test]
    fn a_stream_line_is_written_whole_then_flushed() {
        #[derive(Default)]
        struct Recorded {
            writes: Vec<Vec<u8>>,
            /// How many writes had been made at the last flush.
            flushed_after: Option<usize>,
        }
        impl Write for Recorded {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.writes.push(buf.to_vec());
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                self.flushed_after = Some(self.writes.len());
                Ok(())
            }
        }
        let mut out = Recorded::default();
        write_line(&mut out, r#"{"type":"cov"}"#).unwrap();
        assert_eq!(out.writes, [b"{\"type\":\"cov\"}\n".to_vec()]);
        assert_eq!(out.flushed_after, Some(1));
    }

    /// A closed pipe stops a stream quietly; any other failed write is an
    /// error naming stdout (#2783).
    #[test]
    fn a_closed_pipe_stops_quietly_and_other_write_errors_fail() {
        assert_eq!(written(Ok(())).unwrap(), Printed::Line);
        let closed = io::Error::from(io::ErrorKind::BrokenPipe);
        assert_eq!(written(Err(closed)).unwrap(), Printed::ReaderGone);
        for kind in [io::ErrorKind::StorageFull, io::ErrorKind::Other] {
            let error = written(Err(io::Error::new(kind, "no space left on device")))
                .expect_err("an error");
            assert_eq!(
                error.to_string(),
                "writing to stdout failed: no space left on device"
            );
        }
    }

    /// The error line a failed stream ends with is one compact line, even
    /// for a message that spans lines.
    #[test]
    fn a_stream_ends_with_one_error_line() {
        let line = error_line("renewing failed:\nno answer");
        assert!(!line.contains('\n'), "{line}");
        let json: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"type": "error", "message": "renewing failed:\nno answer"})
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
            segmentation_supported: Segmentation::NO_SEGMENTATION,
            max_segments_accepted: None,
            vendor_id: 24,
            last_seen: std::time::Instant::now(),
            last_announced: None,
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
                    mac: mac.to_vec(),
                    one_station: false
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
