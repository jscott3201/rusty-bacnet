//! BACnet command-line tool.
//!
//! Running `bacnet` with no arguments or with the `shell` subcommand launches
//! an interactive REPL. Subcommands can also be used directly for scripting.

use std::{io::IsTerminal, net::Ipv4Addr};

use bacnet_client::client::BACnetClient;
use bacnet_transport::{bip::BipTransport, port::TransportPort};
use clap::Parser;

mod args;
mod commands;
#[allow(dead_code)] // Public API consumed by capture command handler (Task 4).
mod decode;
mod output;
mod parse;
mod resolve;
mod session;
mod shell;
mod transport;

use args::{Cli, Command};
use output::OutputFormat;

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
        sc_vmac: cli.sc_vmac,
        sc_device_uuid: cli.sc_device_uuid,
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
