//! Command-line argument definitions.
//!
//! Split out of `main.rs` so that adding a flag or a subcommand does not push
//! that file into the 700-LOC cap enforced by `.github/scripts/check-file-size.sh`.
//! Dispatch stays in `main.rs`; this module is the clap surface only.

use std::{net::Ipv4Addr, path::PathBuf};

use clap::{Parser, Subcommand};

#[derive(Parser)]
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

#[derive(Subcommand)]
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
        /// Start position.
        #[arg(long, default_value_t = 0)]
        start: i32,
        /// Byte count.
        #[arg(long, default_value_t = 1024)]
        count: u32,
        /// Output file path.
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
