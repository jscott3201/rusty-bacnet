//! BACnet command-line tool.
//!
//! Running `bacnet` with no arguments or with the `shell` subcommand launches
//! an interactive REPL, and `bacnet tui` opens the full-screen terminal UI
//! (with the opt-in `tui` feature).
//! Subcommands can also be used directly for scripting.
#![allow(clippy::print_stdout, clippy::print_stderr)] // a command-line tool prints its results

use std::{io::IsTerminal, net::Ipv4Addr};

use bacnet_client::client::BACnetClient;
use bacnet_transport::{bip::BipTransport, port::TransportPort};
use clap::Parser;

mod args;
mod commands;
mod core;
#[allow(dead_code)] // Public API consumed by capture command handler (Task 4).
mod decode;
mod foreign;
mod output;
mod parse;
mod resolve;
mod session;
mod shell;
mod timestamp;
mod transport;
#[cfg(feature = "tui")]
mod tui;

use crate::core::range::parse_discover_range;
use args::{Cli, Command};
use output::OutputFormat;

fn setup_tracing(verbosity: u8, to_stderr: bool) {
    use tracing_subscriber::EnvFilter;
    let filter = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_target(false);
    if to_stderr {
        // SC close/handshake diagnostics, and a foreign device's refused or
        // lapsed registration, must not corrupt a JSON result.
        subscriber.with_writer(std::io::stderr).init();
    } else {
        subscriber.init();
    }
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
/// the client's discovered device table and B/IPv6 addresses with
/// Virtual-Address-Resolution.
async fn resolve_target_mac<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    target_str: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    match resolve::parse_target_on(client.transport(), target_str)? {
        resolve::Target::Mac(mac) => Ok(mac),
        resolve::Target::Bip6(peer) => Ok(resolve::bip6_vmac(client.transport(), peer).await?),
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
        Command::Shell | Command::Tui { .. } => unreachable!(),
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
            let range = parse_discover_range(range.as_deref())?;
            if let Some(target_str) = target {
                let mac = resolve::address_mac(
                    client.transport(),
                    target_str,
                    resolve::TARGET_NOT_AN_ADDRESS,
                )
                .await?;
                commands::discover::discover_directed(client, &mac, range, *wait, format).await?;
            } else if let Some(network) = dnet {
                commands::discover::discover_network(client, *network, range, *wait, format)
                    .await?;
            } else {
                commands::discover::discover(client, range, *wait, format).await?;
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
                commands::write::WritePropertyArgs {
                    object_type,
                    instance,
                    property: prop,
                    index,
                    value: val,
                    priority: pri,
                },
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
            access,
            start,
            count,
            output,
        } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::file::file_read_cmd(
                client,
                &mac,
                *file_instance,
                commands::file::FileReadOptions {
                    access: *access,
                    start_position: *start,
                    count: *count,
                    output_path: output.as_deref(),
                },
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
            timestamp,
            ack_time,
        } => {
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            let mac = resolve_target_mac(client, target).await?;
            commands::device::acknowledge_alarm_cmd(
                client,
                &mac,
                commands::device::AcknowledgeAlarmArgs {
                    object_type,
                    instance,
                    event_state: bacnet_types::enums::EventState::from_raw(*state),
                    source,
                    timestamp: timestamp.clone(),
                    time_of_acknowledgment: ack_time.clone(),
                },
                format,
            )
            .await?;
        }
        Command::ReadRange {
            target,
            object,
            property,
            range,
        } => {
            let mac = resolve_target_mac(client, target).await?;
            let (object_type, instance) = parse::parse_object_specifier(object)?;
            let (prop, index) = parse::parse_property(property)?;
            commands::read_range::read_range_cmd(
                client,
                &mac,
                object_type,
                instance,
                prop,
                index,
                range,
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

/// Try to execute a BIP-specific BBMD management command.
/// Returns `Ok(true)` if handled, `Ok(false)` if not a BIP-specific command.
async fn execute_bip_command(
    client: &BACnetClient<BipTransport>,
    cmd: &Command,
    ttl: u16,
    format: OutputFormat,
) -> Result<bool, Box<dyn std::error::Error>> {
    match cmd {
        Command::Discover {
            range,
            wait,
            target,
            bbmd: Some(bbmd_addr),
            dnet,
        } => {
            let bbmd_mac = resolve::address_mac(
                client.transport(),
                bbmd_addr,
                "--bbmd requires an IP address, not a device instance",
            )
            .await?;
            let result = client.register_foreign_device_bvlc(&bbmd_mac, ttl).await?;
            eprintln!("Registered as foreign device with BBMD: {result:?}");
            // Brief pause to allow registration to propagate.
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let range = parse_discover_range(range.as_deref())?;
            if let Some(target_str) = target {
                let mac = resolve::address_mac(
                    client.transport(),
                    target_str,
                    resolve::TARGET_NOT_AN_ADDRESS,
                )
                .await?;
                commands::discover::discover_directed(client, &mac, range, *wait, format).await?;
            } else if let Some(network) = dnet {
                commands::discover::discover_network(client, *network, range, *wait, format)
                    .await?;
            } else {
                commands::discover::discover(client, range, *wait, format).await?;
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
        Command::Register { target } => {
            let mac = resolve_target_mac(client, target).await?;
            commands::router::register_cmd(client, &mac, ttl, format).await?;
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

type CliResult = Result<(), Box<dyn std::error::Error>>;

// The CLI's futures are polled on the main thread, whose stack is 1 MiB on
// Windows (8 MiB on Linux and macOS), and a debug build overflowed it on its
// first SC command (#950). So `cli_main`'s state lives on the heap. While it
// runs, the main thread's stack holds the box pointer, block_on's frames, and
// the poll frames of the futures and whatever they call. This builds the
// runtime `#[tokio::main]` would (multi-thread, every driver), without the
// attribute's temporary of the whole future in main's own frame.
//
// Clap's derived parser needed more than all of that: in a debug build,
// `Cli::parse()` took about 860 KiB of the main thread's stack on macOS, most
// of it one 620 KiB frame (`Command::augment_subcommands`, which builds every
// subcommand's arguments inline). So the command line is parsed on a thread of
// its own (#953). A read or a discover then needs about 240 KiB of the main
// thread's stack in a debug build, where it needed 860 KiB before.
fn main() -> CliResult {
    let cli = parse_on_large_stack(Cli::parse);
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed building the Runtime")
        .block_on(boxed_cli_main(cli))
}

/// Stack for the thread that parses the command line; see `main`.
const PARSE_STACK_BYTES: usize = 8 << 20;

/// Run a clap parse on a thread with a stack of [`PARSE_STACK_BYTES`].
///
/// A parse that fails with `Cli::parse` prints its error or the help text and
/// exits the process from that thread, as it would on the main thread.
fn parse_on_large_stack<T: Send + 'static>(parse: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .name("bacnet-args".into())
        .stack_size(PARSE_STACK_BYTES)
        .spawn(parse)
        .expect("spawn the command-line parser thread")
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

/// Create `cli_main`'s future and move it to the heap. The full-size temporary
/// that creating it takes is in this frame, which returns before polling
/// starts.
#[inline(never)]
fn boxed_cli_main(cli: Cli) -> std::pin::Pin<Box<impl std::future::Future<Output = CliResult>>> {
    Box::pin(cli_main(cli))
}

async fn cli_main(cli: Cli) -> CliResult {
    if matches!(cli.command, Some(Command::Tui { .. })) {
        // The TUI sets up its own tracing: nothing may write to the terminal
        // while it is in raw mode. Boxed so its state stays out of this future.
        return Box::pin(run_tui(cli)).await;
    }
    setup_tracing(cli.verbose, cli.sc || cli.foreign_bbmd.is_some());
    let format = resolve_format(&cli);

    let mut args = transport::TransportArgs::from_cli(&cli, Ipv4Addr::UNSPECIFIED, cli.broadcast)?;

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
    args.interface = interface;
    args.broadcast = broadcast;

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
                shell::run_bip_shell(client, cli.foreign_bbmd, format).await?;
            }
            Some(cmd) => {
                // Broadcasts reach the remote network once the BBMD has
                // taken the registration (#938).
                if let Some(bbmd) = cli.foreign_bbmd {
                    if let Err(e) = foreign::await_registration(&client, bbmd).await {
                        // The registration's failure is the error to report.
                        return Err(match client.stop().await {
                            Ok(()) => e,
                            Err(stop) => format!("{e}; stopping the client also failed: {stop}"),
                        }
                        .into());
                    }
                }
                if !execute_bip_command(&client, cmd, cli.ttl, format).await? {
                    execute_command(&client, cmd, format).await?;
                }
                client.stop().await?;
            }
        }
    }

    Ok(())
}

/// `bacnet tui`, or the rebuild advice when the `tui` feature is off.
async fn run_tui(cli: Cli) -> CliResult {
    #[cfg(feature = "tui")]
    {
        tui::run(cli).await
    }
    #[cfg(not(feature = "tui"))]
    {
        let _ = cli;
        eprintln!("Error: The terminal UI requires the 'tui' feature. Rebuild with:\n  cargo install bacnet-cli --features tui");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv6Addr;

    use bacnet_transport::bip6::Bip6Transport;

    use super::*;

    /// A one-shot command's `[ip]:port` target is the VMAC of the node
    /// there, found with a Virtual-Address-Resolution before the command
    /// runs (Clause U.5); `bip6:1234` names the same VMAC directly.
    #[tokio::test]
    async fn a_bip6_address_target_resolves_to_the_vmac_there() {
        let mut server = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(1234));
        let _server_rx = server.start().await.unwrap();
        let port = server.local_address().unwrap().port();
        let mut client = BACnetClient::bip6_builder()
            .interface(Ipv6Addr::LOCALHOST)
            .port(0)
            // An instance VMAC: a random one needs a multicast probe at start,
            // which Linux loopback can't carry.
            .device_instance(4321)
            .build()
            .await
            .unwrap();
        let target = format!("[::1]:{port}");
        let mac = resolve_target_mac(&client, &target).await.unwrap();
        assert_eq!(mac, [0x00, 0x04, 0xD2]);
        let mac = resolve_target_mac(&client, "bip6:1234").await.unwrap();
        assert_eq!(mac, [0x00, 0x04, 0xD2]);
        client.stop().await.unwrap();
        server.stop().await.unwrap();
    }
}
