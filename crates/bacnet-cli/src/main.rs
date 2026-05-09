//! BACnet command-line tool.
//!
//! Running `bacnet` with no arguments or with the `shell` subcommand launches
//! an interactive REPL. Subcommands can also be used directly for scripting.

use std::io::IsTerminal;
use std::net::Ipv4Addr;
use std::path::PathBuf;

use bacnet_client::client::BACnetClient;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::port::TransportPort;
use clap::{Parser, Subcommand};

mod commands;
#[allow(dead_code)] // Public API consumed by capture command handler (Task 4).
mod decode;
mod output;
mod parse;
mod resolve;
mod session;
mod shell;
mod transport;

use output::OutputFormat;

#[derive(Parser)]
#[command(name = "bacnet", about = "BACnet command-line tool", version)]
struct Cli {
    /// Network interface IP address to bind (omit to select interactively in shell mode).
    #[arg(short, long, global = true)]
    interface: Option<Ipv4Addr>,

    /// BACnet UDP port.
    #[arg(short, long, default_value_t = 0xBAC0, global = true)]
    port: u16,

    /// Broadcast address for WhoIs.
    #[arg(short, long, default_value = "255.255.255.255", global = true)]
    broadcast: Ipv4Addr,

    /// APDU timeout in milliseconds.
    #[arg(short, long, default_value_t = 6000, global = true)]
    timeout: u64,

    /// Use BACnet/IPv6 transport.
    #[arg(long, global = true)]
    ipv6: bool,

    /// IPv6 interface address to bind.
    #[arg(long, global = true)]
    ipv6_interface: Option<String>,

    /// Device instance for BIP6 VMAC derivation.
    #[arg(long, global = true)]
    device_instance: Option<u32>,

    /// Use BACnet/SC transport.
    #[arg(long, global = true)]
    sc: bool,

    /// SC hub WebSocket URL.
    #[arg(long, global = true)]
    sc_url: Option<String>,

    /// SC TLS certificate PEM file.
    #[arg(long, global = true)]
    sc_cert: Option<PathBuf>,

    /// SC TLS private key PEM file.
    #[arg(long, global = true)]
    sc_key: Option<PathBuf>,

    /// Output format (table, json).
    #[arg(long, global = true)]
    format: Option<String>,

    /// JSON output shorthand.
    #[arg(long, global = true)]
    json: bool,

    /// Verbosity (-v, -vv, -vvv).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
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

fn setup_tracing(verbosity: u8) {
    use tracing_subscriber::EnvFilter;
    let filter = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_target(false)
        .init();
}

fn resolve_format(cli: &Cli) -> OutputFormat {
    if cli.json {
        return OutputFormat::Json;
    }
    match cli.format.as_deref() {
        Some("json") => OutputFormat::Json,
        Some("table") => OutputFormat::Table,
        _ => {
            if std::io::stdout().is_terminal() {
                OutputFormat::Table
            } else {
                OutputFormat::Json
            }
        }
    }
}

/// Resolve a target string to a MAC address, looking up device instances from
/// the client's discovered device table.
async fn resolve_target_mac<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    target_str: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    match resolve::parse_target(target_str)? {
        resolve::Target::Mac(mac) => Ok(mac),
        resolve::Target::Instance(n) => match client.get_device(n).await {
            Some(d) => Ok(d.mac_address.to_vec()),
            None => Err(format!(
                "Device {} not found. Use an IP address or run 'discover' first.",
                n
            )
            .into()),
        },
        resolve::Target::Routed(dnet, instance) => Err(format!(
            "Routed target {}:{} is not supported by this command path. \
             Use a direct MAC/IP target or run 'discover' and use the device instance \
             without DNET.",
            dnet, instance
        )
        .into()),
    }
}

/// Execute a one-shot CLI command.
async fn execute_command<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    cmd: &Command,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        Command::Shell => unreachable!(),
        Command::Discover {
            range,
            wait,
            target,
            bbmd,
            dnet,
            ..
        } => {
            if bbmd.is_some() {
                return Err(
                    "--bbmd requires BACnet/IP transport (do not use --sc or --ipv6)".into(),
                );
            }
            let (low, high) = parse_discover_range(range.as_deref())?;
            if let Some(target_str) = target {
                let mac = resolve::parse_target(target_str)
                    .and_then(|t| match t {
                        resolve::Target::Mac(m) => Ok(m),
                        _ => Err("--target requires an IP address, not a device instance or routed address".into()),
                    })
                    .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
                commands::discover::discover_directed(client, &mac, low, high, *wait, format)
                    .await?;
            } else if let Some(network) = dnet {
                commands::discover::discover_network(client, *network, low, high, *wait, format)
                    .await?;
            } else {
                commands::discover::discover(client, low, high, *wait, format).await?;
            }
        }
        Command::Find { name, wait } => match name {
            Some(n) => {
                commands::discover::find_by_name(client, n, *wait, format).await?;
            }
            None => {
                return Err("--name is required for find command".into());
            }
        },
        Command::Read {
            target,
            object,
            property,
        } => {
            let mac = resolve_target_mac(client, target).await?;
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            let (prop, index) = parse::parse_property(property)?;
            commands::read::read_property_cmd(
                client,
                &mac,
                object_type,
                instance,
                prop,
                index,
                format,
            )
            .await?;
        }
        Command::Readm { target, specs } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::read::read_multiple_cmd(client, &mac, specs, format).await?;
        }
        Command::Write {
            target,
            object,
            property,
            value,
            priority,
        } => {
            let mac = resolve_target_mac(client, target).await?;
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            let (prop, index) = parse::parse_property(property)?;
            let (val, inline_priority) = parse::parse_value_with_priority(value)?;
            let pri = priority.or(inline_priority);
            commands::write::write_property_cmd(
                client,
                &mac,
                object_type,
                instance,
                prop,
                index,
                val,
                pri,
                format,
            )
            .await?;
        }
        Command::Subscribe {
            target,
            object,
            lifetime,
            confirmed,
        } => {
            let mac = resolve_target_mac(client, target).await?;
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            commands::subscribe::subscribe_cmd(
                client,
                &mac,
                object_type,
                instance,
                *lifetime,
                *confirmed,
                format,
            )
            .await?;
        }
        Command::Control {
            target,
            action,
            duration,
            password,
        } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::device::control_cmd(
                client,
                &mac,
                action,
                *duration,
                password.as_deref(),
                format,
            )
            .await?;
        }
        Command::Reinit {
            target,
            state,
            password,
        } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::device::reinit_cmd(client, &mac, state, password.as_deref(), format).await?;
        }
        Command::Alarms { target } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::device::alarms_cmd(client, &mac, format).await?;
        }
        Command::FileRead {
            target,
            file_instance,
            start,
            count,
            output,
        } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::file::file_read_cmd(
                client,
                &mac,
                *file_instance,
                *start,
                *count,
                output.as_deref(),
                format,
            )
            .await?;
        }
        Command::FileWrite {
            target,
            file_instance,
            start,
            input,
        } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::file::file_write_cmd(client, &mac, *file_instance, *start, input, format)
                .await?;
        }
        Command::Devices => {
            commands::router::devices_cmd(client, format).await?;
        }
        Command::Bdt { .. }
        | Command::Fdt { .. }
        | Command::Register { .. }
        | Command::Unregister { .. } => {
            return Err("BBMD management commands (bdt, fdt, register, unregister) are only supported on BACnet/IP transport".into());
        }
        Command::WhoisRouter => {
            commands::router::whois_router_cmd(client, format).await?;
        }
        Command::AckAlarm {
            target,
            object,
            state,
            source,
        } => {
            let mac = resolve_target_mac(client, target).await?;
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            commands::device::acknowledge_alarm_cmd(
                client,
                &mac,
                object_type,
                instance,
                *state,
                source,
                format,
            )
            .await?;
        }
        Command::ReadRange {
            target,
            object,
            property,
        } => {
            let mac = resolve_target_mac(client, target).await?;
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            let (prop, index) = parse::parse_property(property)?;
            commands::read::read_range_cmd(
                client,
                &mac,
                object_type,
                instance,
                prop,
                index,
                format,
            )
            .await?;
        }
        Command::CreateObject { target, object } => {
            let mac = resolve_target_mac(client, target).await?;
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            commands::device::create_object_cmd(client, &mac, object_type, instance, format)
                .await?;
        }
        Command::DeleteObject { target, object } => {
            let mac = resolve_target_mac(client, target).await?;
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            commands::device::delete_object_cmd(client, &mac, object_type, instance, format)
                .await?;
        }
        Command::Capture { .. } => {
            return Err("capture command should be handled before client setup".into());
        }
        Command::TimeSync { target, utc } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::device::time_sync_cmd(client, &mac, *utc, format).await?;
        }
    }
    Ok(())
}

/// Parse a discover range string like "1000-2000" into (low, high).
fn parse_discover_range(
    range: Option<&str>,
) -> Result<(Option<u32>, Option<u32>), Box<dyn std::error::Error>> {
    if let Some(r) = range {
        if let Some((lo, hi)) = r.split_once('-') {
            let low = lo
                .parse::<u32>()
                .map_err(|_| format!("invalid range low: '{lo}'"))?;
            let high = hi
                .parse::<u32>()
                .map_err(|_| format!("invalid range high: '{hi}'"))?;
            if low > high {
                return Err(format!("invalid range: low ({low}) > high ({high})").into());
            }
            Ok((Some(low), Some(high)))
        } else {
            Err(format!("invalid range format: '{r}', expected 'low-high'").into())
        }
    } else {
        Ok((None, None))
    }
}

/// Try to execute a BIP-specific BBMD management command.
/// Returns `Ok(true)` if handled, `Ok(false)` if not a BIP-specific command.
async fn execute_bip_command(
    client: &BACnetClient<BipTransport>,
    cmd: &Command,
    format: OutputFormat,
) -> Result<bool, Box<dyn std::error::Error>> {
    match cmd {
        Command::Discover {
            range,
            wait,
            target,
            bbmd: Some(bbmd_addr),
            ttl,
            dnet,
        } => {
            let bbmd_mac = resolve::parse_target(bbmd_addr)
                .and_then(|t| match t {
                    resolve::Target::Mac(m) => Ok(m),
                    _ => Err("--bbmd requires an IP address, not a device instance".into()),
                })
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            let result = client.register_foreign_device_bvlc(&bbmd_mac, *ttl).await?;
            eprintln!("Registered as foreign device with BBMD: {result:?}");
            // Brief pause to allow registration to propagate.
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let (low, high) = parse_discover_range(range.as_deref())?;
            if let Some(target_str) = target {
                let mac = resolve::parse_target(target_str)
                    .and_then(|t| match t {
                        resolve::Target::Mac(m) => Ok(m),
                        _ => Err("--target requires an IP address, not a device instance or routed address".into()),
                    })
                    .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
                commands::discover::discover_directed(client, &mac, low, high, *wait, format)
                    .await?;
            } else if let Some(network) = dnet {
                commands::discover::discover_network(client, *network, low, high, *wait, format)
                    .await?;
            } else {
                commands::discover::discover(client, low, high, *wait, format).await?;
            }
        }
        Command::Bdt { target } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::router::bdt_cmd(client, &mac, format).await?;
        }
        Command::Fdt { target } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::router::fdt_cmd(client, &mac, format).await?;
        }
        Command::Register { target, ttl } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::router::register_cmd(client, &mac, *ttl, format).await?;
        }
        Command::Unregister { target } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::router::unregister_cmd(client, &mac, format).await?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

async fn run<T: TransportPort + 'static>(
    mut client: BACnetClient<T>,
    cli: &Cli,
    format: OutputFormat,
    is_sc: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match &cli.command {
        None | Some(Command::Shell) => {
            shell::run_shell(client, is_sc, format).await?;
        }
        Some(cmd) => {
            execute_command(&client, cmd, format).await?;
            client.stop().await?;
        }
    }
    Ok(())
}

mod interface;
use interface::pick_interface;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    setup_tracing(cli.verbose);
    let format = resolve_format(&cli);

    let ipv6_interface = cli
        .ipv6_interface
        .as_deref()
        .map(|s| {
            s.parse::<std::net::Ipv6Addr>()
                .map_err(|e| format!("invalid --ipv6-interface address '{s}': {e}"))
        })
        .transpose()?;

    // Determine interface and broadcast address.
    // If --interface was explicitly given, use it (with the given or default broadcast).
    // In interactive shell mode without --interface, prompt the user to pick.
    // In one-shot mode without --interface, default to 0.0.0.0.
    let is_shell = matches!(cli.command, None | Some(Command::Shell));
    let (interface, broadcast) = if let Some(iface) = cli.interface {
        (iface, cli.broadcast)
    } else if is_shell && !cli.sc && !cli.ipv6 && std::io::stdin().is_terminal() {
        pick_interface()?
    } else {
        (Ipv4Addr::UNSPECIFIED, cli.broadcast)
    };

    let args = transport::TransportArgs {
        interface,
        port: cli.port,
        broadcast,
        timeout_ms: cli.timeout,
        sc: cli.sc,
        sc_url: cli.sc_url.clone(),
        sc_cert: cli.sc_cert.clone(),
        sc_key: cli.sc_key.clone(),
        ipv6: cli.ipv6,
        ipv6_interface,
        device_instance: cli.device_instance,
    };

    // Handle capture command separately — no BACnet client needed
    if let Some(Command::Capture {
        ref read,
        ref save,
        quiet,
        decode,
        ref device,
        ref filter,
        count,
        snaplen,
    }) = cli.command
    {
        #[cfg(feature = "pcap")]
        {
            let opts = commands::capture::CaptureOpts {
                read: read.clone(),
                save: save.clone(),
                quiet,
                decode,
                device: device.clone(),
                interface_ip: interface,
                filter: filter.clone(),
                count,
                snaplen,
                format,
            };
            return commands::capture::run_capture(opts);
        }
        #[cfg(not(feature = "pcap"))]
        {
            let _ = (read, save, quiet, decode, device, filter, count, snaplen);
            eprintln!("Error: Packet capture requires the 'pcap' feature. Rebuild with:\n  cargo install bacnet-cli --features pcap");
            std::process::exit(1);
        }
    }

    if args.sc {
        #[cfg(feature = "sc-tls")]
        {
            let client = transport::build_sc_client(&args).await?;
            run(client, &cli, format, true).await?;
        }
        #[cfg(not(feature = "sc-tls"))]
        {
            eprintln!("Error: BACnet/SC requires the 'sc-tls' feature. Rebuild with: cargo install bacnet-cli --features sc-tls");
            std::process::exit(1);
        }
    } else if args.ipv6 {
        let client = transport::build_bip6_client(&args).await?;
        run(client, &cli, format, false).await?;
    } else {
        let mut client = transport::build_bip_client(&args).await?;
        match &cli.command {
            None | Some(Command::Shell) => {
                shell::run_bip_shell(client, format).await?;
            }
            Some(cmd) => {
                if !execute_bip_command(&client, cmd, format).await? {
                    execute_command(&client, cmd, format).await?;
                }
                client.stop().await?;
            }
        }
    }

    Ok(())
}
