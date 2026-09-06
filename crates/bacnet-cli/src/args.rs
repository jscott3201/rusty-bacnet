//! Command-line argument definitions.
//!
//! Split out of `main.rs` so that adding a flag or a subcommand does not push
//! that file into the 700-LOC cap enforced by `.github/scripts/check-file-size.sh`.
//! Dispatch stays in `main.rs`; this module is the clap surface only.

use std::{net::Ipv4Addr, path::PathBuf};

use bacnet_types::primitives::BACnetTimeStamp;
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum FileReadAccess {
    Stream,
    Record,
}

#[derive(Debug, Parser)]
#[command(name = "bacnet", about = "BACnet command-line tool", version)]
pub(crate) struct Cli {
    /// Network interface IP address to bind (omit to select interactively in shell mode).
    #[arg(short, long, global = true)]
    pub(crate) interface: Option<Ipv4Addr>,

    /// BACnet UDP port.
    #[arg(short, long, default_value_t = 0xBAC0, global = true)]
    pub(crate) port: u16,

    /// Broadcast address for WhoIs.
    #[arg(short, long, default_value = "255.255.255.255", global = true)]
    pub(crate) broadcast: Ipv4Addr,

    /// APDU timeout in milliseconds.
    #[arg(short, long, default_value_t = 6000, global = true)]
    pub(crate) timeout: u64,

    /// Use BACnet/IPv6 transport.
    #[arg(long, global = true)]
    pub(crate) ipv6: bool,

    /// IPv6 interface address to bind.
    #[arg(long, global = true)]
    pub(crate) ipv6_interface: Option<String>,

    /// Device instance for BIP6 VMAC derivation.
    #[arg(long, global = true)]
    pub(crate) device_instance: Option<u32>,

    /// Use BACnet/SC transport.
    #[arg(long, global = true)]
    pub(crate) sc: bool,

    /// SC hub WebSocket URL.
    #[arg(long, global = true)]
    pub(crate) sc_url: Option<String>,

    /// SC TLS certificate PEM file.
    #[arg(long, global = true)]
    pub(crate) sc_cert: Option<PathBuf>,

    /// SC TLS private key PEM file.
    #[arg(long, global = true)]
    pub(crate) sc_key: Option<PathBuf>,

    /// BACnet/SC local VMAC as 12 hex digits or colon-separated bytes.
    #[arg(long, global = true, value_parser = crate::transport::parse_sc_vmac_arg)]
    pub(crate) sc_vmac: Option<[u8; 6]>,

    /// BACnet/SC device UUID as 32 hex digits or RFC 4122 hyphenated text.
    #[arg(long, global = true, value_parser = crate::transport::parse_sc_device_uuid_arg)]
    pub(crate) sc_device_uuid: Option<[u8; 16]>,

    /// Output format (table, json).
    #[arg(long, global = true)]
    pub(crate) format: Option<String>,

    /// JSON output shorthand.
    #[arg(long, global = true)]
    pub(crate) json: bool,

    /// Verbosity (-v, -vv, -vvv).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub(crate) verbose: u8,

    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Launch interactive shell.
    Shell,

    /// Discover BACnet devices (WhoIs).
    #[command(alias = "whois")]
    Discover {
        /// Device instance range (e.g., "1000-2000").
        range: Option<String>,
        /// Seconds to wait for responses.
        #[arg(long, default_value_t = 3)]
        wait: u64,
        /// Send directed WhoIs to a specific address instead of broadcasting.
        #[arg(long, conflicts_with = "dnet")]
        target: Option<String>,
        /// Register as foreign device with a BBMD before discovering.
        #[arg(long)]
        bbmd: Option<String>,
        /// TTL in seconds for BBMD foreign device registration.
        #[arg(long, default_value_t = 300)]
        ttl: u16,
        /// Target a specific remote network number.
        #[arg(long)]
        dnet: Option<u16>,
    },

    /// Find objects by name (WhoHas).
    #[command(alias = "whohas")]
    Find {
        /// Object name to find.
        #[arg(long)]
        name: Option<String>,
        /// Seconds to wait for responses.
        #[arg(long, default_value_t = 3)]
        wait: u64,
    },

    /// Read a property.
    #[command(alias = "rp")]
    Read {
        /// Target device (IP address or instance number).
        target: String,
        /// Object specifier (e.g., analog-input:1, ai:1).
        object: String,
        /// Property (e.g., present-value, pv).
        property: String,
    },

    /// Read multiple properties.
    #[command(alias = "rpm")]
    Readm {
        /// Target device.
        target: String,
        /// Object and property specs: "ai:1 pv,object-name ao:1 pv".
        specs: Vec<String>,
    },

    /// Write a property.
    #[command(alias = "wp")]
    Write {
        /// Target device.
        target: String,
        /// Object specifier.
        object: String,
        /// Property.
        property: String,
        /// Value to write (e.g., 72.5, true, "string", null).
        value: String,
        /// Priority (1-16).
        #[arg(long)]
        priority: Option<u8>,
    },

    /// Subscribe to COV notifications.
    #[command(alias = "cov")]
    Subscribe {
        /// Target device.
        target: String,
        /// Object specifier.
        object: String,
        /// Subscription lifetime in seconds.
        #[arg(long)]
        lifetime: Option<u32>,
        /// Use confirmed notifications.
        #[arg(long)]
        confirmed: bool,
    },

    /// Device communication control.
    #[command(alias = "dcc")]
    Control {
        /// Target device.
        target: String,
        /// Action: enable, disable, disable-initiation.
        action: String,
        /// Duration in minutes.
        #[arg(long)]
        duration: Option<u16>,
        /// Password.
        #[arg(long)]
        password: Option<String>,
    },

    /// Reinitialize device.
    Reinit {
        /// Target device.
        target: String,
        /// State: coldstart, warmstart.
        state: String,
        /// Password.
        #[arg(long)]
        password: Option<String>,
    },

    /// Get event/alarm information.
    Alarms {
        /// Target device.
        target: String,
    },

    /// Read a file from a device.
    FileRead {
        /// Target device.
        target: String,
        /// File object instance.
        file_instance: u32,
        /// BACnet file access mode.
        #[arg(long, value_enum, default_value = "stream")]
        access: FileReadAccess,
        /// Initial octet position or record index.
        #[arg(long, default_value_t = 0)]
        start: i32,
        /// Per-request octet or record window size.
        #[arg(long, default_value_t = 1024)]
        count: u32,
        /// Stream output file or required record output directory.
        #[arg(long)]
        output: Option<String>,
    },

    /// Write a file to a device.
    FileWrite {
        /// Target device.
        target: String,
        /// File object instance.
        file_instance: u32,
        /// Start position.
        #[arg(long, default_value_t = 0)]
        start: i32,
        /// Input file path.
        input: String,
    },

    /// List cached discovered devices.
    Devices,

    /// Read BBMD broadcast distribution table.
    Bdt {
        /// Target device.
        target: String,
    },

    /// Read BBMD foreign device table.
    Fdt {
        /// Target device.
        target: String,
    },

    /// Register as foreign device with BBMD.
    Register {
        /// Target device.
        target: String,
        /// Time-to-live in seconds.
        #[arg(long, default_value_t = 300)]
        ttl: u16,
    },

    /// Unregister from BBMD.
    Unregister {
        /// Target device.
        target: String,
    },

    /// Send Who-Is-Router-To-Network.
    WhoisRouter,

    /// Acknowledge an alarm.
    #[command(alias = "ack")]
    AckAlarm {
        /// Target device.
        target: String,
        /// Object specifier (e.g., ai:1).
        object: String,
        /// Event state to acknowledge (0=normal, 1=fault, etc.).
        #[arg(long)]
        state: u32,
        /// Acknowledgment source string.
        #[arg(long, default_value = "bacnet-cli")]
        source: String,
        /// Exact timestamp from the original event notification.
        #[arg(long, value_parser = crate::timestamp::parse_bacnet_timestamp)]
        timestamp: BACnetTimeStamp,
        /// Caller-selected time of acknowledgment.
        #[arg(long, value_parser = crate::timestamp::parse_bacnet_timestamp)]
        ack_time: BACnetTimeStamp,
    },

    /// Read a range of items from a list or log buffer.
    #[command(alias = "rr")]
    ReadRange {
        /// Target device.
        target: String,
        /// Object specifier (e.g., trend-log:1).
        object: String,
        /// Property (default: log-buffer).
        #[arg(default_value = "log-buffer")]
        property: String,
    },

    /// Create an object on a remote device.
    CreateObject {
        /// Target device.
        target: String,
        /// Object specifier (type:instance, e.g., av:100).
        object: String,
    },

    /// Delete an object on a remote device.
    DeleteObject {
        /// Target device.
        target: String,
        /// Object specifier (type:instance).
        object: String,
    },

    /// Synchronize time with a device.
    #[command(alias = "ts")]
    TimeSync {
        /// Target device.
        target: String,
        /// Use UTC time synchronization.
        #[arg(long)]
        utc: bool,
    },

    /// Capture and decode BACnet packets.
    Capture {
        /// Read from a pcap file instead of live capture.
        #[arg(long)]
        read: Option<PathBuf>,
        /// Save captured packets to a pcap file.
        #[arg(long)]
        save: Option<PathBuf>,
        /// Suppress decoded output (use with --save).
        #[arg(long)]
        quiet: bool,
        /// Full protocol decode (BVLC/NPDU/APDU/service details).
        #[arg(long)]
        decode: bool,
        /// Network interface name for live capture (e.g., en0, eth0).
        #[arg(long)]
        device: Option<String>,
        /// Additional BPF filter expression (appended to "udp port 47808").
        #[arg(long)]
        filter: Option<String>,
        /// Stop after capturing N packets.
        #[arg(long)]
        count: Option<u64>,
        /// Maximum bytes to capture per packet.
        #[arg(long, default_value_t = 65535)]
        snaplen: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::primitives::{Date, Time};
    use clap::error::ErrorKind;

    #[test]
    fn ack_alarm_and_alias_require_both_timestamp_flags() {
        for command in ["ack-alarm", "ack"] {
            let missing_both =
                Cli::try_parse_from(["bacnet", command, "127.0.0.1", "ai:1", "--state", "1"])
                    .unwrap_err();
            assert_eq!(missing_both.kind(), ErrorKind::MissingRequiredArgument);

            let missing_ack_time = Cli::try_parse_from([
                "bacnet",
                command,
                "127.0.0.1",
                "ai:1",
                "--state",
                "1",
                "--timestamp",
                "sequence:9",
            ])
            .unwrap_err();
            assert_eq!(missing_ack_time.kind(), ErrorKind::MissingRequiredArgument);
        }
    }

    #[test]
    fn ack_alarm_clap_uses_lossless_shared_timestamp_parser() {
        let cli = Cli::try_parse_from([
            "bacnet",
            "ack",
            "127.0.0.1",
            "ai:1",
            "--state",
            "3",
            "--source",
            "operator",
            "--timestamp",
            "time:1,2,3,4",
            "--ack-time",
            "datetime:2026,9,2,3;5,6,7,8",
        ])
        .unwrap();
        let Some(Command::AckAlarm {
            state,
            source,
            timestamp,
            ack_time,
            ..
        }) = cli.command
        else {
            panic!("expected AckAlarm");
        };
        assert_eq!(state, 3);
        assert_eq!(source, "operator");
        assert_eq!(
            timestamp,
            BACnetTimeStamp::Time(Time {
                hour: 1,
                minute: 2,
                second: 3,
                hundredths: 4,
            })
        );
        assert_eq!(
            ack_time,
            BACnetTimeStamp::DateTime {
                date: Date {
                    year: 126,
                    month: 9,
                    day: 2,
                    day_of_week: 3,
                },
                time: Time {
                    hour: 5,
                    minute: 6,
                    second: 7,
                    hundredths: 8,
                },
            }
        );
    }

    #[test]
    fn ack_alarm_clap_reports_shared_parser_error_before_execution() {
        let error = Cli::try_parse_from([
            "bacnet",
            "ack-alarm",
            "127.0.0.1",
            "ai:1",
            "--state",
            "1",
            "--timestamp",
            "time:24,0,0,0",
            "--ack-time",
            "sequence:1",
        ])
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ValueValidation);
        assert!(error
            .to_string()
            .contains("BACnetTimeStamp hour must be 0..=23 or 255 (unspecified), got 24"));
    }
}
